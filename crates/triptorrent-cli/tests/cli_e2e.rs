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
const TEST_CORRUPT_CHUNKS_ENV: &str = "TRIPTORRENT_TEST_CORRUPT_CHUNKS";
const TEST_CHUNK_DELAY_ENV: &str = "TRIPTORRENT_TEST_CHUNK_DELAY_MS";
const PROVIDER_READY_MARKER: &str = "TRIPTORRENT_PROVIDER_READY";

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
        self.wait_for_stderr_count(expected, 1, timeout);
    }

    fn wait_for_stderr_count(&mut self, expected: &str, count: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            self.assert_running();
            if fs::read_to_string(&self.stderr_path)
                .unwrap_or_default()
                .matches(expected)
                .count()
                >= count
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
        .env_remove(TEST_IDLE_MARKER_ENV)
        .env_remove(TEST_CORRUPT_CHUNKS_ENV)
        .env_remove(TEST_CHUNK_DELAY_ENV);
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
    start_bootstrap_with_lease(log_directory, address, 30_000)
}

fn start_bootstrap_with_lease(
    log_directory: &Path,
    address: SocketAddr,
    lease_ms: u64,
) -> ManagedChild {
    let mut command = binary_command();
    command.args([
        "bootstrap",
        "--listen",
        &address.to_string(),
        "--lease-ms",
        &lease_ms.to_string(),
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
    start_share_with_options(
        log_directory,
        label,
        bootstrap,
        source,
        fast_poll,
        None,
        None,
    )
}

fn start_share_with_options(
    log_directory: &Path,
    label: &str,
    bootstrap: SocketAddr,
    source: &Path,
    fast_poll: bool,
    availability: Option<&str>,
    test_environment: Option<(&str, &str)>,
) -> ManagedChild {
    let mut command = binary_command();
    command
        .arg("share")
        .arg("--bootstrap")
        .arg(bootstrap.to_string())
        .arg("--file")
        .arg(source);
    if let Some(availability) = availability {
        command.arg("--available").arg(availability);
    }
    if fast_poll {
        command
            .env(TEST_POLL_INTERVAL_ENV, TEST_POLL_INTERVAL_MS)
            .env(TEST_IDLE_MARKER_ENV, TEST_IDLE_MARKER_AFTER);
    }
    if let Some((name, value)) = test_environment {
        command.env(name, value);
    }
    ManagedChild::spawn(label, command, log_directory)
}

fn wait_for_provider(process: &mut ManagedChild) {
    process.wait_for_stderr(PROVIDER_READY_MARKER, READY_TIMEOUT);
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

fn write_large_source(path: &Path, seed: u8, chunks: usize) -> Vec<u8> {
    let length = chunks * 32 * 1024 - 113;
    let bytes: Vec<_> = (0..length)
        .map(|index| index.to_le_bytes()[0].wrapping_mul(31) ^ seed)
        .collect();
    fs::write(path, &bytes).expect("write deterministic large source file");
    bytes
}

fn positive_provider_count(stderr: &str) -> usize {
    let stats = stderr
        .lines()
        .find(|line| line.starts_with("TRIPTORRENT_SWARM_STATS"))
        .expect("fetch emitted swarm statistics");
    let accepted = stats
        .split_whitespace()
        .find_map(|field| field.strip_prefix("accepted="))
        .expect("statistics include accepted counts");
    accepted
        .split(',')
        .filter(|entry| {
            entry
                .rsplit_once(':')
                .and_then(|(_, count)| count.parse::<usize>().ok())
                .is_some_and(|count| count > 0)
        })
        .count()
}

fn stat_value(stderr: &str, name: &str) -> usize {
    let prefix = format!("{name}=");
    stderr
        .lines()
        .find(|line| line.starts_with("TRIPTORRENT_SWARM_STATS"))
        .and_then(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix(&prefix))
        })
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("swarm statistics omitted {name}"))
}

fn start_ready_provider_pair(
    directory: &Path,
    bootstrap: SocketAddr,
    source: &Path,
    label_prefix: &str,
    delay_ms: Option<&str>,
) -> Vec<ManagedChild> {
    let environment = delay_ms.map(|value| (TEST_CHUNK_DELAY_ENV, value));
    let mut providers: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|suffix| {
            start_share_with_options(
                directory,
                &format!("{label_prefix}-{suffix}"),
                bootstrap,
                source,
                false,
                None,
                environment,
            )
        })
        .collect();
    for provider in &mut providers {
        wait_for_provider(provider);
    }
    providers
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

#[test]
fn cli_swarm_uses_multiple_providers() {
    let _guard = e2e_guard();
    let directory = tempdir().expect("create test directory");
    let addresses = free_addresses(3);
    let bootstrap_address = addresses[0];
    let mut bootstrap = start_bootstrap(directory.path(), bootstrap_address);
    let mut relay_a = start_relay(
        directory.path(),
        "swarm-relay-a",
        addresses[1],
        bootstrap_address,
        "relay-a",
    );
    let mut relay_b = start_relay(
        directory.path(),
        "swarm-relay-b",
        addresses[2],
        bootstrap_address,
        "relay-b",
    );
    let source = directory.path().join("swarm-source.bin");
    let destination = directory.path().join("swarm-output.bin");
    let expected = write_large_source(&source, 0x31, 18);
    let content_id = identify(directory.path(), "swarm-identify", &source, &expected);
    let mut shares: Vec<_> = (0..3)
        .map(|index| {
            start_share(
                directory.path(),
                &format!("swarm-provider-{index}"),
                bootstrap_address,
                &source,
                false,
            )
        })
        .collect();
    for share in &mut shares {
        wait_for_provider(share);
    }

    let fetch = start_fetch(
        directory.path(),
        "swarm-fetch",
        bootstrap_address,
        &content_id,
        &destination,
    );
    let fetch_output = fetch.wait_success(TRANSFER_TIMEOUT);
    assert!(
        positive_provider_count(&fetch_output.stderr) >= 2,
        "fewer than two providers contributed\n{}",
        fetch_output.diagnostics("swarm-fetch")
    );
    for share in shares {
        share.wait_success(TRANSFER_TIMEOUT);
    }
    let actual = fs::read(destination).expect("read swarm output");
    assert_eq!(actual, expected);
    assert_eq!(ContentId::digest(&actual).to_string(), content_id);
    assert!(fetch_output.stdout.contains(&content_id));
    relay_b.assert_running();
    relay_a.assert_running();
    bootstrap.assert_running();
}

#[test]
fn cli_swarm_combines_partial_availability() {
    let _guard = e2e_guard();
    let directory = tempdir().expect("create test directory");
    let addresses = free_addresses(3);
    let bootstrap_address = addresses[0];
    let mut bootstrap = start_bootstrap(directory.path(), bootstrap_address);
    let mut relay_a = start_relay(
        directory.path(),
        "partial-relay-a",
        addresses[1],
        bootstrap_address,
        "relay-a",
    );
    let mut relay_b = start_relay(
        directory.path(),
        "partial-relay-b",
        addresses[2],
        bootstrap_address,
        "relay-b",
    );
    let source = directory.path().join("partial-source.bin");
    let destination = directory.path().join("partial-output.bin");
    let expected = write_large_source(&source, 0x52, 12);
    let content_id = identify(directory.path(), "partial-identify", &source, &expected);
    let mut shares = vec![
        start_share_with_options(
            directory.path(),
            "partial-provider-a",
            bootstrap_address,
            &source,
            false,
            Some("0-5"),
            None,
        ),
        start_share_with_options(
            directory.path(),
            "partial-provider-b",
            bootstrap_address,
            &source,
            false,
            Some("4-11"),
            None,
        ),
        start_share_with_options(
            directory.path(),
            "partial-provider-c",
            bootstrap_address,
            &source,
            false,
            Some("2-3,8-9"),
            None,
        ),
    ];
    for share in &mut shares {
        wait_for_provider(share);
    }
    let fetch = start_fetch(
        directory.path(),
        "partial-fetch",
        bootstrap_address,
        &content_id,
        &destination,
    );
    let fetch_output = fetch.wait_success(TRANSFER_TIMEOUT);
    assert!(positive_provider_count(&fetch_output.stderr) >= 2);
    for share in shares {
        share.wait_success(TRANSFER_TIMEOUT);
    }
    assert_eq!(
        fs::read(destination).expect("read partial output"),
        expected
    );
    relay_b.assert_running();
    relay_a.assert_running();
    bootstrap.assert_running();
}

#[test]
fn cli_swarm_reassigns_after_provider_failure() {
    let _guard = e2e_guard();
    let directory = tempdir().expect("create test directory");
    let addresses = free_addresses(3);
    let bootstrap_address = addresses[0];
    let mut bootstrap = start_bootstrap(directory.path(), bootstrap_address);
    let mut relay_a = start_relay(
        directory.path(),
        "failure-relay-a",
        addresses[1],
        bootstrap_address,
        "relay-a",
    );
    let mut relay_b = start_relay(
        directory.path(),
        "failure-relay-b",
        addresses[2],
        bootstrap_address,
        "relay-b",
    );
    let source = directory.path().join("failure-source.bin");
    let destination = directory.path().join("failure-output.bin");
    let expected = write_large_source(&source, 0x73, 40);
    let content_id = identify(directory.path(), "failure-identify", &source, &expected);
    let mut failed = start_share_with_options(
        directory.path(),
        "failure-provider-a",
        bootstrap_address,
        &source,
        false,
        None,
        Some((TEST_CHUNK_DELAY_ENV, "100")),
    );
    let mut survivor_a = start_share_with_options(
        directory.path(),
        "failure-provider-b",
        bootstrap_address,
        &source,
        false,
        None,
        Some((TEST_CHUNK_DELAY_ENV, "20")),
    );
    let mut survivor_b = start_share_with_options(
        directory.path(),
        "failure-provider-c",
        bootstrap_address,
        &source,
        false,
        None,
        Some((TEST_CHUNK_DELAY_ENV, "20")),
    );
    wait_for_provider(&mut failed);
    wait_for_provider(&mut survivor_a);
    wait_for_provider(&mut survivor_b);
    let fetch = start_fetch(
        directory.path(),
        "failure-fetch",
        bootstrap_address,
        &content_id,
        &destination,
    );
    failed.wait_for_stderr_count("TRIPTORRENT_CHUNK_REQUESTED", 2, TRANSFER_TIMEOUT);
    let _ = failed.terminate();
    let fetch_output = fetch.wait_success(TRANSFER_TIMEOUT);
    assert!(
        stat_value(&fetch_output.stderr, "retries") >= 1,
        "provider failure did not reassign a chunk\n{}",
        fetch_output.diagnostics("failure-fetch")
    );
    survivor_a.wait_success(TRANSFER_TIMEOUT);
    survivor_b.wait_success(TRANSFER_TIMEOUT);
    assert_eq!(
        fs::read(destination).expect("read failure output"),
        expected
    );
    relay_b.assert_running();
    relay_a.assert_running();
    bootstrap.assert_running();
}

#[test]
fn cli_swarm_rejects_a_malicious_chunk() {
    let _guard = e2e_guard();
    let directory = tempdir().expect("create test directory");
    let addresses = free_addresses(3);
    let bootstrap_address = addresses[0];
    let mut bootstrap = start_bootstrap(directory.path(), bootstrap_address);
    let mut relay_a = start_relay(
        directory.path(),
        "malicious-relay-a",
        addresses[1],
        bootstrap_address,
        "relay-a",
    );
    let mut relay_b = start_relay(
        directory.path(),
        "malicious-relay-b",
        addresses[2],
        bootstrap_address,
        "relay-b",
    );
    let source = directory.path().join("malicious-source.bin");
    let destination = directory.path().join("malicious-output.bin");
    let expected = write_large_source(&source, 0x94, 16);
    let content_id = identify(directory.path(), "malicious-identify", &source, &expected);
    let mut malicious = start_share_with_options(
        directory.path(),
        "malicious-provider",
        bootstrap_address,
        &source,
        false,
        None,
        Some((TEST_CORRUPT_CHUNKS_ENV, "0,1,2")),
    );
    let mut honest_a = start_share(
        directory.path(),
        "honest-provider-a",
        bootstrap_address,
        &source,
        false,
    );
    let mut honest_b = start_share(
        directory.path(),
        "honest-provider-b",
        bootstrap_address,
        &source,
        false,
    );
    wait_for_provider(&mut malicious);
    wait_for_provider(&mut honest_a);
    wait_for_provider(&mut honest_b);
    let fetch = start_fetch(
        directory.path(),
        "malicious-fetch",
        bootstrap_address,
        &content_id,
        &destination,
    );
    let fetch_output = fetch.wait_success(TRANSFER_TIMEOUT);
    assert!(stat_value(&fetch_output.stderr, "retries") >= 1);
    malicious.wait_success(TRANSFER_TIMEOUT);
    honest_a.wait_success(TRANSFER_TIMEOUT);
    honest_b.wait_success(TRANSFER_TIMEOUT);
    assert_eq!(
        fs::read(destination).expect("read malicious output"),
        expected
    );
    relay_b.assert_running();
    relay_a.assert_running();
    bootstrap.assert_running();
}

#[test]
fn cli_swarm_resumes_verified_chunks_after_restart() {
    let _guard = e2e_guard();
    let directory = tempdir().expect("create test directory");
    let addresses = free_addresses(3);
    let bootstrap_address = addresses[0];
    let mut bootstrap = start_bootstrap_with_lease(directory.path(), bootstrap_address, 800);
    let mut relay_a = start_relay(
        directory.path(),
        "resume-relay-a",
        addresses[1],
        bootstrap_address,
        "relay-a",
    );
    let mut relay_b = start_relay(
        directory.path(),
        "resume-relay-b",
        addresses[2],
        bootstrap_address,
        "relay-b",
    );
    let source = directory.path().join("resume-source.bin");
    let destination = directory.path().join("resume-output.bin");
    let expected = write_large_source(&source, 0xb5, 48);
    let content_id = identify(directory.path(), "resume-identify", &source, &expected);
    let mut first_providers = start_ready_provider_pair(
        directory.path(),
        bootstrap_address,
        &source,
        "resume-first",
        Some("50"),
    );
    let mut first_fetch = start_fetch(
        directory.path(),
        "resume-first-fetch",
        bootstrap_address,
        &content_id,
        &destination,
    );
    first_fetch.wait_for_stderr_count("TRIPTORRENT_CHUNK_ACCEPTED", 6, TRANSFER_TIMEOUT);
    let _ = first_fetch.terminate();
    for provider in &mut first_providers {
        let _ = provider.terminate();
    }
    assert!(
        directory
            .path()
            .join("resume-output.bin.triptorrent-part")
            .exists()
    );
    assert!(
        directory
            .path()
            .join("resume-output.bin.triptorrent-state")
            .exists()
    );

    thread::sleep(Duration::from_millis(900));
    let second_providers = start_ready_provider_pair(
        directory.path(),
        bootstrap_address,
        &source,
        "resume-second",
        None,
    );
    let second_fetch = start_fetch(
        directory.path(),
        "resume-second-fetch",
        bootstrap_address,
        &content_id,
        &destination,
    );
    let fetch_output = second_fetch.wait_success(TRANSFER_TIMEOUT);
    assert!(
        stat_value(&fetch_output.stderr, "resumed") >= 6,
        "restart did not reuse verified chunks\n{}",
        fetch_output.diagnostics("resume-second-fetch")
    );
    for provider in second_providers {
        provider.wait_success(TRANSFER_TIMEOUT);
    }
    assert_eq!(
        fs::read(&destination).expect("read resumed output"),
        expected
    );
    assert!(
        !directory
            .path()
            .join("resume-output.bin.triptorrent-part")
            .exists()
    );
    assert!(
        !directory
            .path()
            .join("resume-output.bin.triptorrent-state")
            .exists()
    );
    relay_b.assert_running();
    relay_a.assert_running();
    bootstrap.assert_running();
}
