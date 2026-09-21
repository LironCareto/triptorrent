use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use triptorrent_core::ContentId;
use triptorrent_desktop::{ConnectionState, DesktopController, DesktopOptions, DesktopSnapshot};
use triptorrent_node_api::LocalApiClient;

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30);
const PROVIDER_READY: &str = "TRIPTORRENT_PROVIDER_READY";
const CHUNK_DELAY_ENV: &str = "TRIPTORRENT_TEST_CHUNK_DELAY_MS";
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn guard() -> MutexGuard<'static, ()> {
    TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug)]
struct ProcessOutput {
    status: Option<ExitStatus>,
    stdout: String,
    stderr: String,
}

impl ProcessOutput {
    fn diagnostics(&self, label: &str) -> String {
        let status = self
            .status
            .map_or_else(|| "running".into(), |status| status.to_string());
        format!(
            "{label}; status={status}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            self.stdout, self.stderr
        )
    }
}

struct ManagedChild {
    label: String,
    child: Option<Child>,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    stdout_offset: u64,
    stderr_offset: u64,
    stdout: String,
    stderr: String,
}

impl ManagedChild {
    fn spawn(label: &str, mut command: Command, logs: &Path) -> Self {
        let stdout_path = logs.join(format!("{label}.stdout.log"));
        let stderr_path = logs.join(format!("{label}.stderr.log"));
        command
            .stdout(Stdio::from(
                File::create(&stdout_path).expect("create stdout log"),
            ))
            .stderr(Stdio::from(
                File::create(&stderr_path).expect("create stderr log"),
            ));
        let child = command
            .spawn()
            .unwrap_or_else(|error| panic!("spawn {label}: {error}"));
        Self {
            label: label.into(),
            child: Some(child),
            stdout_path,
            stderr_path,
            stdout_offset: 0,
            stderr_offset: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn assert_running(&mut self) {
        self.read_new();
        if let Some(status) = self
            .child
            .as_mut()
            .expect("child present")
            .try_wait()
            .expect("query child")
        {
            self.child.take();
            let label = self.label.clone();
            let diagnostics = self.output(Some(status)).diagnostics(&label);
            panic!("{label} exited early\n{diagnostics}");
        }
    }

    fn wait_for_stderr(&mut self, marker: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            self.assert_running();
            if self.stderr.contains(marker) {
                return;
            }
            let label = self.label.clone();
            let diagnostics = self.output(None).diagnostics(&label);
            assert!(
                Instant::now() < deadline,
                "{label} did not emit {marker}\n{diagnostics}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn read_new(&mut self) {
        read_log(&self.stdout_path, &mut self.stdout_offset, &mut self.stdout);
        read_log(&self.stderr_path, &mut self.stderr_offset, &mut self.stderr);
    }

    fn terminate(&mut self) -> ProcessOutput {
        let status = self.child.take().and_then(|mut child| {
            if let Ok(Some(status)) = child.try_wait() {
                Some(status)
            } else {
                let _ = child.kill();
                child.wait().ok()
            }
        });
        self.output(status)
    }

    fn output(&mut self, status: Option<ExitStatus>) -> ProcessOutput {
        self.read_new();
        ProcessOutput {
            status,
            stdout: self.stdout.clone(),
            stderr: self.stderr.clone(),
        }
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

fn read_log(path: &Path, offset: &mut u64, output: &mut String) {
    let Ok(mut file) = OpenOptions::new().read(true).open(path) else {
        return;
    };
    if file.seek(SeekFrom::Start(*offset)).is_err() {
        return;
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_ok() {
        *offset += u64::try_from(bytes.len()).unwrap_or(0);
        output.push_str(&String::from_utf8_lossy(&bytes));
    }
}

fn binary_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_triptorrent"));
    for variable in [
        "TRIPTORRENT_DATA_DIR",
        "TRIPTORRENT_API_LISTEN",
        "TRIPTORRENT_BOOTSTRAP",
        "TRIPTORRENT_DOWNLOAD_LIMIT",
        "TRIPTORRENT_UPLOAD_LIMIT",
        "TRIPTORRENT_MAX_CONCURRENT_TRANSFERS",
        "TRIPTORRENT_LOG",
        CHUNK_DELAY_ENV,
    ] {
        command.env_remove(variable);
    }
    command
}

fn free_addresses(count: usize) -> Vec<SocketAddr> {
    let listeners: Vec<_> = (0..count)
        .map(|_| TcpListener::bind("127.0.0.1:0").expect("ephemeral bind"))
        .collect();
    listeners
        .iter()
        .map(|listener| listener.local_addr().expect("local address"))
        .collect()
}

fn wait_listener(process: &mut ManagedChild, address: SocketAddr) {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        process.assert_running();
        if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} did not listen",
            process.label
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn start_bootstrap(logs: &Path, address: SocketAddr) -> ManagedChild {
    let mut command = binary_command();
    command.args([
        "bootstrap",
        "--listen",
        &address.to_string(),
        "--lease-ms",
        "1000",
    ]);
    let mut child = ManagedChild::spawn("desktop-bootstrap", command, logs);
    wait_listener(&mut child, address);
    child
}

fn start_relay(
    logs: &Path,
    label: &str,
    address: SocketAddr,
    bootstrap: SocketAddr,
) -> ManagedChild {
    let mut command = binary_command();
    command.args([
        "relay",
        "--listen",
        &address.to_string(),
        "--bootstrap",
        &bootstrap.to_string(),
        "--id",
        label,
    ]);
    let mut child = ManagedChild::spawn(label, command, logs);
    let deadline = Instant::now() + Duration::from_millis(350);
    while Instant::now() < deadline {
        child.assert_running();
        thread::sleep(Duration::from_millis(20));
    }
    child
}

fn start_provider(
    logs: &Path,
    label: &str,
    bootstrap: SocketAddr,
    source: &Path,
    delay_ms: u64,
) -> ManagedChild {
    let mut command = binary_command();
    command
        .args(["share", "--bootstrap", &bootstrap.to_string(), "--file"])
        .arg(source);
    if delay_ms != 0 {
        command.env(CHUNK_DELAY_ENV, delay_ms.to_string());
    }
    let mut child = ManagedChild::spawn(label, command, logs);
    child.wait_for_stderr(PROVIDER_READY, READY_TIMEOUT);
    child
}

fn options(root: &Path, api: SocketAddr, bootstrap: Option<SocketAddr>) -> DesktopOptions {
    DesktopOptions {
        config_path: root.join("node.toml"),
        data_dir: root.join("data"),
        node_binary: PathBuf::from(env!("CARGO_BIN_EXE_triptorrent")),
        api_listen: api,
        bootstrap,
        poll_interval: Duration::from_millis(50),
        startup_timeout: READY_TIMEOUT,
        reconnect_backoff: Duration::from_millis(100),
    }
}

fn wait_snapshot(
    controller: &DesktopController,
    timeout: Duration,
    description: &str,
    predicate: impl Fn(&DesktopSnapshot) -> bool,
) -> DesktopSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = controller.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}: {snapshot:#?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn read_token(config: &Path) -> String {
    fs::read_to_string(config)
        .expect("read config")
        .lines()
        .find_map(|line| {
            line.strip_prefix("api_token = \"")
                .map(|value| value.trim_end_matches('"').to_owned())
        })
        .expect("api token")
}

fn raw_request(address: SocketAddr, request: &[u8]) -> (u16, Value) {
    let mut stream =
        TcpStream::connect_timeout(&address, Duration::from_secs(2)).expect("connect API");
    stream.write_all(request).expect("write request");
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("shutdown request");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("read response");
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP delimiter");
    let status = std::str::from_utf8(&response[..split])
        .expect("headers")
        .split_whitespace()
        .nth(1)
        .expect("status")
        .parse()
        .expect("numeric status");
    let body = serde_json::from_slice(&response[split + 4..]).expect("JSON response");
    (status, body)
}

struct NodeCleanup(PathBuf);

impl Drop for NodeCleanup {
    fn drop(&mut self) {
        if let Ok(client) = LocalApiClient::from_config(&self.0) {
            let _ = client.shutdown();
            thread::sleep(Duration::from_millis(100));
        }
    }
}

fn write_source(path: &Path, seed: u8, chunks: usize) -> Vec<u8> {
    let length = chunks * 32 * 1024 - 113;
    let bytes: Vec<_> = (0..length)
        .map(|index| index.to_le_bytes()[0].wrapping_mul(31) ^ seed)
        .collect();
    fs::write(path, &bytes).expect("write source");
    bytes
}

#[test]
#[allow(clippy::too_many_lines)]
fn desktop_lifecycle_library_auth_and_api_recovery() {
    let _guard = guard();
    let directory = tempdir().expect("temp directory");
    let api = free_addresses(1)[0];
    let desktop_options = options(directory.path(), api, None);
    let _cleanup = NodeCleanup(desktop_options.config_path.clone());
    let controller = DesktopController::start(desktop_options.clone());
    let connected = wait_snapshot(
        &controller,
        READY_TIMEOUT,
        "desktop-started node",
        |state| state.connection == ConnectionState::Connected,
    );
    assert_eq!(connected.daemon_launch_count, 1);
    assert!(
        connected
            .status
            .as_ref()
            .is_some_and(|status| status.api_ready)
    );

    let second = DesktopController::start(desktop_options.clone());
    let second_state = wait_snapshot(
        &second,
        READY_TIMEOUT,
        "second desktop connection",
        |state| state.connection == ConnectionState::Connected,
    );
    assert_eq!(
        second_state.daemon_launch_count, 0,
        "second controller started a duplicate daemon"
    );
    drop(second);

    let source = directory.path().join("original.bin");
    let bytes = write_source(&source, 0x42, 4);
    let content_id = ContentId::digest(&bytes).to_string();
    controller
        .add_content(source.clone(), true)
        .expect("queue add");
    let library = wait_snapshot(&controller, READY_TIMEOUT, "library import", |state| {
        state
            .content
            .iter()
            .any(|item| item.content_id == content_id)
    });
    let content = library
        .content
        .iter()
        .find(|item| item.content_id == content_id)
        .expect("content record");
    assert_eq!(fs::read(&content.data_path).expect("managed bytes"), bytes);
    assert_eq!(content.identities[0].namespace, "triptorrent");
    assert_eq!(content.identities[0].verification, "verified");
    assert!(
        !content.advertised,
        "no bootstrap means no active advertisement"
    );

    let (status, _) = raw_request(api, b"POST /v1/shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
    assert_eq!(status, 401);
    let token = read_token(&desktop_options.config_path);
    let diagnostics = library.diagnostic_text();
    assert!(!diagnostics.contains(&token));
    assert!(!diagnostics.contains("api_token"));

    let managed_path = content.data_path.clone();
    controller
        .remove_content(content_id.clone(), false)
        .expect("remove metadata");
    wait_snapshot(&controller, READY_TIMEOUT, "metadata removal", |state| {
        state
            .content
            .iter()
            .all(|item| item.content_id != content_id)
    });
    assert!(managed_path.exists());
    assert!(source.exists());
    controller
        .add_content(source.clone(), false)
        .expect("re-add content");
    wait_snapshot(&controller, READY_TIMEOUT, "content re-add", |state| {
        state
            .content
            .iter()
            .any(|item| item.content_id == content_id)
    });
    controller
        .remove_content(content_id.clone(), true)
        .expect("delete managed copy");
    wait_snapshot(&controller, READY_TIMEOUT, "managed deletion", |state| {
        state
            .content
            .iter()
            .all(|item| item.content_id != content_id)
    });
    assert!(!managed_path.exists());
    assert!(source.exists());

    let client = LocalApiClient::from_config(&desktop_options.config_path).expect("API client");
    client.shutdown().expect("simulate API loss");
    wait_snapshot(
        &controller,
        READY_TIMEOUT,
        "disconnect observation",
        |state| state.connection != ConnectionState::Connected,
    );
    let recovered = wait_snapshot(&controller, READY_TIMEOUT, "automatic restart", |state| {
        state.connection == ConnectionState::Connected && state.daemon_launch_count >= 2
    });
    assert!(recovered.status.is_some());

    controller.stop_node().expect("stop node action");
    wait_snapshot(&controller, READY_TIMEOUT, "explicit stop", |state| {
        state.connection == ConnectionState::Stopped
    });
    controller.start_node().expect("start node action");
    wait_snapshot(&controller, READY_TIMEOUT, "explicit restart", |state| {
        state.connection == ConnectionState::Connected
    });
    controller.stop_node().expect("final stop");
    wait_snapshot(&controller, READY_TIMEOUT, "final stopped state", |state| {
        state.connection == ConnectionState::Stopped
    });

    let bad_root = directory.path().join("bad-start");
    let mut bad_options = options(&bad_root, free_addresses(1)[0], None);
    bad_options.node_binary = bad_root.join("missing-triptorrent.exe");
    let bad_controller = DesktopController::start(bad_options);
    let bad = wait_snapshot(
        &bad_controller,
        READY_TIMEOUT,
        "actionable startup failure",
        |state| state.connection == ConnectionState::Unhealthy,
    );
    assert!(
        bad.last_error
            .as_deref()
            .is_some_and(|error| error.contains("sidecar"))
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn desktop_fetch_progress_pause_resume_and_restart() {
    let _guard = guard();
    let directory = tempdir().expect("temp directory");
    let addresses = free_addresses(4);
    let bootstrap_address = addresses[0];
    let mut bootstrap = start_bootstrap(directory.path(), bootstrap_address);
    let mut relay_a = start_relay(
        directory.path(),
        "desktop-relay-a",
        addresses[1],
        bootstrap_address,
    );
    let mut relay_b = start_relay(
        directory.path(),
        "desktop-relay-b",
        addresses[2],
        bootstrap_address,
    );
    let source = directory.path().join("large-source.bin");
    let expected = write_source(&source, 0x73, 72);
    let content_id = ContentId::digest(&expected).to_string();
    let mut provider_a = start_provider(
        directory.path(),
        "desktop-provider-a",
        bootstrap_address,
        &source,
        55,
    );
    let mut provider_b = start_provider(
        directory.path(),
        "desktop-provider-b",
        bootstrap_address,
        &source,
        55,
    );

    let desktop_options = options(directory.path(), addresses[3], Some(bootstrap_address));
    let _cleanup = NodeCleanup(desktop_options.config_path.clone());
    let controller = DesktopController::start(desktop_options.clone());
    wait_snapshot(&controller, READY_TIMEOUT, "networked node", |state| {
        state.connection == ConnectionState::Connected
    });
    controller
        .start_fetch(content_id.clone(), true)
        .expect("start fetch");
    let running = wait_snapshot(
        &controller,
        TRANSFER_TIMEOUT,
        "verified transfer progress",
        |state| {
            state.transfers.iter().any(|transfer| {
                transfer.content_id == content_id
                    && transfer.status == "running"
                    && transfer.bytes_transferred > 0
                    && transfer.bytes_transferred < transfer.bytes_total
            })
        },
    );
    let transfer = running
        .transfers
        .iter()
        .find(|item| item.content_id == content_id)
        .expect("running transfer");
    assert!(transfer.verified_chunks > 0);
    assert!(transfer.provider_count >= 1);
    let transfer_id = transfer.id;
    controller.pause(transfer_id).expect("pause transfer");
    let paused = wait_snapshot(&controller, TRANSFER_TIMEOUT, "paused transfer", |state| {
        state
            .transfers
            .iter()
            .any(|item| item.id == transfer_id && item.status == "paused")
    });
    let paused_transfer = paused
        .transfers
        .iter()
        .find(|item| item.id == transfer_id)
        .expect("paused record");
    let paused_bytes = paused_transfer.bytes_transferred;
    let paused_chunks = paused_transfer.verified_chunks;
    assert!(paused_bytes > 0 && paused_bytes < paused_transfer.bytes_total);
    thread::sleep(Duration::from_millis(500));
    let still_paused = controller.snapshot();
    let unchanged = still_paused
        .transfers
        .iter()
        .find(|item| item.id == transfer_id)
        .expect("still paused");
    assert_eq!(unchanged.bytes_transferred, paused_bytes);
    assert_eq!(unchanged.verified_chunks, paused_chunks);
    let content_dir = desktop_options.data_dir.join("content").join(&content_id);
    assert!(content_dir.join("data.triptorrent-part").exists());
    assert!(content_dir.join("data.triptorrent-state").exists());

    provider_a.terminate();
    provider_b.terminate();
    let api = LocalApiClient::from_config(&desktop_options.config_path).expect("API client");
    api.shutdown().expect("restart node");
    let restarted = wait_snapshot(
        &controller,
        READY_TIMEOUT,
        "node restart while paused",
        |state| state.connection == ConnectionState::Connected && state.daemon_launch_count >= 2,
    );
    let after_restart = restarted
        .transfers
        .iter()
        .find(|item| item.id == transfer_id)
        .expect("transfer after restart");
    assert_eq!(after_restart.status, "paused");
    assert_eq!(after_restart.bytes_transferred, paused_bytes);

    thread::sleep(Duration::from_millis(1_100));
    let mut resumed_a = start_provider(
        directory.path(),
        "desktop-resumed-a",
        bootstrap_address,
        &source,
        0,
    );
    let mut resumed_b = start_provider(
        directory.path(),
        "desktop-resumed-b",
        bootstrap_address,
        &source,
        0,
    );
    controller.resume(transfer_id).expect("resume transfer");
    let completed = wait_snapshot(
        &controller,
        TRANSFER_TIMEOUT,
        "completed resumed transfer",
        |state| {
            state
                .transfers
                .iter()
                .any(|item| item.id == transfer_id && item.status == "completed")
        },
    );
    let completed_transfer = completed
        .transfers
        .iter()
        .find(|item| item.id == transfer_id)
        .expect("completed transfer");
    assert_eq!(
        completed_transfer.bytes_transferred,
        u64::try_from(expected.len()).unwrap()
    );
    assert!(completed_transfer.resumed_chunks >= paused_chunks);
    assert!(!completed_transfer.provider_contributions.is_empty());
    assert_eq!(
        fs::read(content_dir.join("data")).expect("completed content"),
        expected
    );
    assert!(!content_dir.join("data.triptorrent-state").exists());
    assert_eq!(
        completed.network_privacy.discovery,
        "centralized_m2_derived_testnet_bootstrap"
    );
    assert_eq!(
        completed.network_privacy.data_path,
        "end_to_end_encrypted_relayed_swarm"
    );
    assert!(!completed.network_privacy.direct_peer_connection);
    assert!(
        !completed
            .network_privacy
            .compatibility
            .classic_bittorrent_active
    );
    assert!(!completed.network_privacy.anonymity_guarantee);

    controller.stop_node().expect("stop test node");
    wait_snapshot(&controller, READY_TIMEOUT, "test node stop", |state| {
        state.connection == ConnectionState::Stopped
    });
    resumed_a.terminate();
    resumed_b.terminate();
    relay_a.assert_running();
    relay_b.assert_running();
    bootstrap.assert_running();
}
