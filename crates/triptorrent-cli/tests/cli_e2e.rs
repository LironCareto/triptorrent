use std::fs::{self, File};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use triptorrent_core::ContentId;
use triptorrent_overlay::BootstrapClient;
use triptorrent_protocol::RelayAdvertisement;

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30);
const TEST_POLL_INTERVAL_ENV: &str = "TRIPTORRENT_TEST_OVERLAY_POLL_MS";
const TEST_POLL_INTERVAL_MS: &str = "1";
const TEST_IDLE_MARKER_ENV: &str = "TRIPTORRENT_TEST_IDLE_MARKER_AFTER";
const TEST_IDLE_MARKER_AFTER: &str = "601";
const TEST_IDLE_MARKER: &str = "TRIPTORRENT_TEST_IDLE_POLLS_REACHED=601";

static E2E_LOCK: Mutex<()> = Mutex::new(());

struct ProcessOutput {
    status: Option<ExitStatus>,
    stdout: String,
    stderr: String,
}

impl ProcessOutput {
    fn diagnostics(&self, label: &str) -> String {
        let status = self
            .status
            .map_or_else(|| "unknown".to_owned(), |status| status.to_string());
        format!(
            "process {label}; status={status}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            self.stdout, self.stderr
        )
    }
}

struct ManagedChild {
    label: String,
    child: Option<Child>,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

impl ManagedChild {
    fn spawn(label: &str, mut command: Command, log_directory: &Path) -> Self {
        let stdout_path = log_directory.join(format!("{label}.stdout.log"));
        let stderr_path = log_directory.join(format!("{label}.stderr.log"));
        let stdout = File::create(&stdout_path).expect("create child stdout log");
        let stderr = File::create(&stderr_path).expect("create child stderr log");
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        let child = command
            .spawn()
            .unwrap_or_else(|error| panic!("failed to spawn {label}: {error}"));
        Self {
            label: label.to_owned(),
            child: Some(child),
            stdout_path,
            stderr_path,
        }
    }

    fn assert_running(&mut self) {
        let status = self
            .child
            .as_mut()
            .expect("child process should be present")
            .try_wait()
            .unwrap_or_else(|error| panic!("failed to query {}: {error}", self.label));
        if let Some(status) = status {
            self.child.take();
            let output = self.read_output(Some(status));
            panic!(
                "{} exited early\n{}",
                self.label,
                output.diagnostics(&self.label)
            );
        }
    }

    fn wait_success(mut self, timeout: Duration) -> ProcessOutput {
        let deadline = Instant::now() + timeout;
        loop {
            let status = self
                .child
                .as_mut()
                .expect("child process should be present")
                .try_wait()
                .unwrap_or_else(|error| panic!("failed to wait for {}: {error}", self.label));
            if let Some(status) = status {
                self.child.take();
                let output = self.read_output(Some(status));
                assert!(
                    status.success(),
                    "{} failed\n{}",
                    self.label,
                    output.diagnostics(&self.label)
                );
                return output;
            }
            if Instant::now() >= deadline {
                let output = self.terminate();
                panic!(
                    "{} exceeded {timeout:?}\n{}",
                    self.label,
                    output.diagnostics(&self.label)
                );
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_stderr(&mut self, expected: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            self.assert_running();
            if fs::read_to_string(&self.stderr_path)
                .unwrap_or_default()
                .contains(expected)
            {
                return;
            }
            if Instant::now() >= deadline {
                let output = self.terminate();
                panic!(
                    "{} did not emit {expected:?} within {timeout:?}\n{}",
                    self.label,
                    output.diagnostics(&self.label)
                );
            }
            thread::sleep(Duration::from_millis(20));
        }
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
        self.read_output(status)
    }

    fn read_output(&self, status: Option<ExitStatus>) -> ProcessOutput {
        ProcessOutput {
            status,
            stdout: fs::read_to_string(&self.stdout_path).unwrap_or_default(),
            stderr: fs::read_to_string(&self.stderr_path).unwrap_or_default(),
        }
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if self.child.is_some() {
            let output = self.terminate();
            if thread::panicking() {
                eprintln!("{}", output.diagnostics(&self.label));
            }
        }
    }
}

fn e2e_guard() -> MutexGuard<'static, ()> {
    E2E_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn binary_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_triptorrent"));
    command
        .env_remove(TEST_POLL_INTERVAL_ENV)
        .env_remove(TEST_IDLE_MARKER_ENV);
    command
}

fn free_addresses(count: usize) -> Vec<SocketAddr> {
    let listeners: Vec<_> = (0..count)
        .map(|_| TcpListener::bind("127.0.0.1:0").expect("bind ephemeral test port"))
        .collect();
    listeners
        .iter()
        .map(|listener| listener.local_addr().expect("read ephemeral test address"))
        .collect()
}

fn wait_for_listener(process: &mut ManagedChild, address: SocketAddr) {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        process.assert_running();
        if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} did not listen on {address} within {READY_TIMEOUT:?}",
            process.label
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_startup(process: &mut ManagedChild) {
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        process.assert_running();
        thread::sleep(Duration::from_millis(20));
    }
    process.assert_running();
}

fn start_bootstrap(log_directory: &Path, address: SocketAddr) -> ManagedChild {
    let mut command = binary_command();
    command.args([
        "bootstrap",
        "--listen",
        &address.to_string(),
        "--lease-ms",
        "30000",
    ]);
    let mut process = ManagedChild::spawn("bootstrap", command, log_directory);
    wait_for_listener(&mut process, address);
    process
}

fn start_relay(
    log_directory: &Path,
    label: &str,
    address: SocketAddr,
    bootstrap: SocketAddr,
    relay_id: &str,
) -> ManagedChild {
    let mut command = binary_command();
    command
        .arg("relay")
        .arg("--listen")
        .arg(address.to_string())
        .arg("--bootstrap")
        .arg(bootstrap.to_string())
        .arg("--id")
        .arg(relay_id);
    let mut process = ManagedChild::spawn(label, command, log_directory);
    wait_for_startup(&mut process);
    process
}

fn start_share(
    log_directory: &Path,
    label: &str,
    bootstrap: SocketAddr,
    source: &Path,
    fast_poll: bool,
) -> ManagedChild {
    let mut command = binary_command();
    command
        .arg("share")
        .arg("--bootstrap")
        .arg(bootstrap.to_string())
        .arg("--file")
        .arg(source);
    if fast_poll {
        command
            .env(TEST_POLL_INTERVAL_ENV, TEST_POLL_INTERVAL_MS)
            .env(TEST_IDLE_MARKER_ENV, TEST_IDLE_MARKER_AFTER);
    }
    ManagedChild::spawn(label, command, log_directory)
}

fn start_fetch(
    log_directory: &Path,
    label: &str,
    bootstrap: SocketAddr,
    content_id: &str,
    destination: &Path,
) -> ManagedChild {
    let mut command = binary_command();
    command
        .arg("fetch")
        .arg("--bootstrap")
        .arg(bootstrap.to_string())
        .arg("--content")
        .arg(content_id)
        .arg("--output")
        .arg(destination);
    ManagedChild::spawn(label, command, log_directory)
}

fn identify(log_directory: &Path, label: &str, source: &Path, expected: &[u8]) -> String {
    let mut command = binary_command();
    command.arg("id").arg(source);
    let output = ManagedChild::spawn(label, command, log_directory).wait_success(COMMAND_TIMEOUT);
    let content_id = output.stdout.trim().to_owned();
    assert_eq!(
        content_id,
        ContentId::digest(expected).to_string(),
        "CLI returned an unexpected content ID\n{}",
        output.diagnostics(label)
    );
    content_id
}

fn write_source(path: &Path, seed: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in 0_u8..=255 {
        bytes.extend([value ^ seed; 1_025]);
    }
    fs::write(path, &bytes).expect("write deterministic source file");
    bytes
}

fn assert_transfer(
    share: ManagedChild,
    fetch: ManagedChild,
    destination: &Path,
    expected: &[u8],
    content_id: &str,
) {
    let fetch_output = fetch.wait_success(TRANSFER_TIMEOUT);
    let share_output = share.wait_success(TRANSFER_TIMEOUT);
    assert!(
        fetch_output.stdout.contains(content_id),
        "fetch output omitted the content ID\n{}",
        fetch_output.diagnostics("fetch")
    );
    assert!(
        share_output.stdout.contains("transfer complete"),
        "share output omitted completion\n{}",
        share_output.diagnostics("share")
    );
    assert_eq!(
        fs::read(destination).expect("read fetched file"),
        expected,
        "fetched bytes differ from the source"
    );
}

#[test]
fn cli_m2_end_to_end_transfer() {
    let _guard = e2e_guard();
    let directory = tempdir().expect("create test directory");
    let addresses = free_addresses(3);
    let bootstrap_address = addresses[0];
    let mut bootstrap = start_bootstrap(directory.path(), bootstrap_address);
    let mut relay_a = start_relay(
        directory.path(),
        "relay-a",
        addresses[1],
        bootstrap_address,
        "relay-a",
    );
    let mut relay_b = start_relay(
        directory.path(),
        "relay-b",
        addresses[2],
        bootstrap_address,
        "relay-b",
    );

    let source = directory.path().join("source.bin");
    let destination = directory.path().join("destination.bin");
    let expected = write_source(&source, 0x21);
    let content_id = identify(directory.path(), "identify", &source, &expected);
    let mut share = start_share(directory.path(), "share", bootstrap_address, &source, false);
    share.assert_running();
    let fetch = start_fetch(
        directory.path(),
        "fetch",
        bootstrap_address,
        &content_id,
        &destination,
    );

    assert_transfer(share, fetch, &destination, &expected, &content_id);
    relay_b.assert_running();
    relay_a.assert_running();
    bootstrap.assert_running();
}

#[test]
fn cli_share_remains_advertised_while_idle() {
    let _guard = e2e_guard();
    let directory = tempdir().expect("create test directory");
    let addresses = free_addresses(2);
    let bootstrap_address = addresses[0];
    let mut bootstrap = start_bootstrap(directory.path(), bootstrap_address);
    let mut relay = start_relay(
        directory.path(),
        "relay",
        addresses[1],
        bootstrap_address,
        "relay-a",
    );

    let source = directory.path().join("idle-source.bin");
    let destination = directory.path().join("idle-destination.bin");
    let expected = write_source(&source, 0x42);
    let content_id = identify(directory.path(), "idle-identify", &source, &expected);
    let mut share = start_share(
        directory.path(),
        "idle-share",
        bootstrap_address,
        &source,
        true,
    );

    share.wait_for_stderr(TEST_IDLE_MARKER, COMMAND_TIMEOUT);
    share.assert_running();
    let fetch = start_fetch(
        directory.path(),
        "idle-fetch",
        bootstrap_address,
        &content_id,
        &destination,
    );
    assert_transfer(share, fetch, &destination, &expected, &content_id);
    relay.assert_running();
    bootstrap.assert_running();
}

#[test]
fn cli_relay_failover_uses_the_live_relay() {
    let _guard = e2e_guard();
    let directory = tempdir().expect("create test directory");
    let addresses = free_addresses(3);
    let bootstrap_address = addresses[0];
    let dead_relay_address = addresses[1];
    let mut bootstrap = start_bootstrap(directory.path(), bootstrap_address);
    BootstrapClient::new(bootstrap_address)
        .register_relay(RelayAdvertisement {
            relay_id: "relay-a-dead".into(),
            address: dead_relay_address.to_string(),
        })
        .expect("register the unreachable relay");
    let mut live_relay = start_relay(
        directory.path(),
        "relay-live",
        addresses[2],
        bootstrap_address,
        "relay-b-live",
    );

    let source = directory.path().join("failover-source.bin");
    let destination = directory.path().join("failover-destination.bin");
    let expected = write_source(&source, 0x63);
    let content_id = identify(directory.path(), "failover-identify", &source, &expected);
    let share = start_share(
        directory.path(),
        "failover-share",
        bootstrap_address,
        &source,
        false,
    );
    let fetch = start_fetch(
        directory.path(),
        "failover-fetch",
        bootstrap_address,
        &content_id,
        &destination,
    );

    assert_transfer(share, fetch, &destination, &expected, &content_id);
    live_relay.assert_running();
    bootstrap.assert_running();
}

#[test]
fn cli_simultaneous_transfers_use_independent_routes() {
    let _guard = e2e_guard();
    let directory = tempdir().expect("create test directory");
    let addresses = free_addresses(3);
    let bootstrap_address = addresses[0];
    let mut bootstrap = start_bootstrap(directory.path(), bootstrap_address);
    let mut relay_a = start_relay(
        directory.path(),
        "multi-relay-a",
        addresses[1],
        bootstrap_address,
        "relay-a",
    );
    let mut relay_b = start_relay(
        directory.path(),
        "multi-relay-b",
        addresses[2],
        bootstrap_address,
        "relay-b",
    );

    let source_a = directory.path().join("source-a.bin");
    let source_b = directory.path().join("source-b.bin");
    let destination_a = directory.path().join("destination-a.bin");
    let destination_b = directory.path().join("destination-b.bin");
    let expected_a = write_source(&source_a, 0x11);
    let expected_b = write_source(&source_b, 0x77);
    let content_a = identify(directory.path(), "identify-a", &source_a, &expected_a);
    let content_b = identify(directory.path(), "identify-b", &source_b, &expected_b);

    let share_a = start_share(
        directory.path(),
        "share-a",
        bootstrap_address,
        &source_a,
        false,
    );
    let share_b = start_share(
        directory.path(),
        "share-b",
        bootstrap_address,
        &source_b,
        false,
    );
    let fetch_a = start_fetch(
        directory.path(),
        "fetch-a",
        bootstrap_address,
        &content_a,
        &destination_a,
    );
    let fetch_b = start_fetch(
        directory.path(),
        "fetch-b",
        bootstrap_address,
        &content_b,
        &destination_b,
    );

    let fetch_output_a = fetch_a.wait_success(TRANSFER_TIMEOUT);
    let fetch_output_b = fetch_b.wait_success(TRANSFER_TIMEOUT);
    let share_output_a = share_a.wait_success(TRANSFER_TIMEOUT);
    let share_output_b = share_b.wait_success(TRANSFER_TIMEOUT);
    assert!(fetch_output_a.stdout.contains(&content_a));
    assert!(fetch_output_b.stdout.contains(&content_b));
    assert!(share_output_a.stdout.contains("transfer complete"));
    assert!(share_output_b.stdout.contains("transfer complete"));
    assert_eq!(fs::read(destination_a).expect("read output A"), expected_a);
    assert_eq!(fs::read(destination_b).expect("read output B"), expected_b);
    relay_b.assert_running();
    relay_a.assert_running();
    bootstrap.assert_running();
}
