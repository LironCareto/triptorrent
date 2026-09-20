//! Native reference client and display-independent desktop controller.

use anyhow::{Context, Result, bail};
use eframe::egui;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use triptorrent_node_api::{
    ContentRecord, Diagnostics, LocalApiClient, NetworkPrivacyStatus, NodeStatus, NodeSummary,
    TransferRecord,
};

/// Desktop-to-daemon startup and polling options.
#[derive(Clone, Debug)]
pub struct DesktopOptions {
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
    pub node_binary: PathBuf,
    pub api_listen: std::net::SocketAddr,
    pub bootstrap: Option<std::net::SocketAddr>,
    pub poll_interval: Duration,
    pub startup_timeout: Duration,
    pub reconnect_backoff: Duration,
}

impl DesktopOptions {
    /// Resolves OS-appropriate defaults and a sidecar next to this executable.
    ///
    /// # Errors
    ///
    /// Returns an error when the process has no usable user-data directory.
    pub fn system_default() -> Result<Self> {
        let root = default_user_root()?;
        let executable = std::env::current_exe().context("cannot locate desktop executable")?;
        let sidecar =
            executable.with_file_name(format!("triptorrent{}", std::env::consts::EXE_SUFFIX));
        Ok(Self {
            config_path: root.join("triptorrent.toml"),
            data_dir: root.join("data"),
            node_binary: sidecar,
            api_listen: std::net::SocketAddr::from(([127, 0, 0, 1], 7331)),
            bootstrap: None,
            poll_interval: Duration::from_millis(750),
            startup_timeout: Duration::from_secs(8),
            reconnect_backoff: Duration::from_secs(2),
        })
    }
}

fn default_user_root() -> Result<PathBuf> {
    if let Some(override_root) = std::env::var_os("TRIPTORRENT_HOME") {
        return Ok(PathBuf::from(override_root));
    }
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("TripTorrent"));
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join("Library/Application Support/TripTorrent"));
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".config"))
        })
        .map(|path| path.join("triptorrent"));
    root.context("cannot determine a per-user TripTorrent directory")
}

/// Connection lifecycle shown persistently by the GUI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    Starting,
    Connected,
    Disconnected,
    Unhealthy,
    Stopped,
}

impl ConnectionState {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Starting => "Starting",
            Self::Connected => "Connected",
            Self::Disconnected => "Disconnected",
            Self::Unhealthy => "Unhealthy",
            Self::Stopped => "Stopped",
        }
    }
}

/// Immutable state snapshot consumed by the immediate-mode view.
#[derive(Clone, Debug)]
pub struct DesktopSnapshot {
    pub connection: ConnectionState,
    pub status: Option<NodeStatus>,
    pub node: Option<NodeSummary>,
    pub content: Vec<ContentRecord>,
    pub transfers: Vec<TransferRecord>,
    pub diagnostics: Option<Diagnostics>,
    pub network_privacy: NetworkPrivacyStatus,
    pub last_error: Option<String>,
    pub notice: Option<String>,
    pub daemon_started_by_desktop: bool,
    pub daemon_launch_count: usize,
    pub config_path: PathBuf,
}

impl DesktopSnapshot {
    fn initial(config_path: PathBuf) -> Self {
        Self {
            connection: ConnectionState::Starting,
            status: None,
            node: None,
            content: Vec::new(),
            transfers: Vec::new(),
            diagnostics: None,
            network_privacy: NetworkPrivacyStatus::current_prototype(),
            last_error: None,
            notice: None,
            daemon_started_by_desktop: false,
            daemon_launch_count: 0,
            config_path,
        }
    }

    /// Generates copyable diagnostics without bearer tokens or session keys.
    #[must_use]
    pub fn diagnostic_text(&self) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "TripTorrent desktop {}", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(text, "Connection: {}", self.connection.label());
        let _ = writeln!(text, "Config: {}", self.config_path.display());
        if let Some(node) = &self.node {
            let _ = writeln!(text, "API: {}", node.api_listen);
            let _ = writeln!(text, "Data directory: {}", node.data_dir.display());
            let _ = writeln!(text, "Bootstrap: {:?}", node.bootstrap);
        }
        if let Some(status) = &self.status {
            let _ = writeln!(text, "Uptime ms: {}", status.uptime_ms);
            let _ = writeln!(
                text,
                "Stored/shared: {}/{}",
                status.stored_content, status.shared_content
            );
            let _ = writeln!(
                text,
                "Active downloads/uploads: {}/{}",
                status.active_downloads, status.active_uploads
            );
            let _ = writeln!(
                text,
                "Downloaded/uploaded bytes: {}/{}",
                status.bytes_downloaded, status.bytes_uploaded
            );
            let _ = writeln!(
                text,
                "Completed/failed: {}/{}",
                status.completed_transfers, status.failed_transfers
            );
        }
        if let Some(diagnostics) = &self.diagnostics {
            let _ = writeln!(
                text,
                "API/node version: {}/{}",
                diagnostics.api_version, diagnostics.node_version
            );
            let _ = writeln!(text, "Provider workers: {}", diagnostics.provider_workers);
            let _ = writeln!(text, "Known relays: {:?}", diagnostics.known_relays);
        }
        let _ = writeln!(text, "Network mode: {}", self.network_privacy.network_mode);
        let _ = writeln!(text, "Discovery: {}", self.network_privacy.discovery);
        let _ = writeln!(text, "Data path: {}", self.network_privacy.data_path);
        let _ = writeln!(
            text,
            "Anonymity guarantee: {}",
            self.network_privacy.anonymity_guarantee
        );
        text.push_str(
            "Content IDs, paths, endpoints, routes, and timestamps may be privacy-sensitive.\n",
        );
        text
    }
}

enum ControllerCommand {
    AddContent(PathBuf, bool),
    RemoveContent(String, bool),
    StartFetch(String, bool),
    Pause(i64),
    Resume(i64),
    StopNode,
    StartNode,
    Refresh,
}

/// Background controller shared by automated tests and the graphical view.
pub struct DesktopController {
    state: Arc<Mutex<DesktopSnapshot>>,
    sender: mpsc::Sender<ControllerCommand>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl DesktopController {
    /// Starts the non-blocking controller and daemon lifecycle worker.
    #[must_use]
    pub fn start(options: DesktopOptions) -> Self {
        let state = Arc::new(Mutex::new(DesktopSnapshot::initial(
            options.config_path.clone(),
        )));
        let (sender, receiver) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_shutdown = Arc::clone(&shutdown);
        let worker = thread::spawn(move || {
            run_controller(&options, &worker_state, &receiver, &worker_shutdown);
        });
        Self {
            state,
            sender,
            shutdown,
            worker: Some(worker),
        }
    }

    /// Returns the latest state without performing I/O.
    #[must_use]
    pub fn snapshot(&self) -> DesktopSnapshot {
        self.state.lock().map_or_else(
            |poisoned| poisoned.into_inner().clone(),
            |state| state.clone(),
        )
    }

    /// Queues a local file import.
    ///
    /// # Errors
    ///
    /// Returns an error if the background controller has stopped.
    pub fn add_content(&self, path: PathBuf, shared: bool) -> Result<()> {
        self.send(ControllerCommand::AddContent(path, shared))
    }

    /// Queues removal of a library record and optionally its managed copy.
    ///
    /// # Errors
    ///
    /// Returns an error if the background controller has stopped.
    pub fn remove_content(&self, content_id: String, delete_bytes: bool) -> Result<()> {
        self.send(ControllerCommand::RemoveContent(content_id, delete_bytes))
    }

    /// Queues a network fetch.
    ///
    /// # Errors
    ///
    /// Returns an error if the background controller has stopped.
    pub fn start_fetch(&self, content_id: String, shared: bool) -> Result<()> {
        self.send(ControllerCommand::StartFetch(content_id, shared))
    }

    /// Requests a cooperative pause for a download.
    ///
    /// # Errors
    ///
    /// Returns an error if the background controller has stopped.
    pub fn pause(&self, transfer_id: i64) -> Result<()> {
        self.send(ControllerCommand::Pause(transfer_id))
    }

    /// Resumes a paused download from its verified partial state.
    ///
    /// # Errors
    ///
    /// Returns an error if the background controller has stopped.
    pub fn resume(&self, transfer_id: i64) -> Result<()> {
        self.send(ControllerCommand::Resume(transfer_id))
    }

    /// Requests an orderly daemon shutdown and disables automatic restart.
    ///
    /// # Errors
    ///
    /// Returns an error if the background controller has stopped.
    pub fn stop_node(&self) -> Result<()> {
        self.send(ControllerCommand::StopNode)
    }

    /// Enables the daemon lifecycle and requests a connection immediately.
    ///
    /// # Errors
    ///
    /// Returns an error if the background controller has stopped.
    pub fn start_node(&self) -> Result<()> {
        self.send(ControllerCommand::StartNode)
    }

    /// Requests an immediate state refresh.
    ///
    /// # Errors
    ///
    /// Returns an error if the background controller has stopped.
    pub fn refresh(&self) -> Result<()> {
        self.send(ControllerCommand::Refresh)
    }

    fn send(&self, command: ControllerCommand) -> Result<()> {
        self.sender
            .send(command)
            .context("desktop controller is not running")
    }
}

impl Drop for DesktopController {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        drop(self.worker.take());
    }
}

fn run_controller(
    options: &DesktopOptions,
    state: &Arc<Mutex<DesktopSnapshot>>,
    receiver: &mpsc::Receiver<ControllerCommand>,
    shutdown: &AtomicBool,
) {
    let mut client = None;
    let mut child: Option<Child> = None;
    let mut desired_running = true;
    let mut next_refresh = Instant::now();
    let mut next_launch = Instant::now();
    while !shutdown.load(Ordering::Relaxed) {
        while let Ok(command) = receiver.try_recv() {
            handle_command(
                command,
                &mut client,
                &mut desired_running,
                &mut child,
                state,
                &mut next_refresh,
            );
        }

        if let Some(process) = &mut child
            && let Ok(Some(status)) = process.try_wait()
        {
            child = None;
            set_error(
                state,
                ConnectionState::Disconnected,
                format!("node process exited with {status}"),
            );
        }

        if desired_running && Instant::now() >= next_refresh {
            if client.is_none() {
                match ensure_config(options) {
                    Ok(()) => match LocalApiClient::from_config(&options.config_path) {
                        Ok(loaded) => client = Some(loaded),
                        Err(error) => {
                            set_error(state, ConnectionState::Unhealthy, error.to_string());
                        }
                    },
                    Err(error) => set_error(state, ConnectionState::Unhealthy, error.to_string()),
                }
            }
            if let Some(api) = &client {
                match api.health() {
                    Ok(health) if health.alive && health.api_ready => {
                        if let Err(error) = refresh_snapshot(api, state) {
                            set_error(state, ConnectionState::Disconnected, error.to_string());
                        }
                    }
                    Ok(_) => set_error(
                        state,
                        ConnectionState::Unhealthy,
                        "node health check is not ready".into(),
                    ),
                    Err(error) => {
                        set_error(state, ConnectionState::Disconnected, error.to_string());
                        if child.is_none() && Instant::now() >= next_launch {
                            set_connection(state, ConnectionState::Starting);
                            match start_daemon(options, api) {
                                Ok(started) => {
                                    child = Some(started);
                                    if let Ok(mut snapshot) = state.lock() {
                                        snapshot.daemon_started_by_desktop = true;
                                        snapshot.daemon_launch_count += 1;
                                        snapshot.last_error = None;
                                    }
                                    let _ = refresh_snapshot(api, state);
                                }
                                Err(start_error) => set_error(
                                    state,
                                    ConnectionState::Unhealthy,
                                    format!("could not start local node: {start_error}"),
                                ),
                            }
                            next_launch = Instant::now() + options.reconnect_backoff;
                        }
                    }
                }
            }
            next_refresh = Instant::now() + options.poll_interval;
        }
        thread::sleep(Duration::from_millis(25));
    }
    drop(child);
}

fn handle_command(
    command: ControllerCommand,
    client: &mut Option<LocalApiClient>,
    desired_running: &mut bool,
    child: &mut Option<Child>,
    state: &Arc<Mutex<DesktopSnapshot>>,
    next_refresh: &mut Instant,
) {
    match command {
        ControllerCommand::StartNode => {
            *desired_running = true;
            *next_refresh = Instant::now();
            set_connection(state, ConnectionState::Starting);
        }
        ControllerCommand::StopNode => {
            *desired_running = false;
            let result = client
                .as_ref()
                .context("node API is unavailable")
                .and_then(LocalApiClient::shutdown);
            match result {
                Ok(_) => {
                    set_notice(
                        state,
                        "Node stop requested; it will remain stopped until started again.",
                    );
                    set_connection(state, ConnectionState::Stopped);
                }
                Err(error) => set_error(state, ConnectionState::Disconnected, error.to_string()),
            }
            drop(child.take());
        }
        ControllerCommand::Refresh => *next_refresh = Instant::now(),
        operation => {
            let result = client
                .as_ref()
                .context("node API is unavailable")
                .and_then(|api| match operation {
                    ControllerCommand::AddContent(path, shared) => api
                        .add_content(&path, shared)
                        .map(|_| "Content imported".to_owned()),
                    ControllerCommand::RemoveContent(id, delete) => {
                        api.remove_content(&id, delete).map(|_| {
                            if delete {
                                "Content metadata and managed copy removed"
                            } else {
                                "Content removed; managed copy preserved"
                            }
                            .to_owned()
                        })
                    }
                    ControllerCommand::StartFetch(id, shared) => api
                        .start_fetch(&id, shared)
                        .map(|transfer| format!("Transfer {transfer} started")),
                    ControllerCommand::Pause(id) => api
                        .pause_transfer(id)
                        .map(|_| format!("Pausing transfer {id}")),
                    ControllerCommand::Resume(id) => api
                        .resume_transfer(id)
                        .map(|_| format!("Transfer {id} resumed")),
                    ControllerCommand::StopNode
                    | ControllerCommand::StartNode
                    | ControllerCommand::Refresh => unreachable!(),
                });
            match result {
                Ok(message) => {
                    set_notice(state, &message);
                    *next_refresh = Instant::now();
                }
                Err(error) => set_operation_error(state, error.to_string()),
            }
        }
    }
}

fn ensure_config(options: &DesktopOptions) -> Result<()> {
    if options.config_path.exists() {
        return Ok(());
    }
    if let Some(parent) = options.config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut command = Command::new(&options.node_binary);
    command
        .arg("node")
        .arg("init")
        .arg("--config")
        .arg(&options.config_path)
        .arg("--data-dir")
        .arg(&options.data_dir)
        .arg("--api-listen")
        .arg(options.api_listen.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(bootstrap) = options.bootstrap {
        command.arg("--bootstrap").arg(bootstrap.to_string());
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("cannot run node sidecar {}", options.node_binary.display()))?;
    wait_for_exit(
        &mut child,
        options.startup_timeout,
        "node configuration initialization",
    )?;
    if !options.config_path.exists() {
        bail!("node initializer exited without creating configuration");
    }
    Ok(())
}

fn wait_for_exit(child: &mut Child, timeout: Duration, operation: &str) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            bail!("{operation} failed with {status}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{operation} timed out");
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn start_daemon(options: &DesktopOptions, client: &LocalApiClient) -> Result<Child> {
    if client.health().is_ok() {
        bail!("a healthy node is already running");
    }
    let mut child = Command::new(&options.node_binary)
        .arg("node")
        .arg("start")
        .arg("--config")
        .arg(&options.config_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| {
            format!(
                "cannot launch node sidecar {}",
                options.node_binary.display()
            )
        })?;
    let deadline = Instant::now() + options.startup_timeout;
    loop {
        if client.health().is_ok() {
            return Ok(child);
        }
        if let Some(status) = child.try_wait()? {
            bail!("node exited during startup with {status}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("node API was not ready before the startup timeout");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn refresh_snapshot(client: &LocalApiClient, state: &Arc<Mutex<DesktopSnapshot>>) -> Result<()> {
    let status = client.status()?;
    let node = client.node()?;
    let content = client.content()?;
    let transfers = client.transfers()?;
    let diagnostics = client.diagnostics()?;
    let network_privacy = client.network_privacy()?;
    if let Ok(mut snapshot) = state.lock() {
        snapshot.connection = ConnectionState::Connected;
        snapshot.status = Some(status);
        snapshot.node = Some(node);
        snapshot.content = content;
        snapshot.transfers = transfers;
        snapshot.diagnostics = Some(diagnostics);
        snapshot.network_privacy = network_privacy;
        snapshot.last_error = None;
    }
    Ok(())
}

fn set_connection(state: &Arc<Mutex<DesktopSnapshot>>, connection: ConnectionState) {
    if let Ok(mut snapshot) = state.lock() {
        snapshot.connection = connection;
    }
}

fn set_error(state: &Arc<Mutex<DesktopSnapshot>>, connection: ConnectionState, error: String) {
    if let Ok(mut snapshot) = state.lock() {
        snapshot.connection = connection;
        snapshot.last_error = Some(error);
    }
}

fn set_operation_error(state: &Arc<Mutex<DesktopSnapshot>>, error: String) {
    if let Ok(mut snapshot) = state.lock() {
        snapshot.last_error = Some(error);
    }
}

fn set_notice(state: &Arc<Mutex<DesktopSnapshot>>, message: &str) {
    if let Ok(mut snapshot) = state.lock() {
        snapshot.notice = Some(message.to_owned());
        snapshot.last_error = None;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Page {
    #[default]
    Transfers,
    Library,
    Network,
    Diagnostics,
}

/// Native egui application. All I/O is delegated to [`DesktopController`].
pub struct DesktopApp {
    controller: DesktopController,
    page: Page,
    selected_transfer: Option<i64>,
    selected_content: Option<String>,
    fetch_id: String,
    import_path: String,
    share_import: bool,
    share_fetch: bool,
    confirm_delete: Option<String>,
}

impl DesktopApp {
    #[must_use]
    pub fn new(controller: DesktopController) -> Self {
        Self {
            controller,
            page: Page::Transfers,
            selected_transfer: None,
            selected_content: None,
            fetch_id: String::new(),
            import_path: String::new(),
            share_import: true,
            share_fetch: true,
            confirm_delete: None,
        }
    }

    fn dispatch_dropped_files(&self, ctx: &egui::Context) {
        let paths: Vec<_> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect()
        });
        for path in paths {
            let _ = self.controller.add_content(path, self.share_import);
        }
    }

    fn render(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.controller.snapshot();
        Self::top_status(ui, &snapshot);
        ui.separator();
        ui.horizontal(|ui| {
            ui.vertical(|ui| self.navigation(ui, &snapshot));
            ui.separator();
            ui.vertical(|ui| match self.page {
                Page::Transfers => self.transfers(ui, &snapshot),
                Page::Library => self.library(ui, &snapshot),
                Page::Network => Self::network(ui, &snapshot),
                Page::Diagnostics => self.diagnostics(ui, &snapshot),
            });
        });
        self.delete_confirmation(ui.ctx());
    }

    fn top_status(ui: &mut egui::Ui, snapshot: &DesktopSnapshot) {
        ui.horizontal(|ui| {
            ui.heading("TripTorrent");
            ui.separator();
            ui.label(format!("Node: {}", snapshot.connection.label()));
            if snapshot.daemon_started_by_desktop {
                ui.weak("Started by this desktop session; remains running when the window closes");
            }
        });
        if let Some(error) = &snapshot.last_error {
            ui.colored_label(egui::Color32::from_rgb(190, 70, 60), error);
        } else if let Some(notice) = &snapshot.notice {
            ui.label(notice);
        }
    }

    fn navigation(&mut self, ui: &mut egui::Ui, snapshot: &DesktopSnapshot) {
        ui.set_min_width(160.0);
        for (page, label) in [
            (Page::Transfers, "Transfers"),
            (Page::Library, "Library"),
            (Page::Network, "Network & Privacy"),
            (Page::Diagnostics, "Diagnostics / Settings"),
        ] {
            ui.selectable_value(&mut self.page, page, label);
        }
        ui.add_space(18.0);
        if snapshot.connection == ConnectionState::Connected {
            if ui.button("Stop node").clicked() {
                let _ = self.controller.stop_node();
            }
        } else if ui.button("Start / reconnect node").clicked() {
            let _ = self.controller.start_node();
        }
    }

    fn transfers(&mut self, ui: &mut egui::Ui, snapshot: &DesktopSnapshot) {
        ui.heading("Transfers");
        self.fetch_controls(ui, snapshot);
        ui.add_space(8.0);
        self.transfer_table(ui, snapshot);
        if let Some(transfer) = self
            .selected_transfer
            .and_then(|id| snapshot.transfers.iter().find(|item| item.id == id))
        {
            self.transfer_detail(ui, transfer);
        }
    }

    fn fetch_controls(&mut self, ui: &mut egui::Ui, snapshot: &DesktopSnapshot) {
        let network_available = snapshot
            .node
            .as_ref()
            .is_some_and(|node| node.bootstrap.is_some());
        ui.group(|ui| {
            ui.label("Fetch by TripTorrent content ID");
            ui.horizontal(|ui| {
                ui.add_enabled(
                    network_available,
                    egui::TextEdit::singleline(&mut self.fetch_id).desired_width(420.0),
                );
                if ui
                    .add_enabled(
                        network_available && !self.fetch_id.trim().is_empty(),
                        egui::Button::new("Fetch"),
                    )
                    .clicked()
                {
                    let _ = self
                        .controller
                        .start_fetch(self.fetch_id.trim().to_owned(), self.share_fetch);
                    self.fetch_id.clear();
                }
            });
            ui.checkbox(&mut self.share_fetch, "Share after completion");
            if !network_available {
                ui.weak("Network fetch is unavailable because no bootstrap is configured.");
            }
        });
    }

    fn transfer_table(&mut self, ui: &mut egui::Ui, snapshot: &DesktopSnapshot) {
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                egui::Grid::new("transfer_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("ID");
                        ui.strong("Content");
                        ui.strong("State");
                        ui.strong("Verified");
                        ui.strong("Rate");
                        ui.end_row();
                        for transfer in &snapshot.transfers {
                            if ui
                                .selectable_label(
                                    self.selected_transfer == Some(transfer.id),
                                    transfer.id.to_string(),
                                )
                                .clicked()
                            {
                                self.selected_transfer = Some(transfer.id);
                            }
                            ui.monospace(short_id(&transfer.content_id));
                            ui.label(&transfer.status);
                            let fraction = progress_fraction(transfer);
                            ui.add(egui::ProgressBar::new(fraction).text(format!(
                                "{} / {}",
                                format_bytes(transfer.bytes_transferred),
                                format_bytes(transfer.bytes_total)
                            )));
                            ui.label(format!("{}/s", format_bytes(transfer.current_rate_bps)));
                            ui.end_row();
                        }
                    });
            });
    }

    fn transfer_detail(&mut self, ui: &mut egui::Ui, transfer: &TransferRecord) {
        ui.separator();
        ui.heading(format!("Transfer {}", transfer.id));
        ui.monospace(format!("TripTorrent ID: {}", transfer.content_id));
        ui.label(format!(
            "Verified: {} chunks, {}",
            transfer.verified_chunks,
            format_bytes(transfer.bytes_transferred)
        ));
        ui.label(format!(
            "Providers: {} · retries: {} · rejected: {}",
            transfer.provider_count, transfer.retry_count, transfer.rejected_chunks
        ));
        ui.label(format!("Network path: {}", transfer.network_path));
        ui.label(format!(
            "Resume state: {} chunks reused",
            transfer.resumed_chunks
        ));
        if let Some(error) = &transfer.error {
            ui.colored_label(egui::Color32::from_rgb(190, 70, 60), error);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(transfer.status == "running", egui::Button::new("Pause"))
                .clicked()
            {
                let _ = self.controller.pause(transfer.id);
            }
            if ui
                .add_enabled(transfer.status == "paused", egui::Button::new("Resume"))
                .clicked()
            {
                let _ = self.controller.resume(transfer.id);
            }
        });
        if !transfer.provider_contributions.is_empty() {
            ui.label("Verified provider contribution:");
            for provider in &transfer.provider_contributions {
                ui.monospace(format!(
                    "{}: {} chunks ({} rejected)",
                    short_id(&provider.provider_id),
                    provider.verified_chunks,
                    provider.rejected_requests
                ));
            }
        }
    }

    fn library(&mut self, ui: &mut egui::Ui, snapshot: &DesktopSnapshot) {
        ui.heading("Library");
        ui.group(|ui| {
            ui.label(
                "Add a local file by dropping it anywhere in the window or entering its path:",
            );
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.import_path);
                if ui
                    .add_enabled(
                        !self.import_path.trim().is_empty(),
                        egui::Button::new("Add file"),
                    )
                    .clicked()
                {
                    let _ = self
                        .controller
                        .add_content(PathBuf::from(self.import_path.trim()), self.share_import);
                    self.import_path.clear();
                }
            });
            ui.checkbox(
                &mut self.share_import,
                "Share when network infrastructure is configured",
            );
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .max_height(330.0)
            .show(ui, |ui| {
                egui::Grid::new("library_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("TripTorrent identity");
                        ui.strong("Size");
                        ui.strong("Integrity");
                        ui.strong("Sharing");
                        ui.end_row();
                        for content in &snapshot.content {
                            if ui
                                .selectable_label(
                                    self.selected_content.as_deref() == Some(&content.content_id),
                                    short_id(&content.content_id),
                                )
                                .clicked()
                            {
                                self.selected_content = Some(content.content_id.clone());
                            }
                            ui.label(format_bytes(content.length));
                            ui.label(&content.integrity);
                            ui.label(if content.advertised {
                                "Advertised"
                            } else if content.shared {
                                "Enabled; offline"
                            } else {
                                "Not shared"
                            });
                            ui.end_row();
                        }
                    });
            });
        if let Some(content_id) = self.selected_content.clone() {
            ui.separator();
            ui.monospace(format!("TripTorrent ID: {content_id}"));
            ui.label("No verified BitTorrent aliases are present in the current runtime.");
            ui.horizontal(|ui| {
                if ui.button("Remove from TripTorrent").clicked() {
                    let _ = self.controller.remove_content(content_id.clone(), false);
                    self.selected_content = None;
                }
                if ui.button("Delete managed copy…").clicked() {
                    self.confirm_delete = Some(content_id);
                }
            });
            ui.weak("Removing metadata preserves managed bytes. Deleting the managed copy never deletes the original imported file.");
        }
    }

    fn network(ui: &mut egui::Ui, snapshot: &DesktopSnapshot) {
        let status = &snapshot.network_privacy;
        ui.heading("Network & Privacy");
        ui.label("Current implementation facts");
        egui::Grid::new("network_status")
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Network mode");
                ui.label("TripTorrent prototype");
                ui.end_row();
                ui.strong("Discovery");
                ui.label("M2 bootstrap — temporary and centralized");
                ui.end_row();
                ui.strong("Data path");
                ui.label("End-to-end encrypted relayed swarm");
                ui.end_row();
                ui.strong("Direct peer connection");
                ui.label(if status.direct_peer_connection {
                    "Yes"
                } else {
                    "No"
                });
                ui.end_row();
                ui.strong("Classic BitTorrent");
                ui.label(if status.compatibility.classic_bittorrent_active {
                    "Active"
                } else {
                    "Not implemented / inactive"
                });
                ui.end_row();
                ui.strong("M3 private discovery");
                ui.label(if status.compatibility.m3_private_discovery_implemented {
                    "Implemented"
                } else {
                    "Research only"
                });
                ui.end_row();
                ui.strong("Anonymity");
                ui.label("Not guaranteed");
                ui.end_row();
            });
        ui.add_space(12.0);
        ui.label("The relay observes both endpoints, route identifiers, timing and traffic volume. The bootstrap observes peer IP addresses, content advertisements and queries, selected routes and timing.");
        ui.label(
            "Future M6 dual and classic compatibility modes are not available in this client.",
        );
    }

    fn diagnostics(&self, ui: &mut egui::Ui, snapshot: &DesktopSnapshot) {
        ui.heading("Diagnostics / Settings");
        let mut diagnostics = snapshot.diagnostic_text();
        ui.add(
            egui::TextEdit::multiline(&mut diagnostics)
                .desired_rows(18)
                .desired_width(f32::INFINITY)
                .interactive(true),
        );
        ui.label("The API bearer token and cryptographic session material are omitted.");
        if ui.button("Refresh now").clicked() {
            let _ = self.controller.refresh();
        }
    }

    fn delete_confirmation(&mut self, ctx: &egui::Context) {
        let Some(content_id) = self.confirm_delete.clone() else {
            return;
        };
        egui::Window::new("Delete managed copy?").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label("This removes TripTorrent metadata and its managed copy. The original imported source file is never deleted.");
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() { self.confirm_delete = None; }
                if ui.button("Delete managed copy").clicked() {
                    let _ = self.controller.remove_content(content_id.clone(), true);
                    self.selected_content = None;
                    self.confirm_delete = None;
                }
            });
        });
    }
}

impl eframe::App for DesktopApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.dispatch_dropped_files(ui.ctx());
        egui::CentralPanel::default().show(ui, |ui| self.render(ui));
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }
}

fn progress_fraction(transfer: &TransferRecord) -> f32 {
    if transfer.bytes_total == 0 {
        0.0
    } else {
        let verified = transfer.bytes_transferred.min(transfer.bytes_total);
        let scaled = u128::from(verified) * 10_000 / u128::from(transfer.bytes_total);
        let basis_points = u16::try_from(scaled).unwrap_or(10_000);
        f32::from(basis_points) / 10_000.0
    }
}

fn short_id(value: &str) -> &str {
    value.get(..12).unwrap_or(value)
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut divisor = 1_u64;
    let mut unit = 0;
    while bytes / divisor >= 1024 && unit + 1 < UNITS.len() {
        divisor *= 1024;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        let whole = bytes / divisor;
        let tenth = (bytes % divisor) * 10 / divisor;
        format!("{whole}.{tenth} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn structured_network_status_is_truthful() {
        let status = NetworkPrivacyStatus::current_prototype();
        assert_eq!(status.discovery, "temporary_centralized_m2_bootstrap");
        assert_eq!(status.data_path, "end_to_end_encrypted_relayed_swarm");
        assert!(!status.direct_peer_connection);
        assert!(!status.compatibility.classic_bittorrent_active);
        assert!(!status.compatibility.m3_private_discovery_implemented);
        assert!(!status.anonymity_guarantee);
    }

    #[test]
    fn verified_progress_never_exceeds_total() {
        let transfer = TransferRecord {
            id: 1,
            content_id: "a".repeat(64),
            direction: "download".into(),
            status: "running".into(),
            bytes_total: 10,
            bytes_transferred: 12,
            verified_chunks: 1,
            total_chunks: 1,
            current_rate_bps: 0,
            provider_count: 1,
            retry_count: 0,
            rejected_chunks: 0,
            provider_contributions: Vec::new(),
            resumed_chunks: 0,
            error: None,
            share_on_complete: true,
            network_path: "encrypted_relayed_swarm".into(),
        };
        assert!((progress_fraction(&transfer) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn diagnostics_do_not_include_unmodeled_secrets() {
        let snapshot = DesktopSnapshot::initial(PathBuf::from("node.toml"));
        let text = snapshot.diagnostic_text();
        assert!(!text.contains("Bearer"));
        assert!(!text.contains("api_token"));
        assert!(!text.contains("session key"));
    }

    #[test]
    fn representative_view_renders_headlessly() {
        let directory = tempdir().unwrap();
        let options = DesktopOptions {
            config_path: directory.path().join("node.toml"),
            data_dir: directory.path().join("data"),
            node_binary: directory.path().join("missing-sidecar"),
            api_listen: "127.0.0.1:7331".parse().unwrap(),
            bootstrap: None,
            poll_interval: Duration::from_secs(1),
            startup_timeout: Duration::from_millis(50),
            reconnect_backoff: Duration::from_secs(1),
        };
        let controller = DesktopController::start(options);
        let mut app = DesktopApp::new(controller);
        egui::__run_test_ui(|ui| app.render(ui));
    }
}
