use serde_json::Value;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use triptorrent_core::ContentId;

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(45);
const CHUNK_DELAY_ENV: &str = "TRIPTORRENT_TEST_CHUNK_DELAY_MS";
const PROVIDER_READY: &str = "TRIPTORRENT_PROVIDER_READY";

static NODE_E2E_LOCK: Mutex<()> = Mutex::new(());

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
    fn spawn(label: &str, mut command: Command, logs: &Path) -> Self {
        let stdout_path = logs.join(format!("{label}.stdout.log"));
        let stderr_path = logs.join(format!("{label}.stderr.log"));
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                File::create(&stdout_path).expect("create stdout log"),
            ))
            .stderr(Stdio::from(
                File::create(&stderr_path).expect("create stderr log"),
            ));
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
            .expect("child present")
            .try_wait()
            .unwrap_or_else(|error| panic!("failed to query {}: {error}", self.label));
        if let Some(status) = status {
            self.child.take();
            let output = self.output(Some(status));
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
            if let Some(status) = self
                .child
                .as_mut()
                .expect("child present")
                .try_wait()
                .expect("query child")
            {
                self.child.take();
                let output = self.output(Some(status));
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
                    "{} timed out\n{}",
                    self.label,
                    output.diagnostics(&self.label)
                );
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_stderr_count(&mut self, marker: &str, count: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            self.assert_running();
            if fs::read_to_string(&self.stderr_path)
                .unwrap_or_default()
                .matches(marker)
                .count()
                >= count
            {
                return;
            }
            if Instant::now() >= deadline {
                let output = self.terminate();
                panic!(
                    "{} did not emit {marker:?}\n{}",
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
        self.output(status)
    }

    fn output(&self, status: Option<ExitStatus>) -> ProcessOutput {
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

fn guard() -> MutexGuard<'static, ()> {
    NODE_E2E_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn binary_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_triptorrent"));
    for name in [
        "TRIPTORRENT_DATA_DIR",
        "TRIPTORRENT_API_LISTEN",
        "TRIPTORRENT_BOOTSTRAP",
        "TRIPTORRENT_DOWNLOAD_LIMIT",
        "TRIPTORRENT_UPLOAD_LIMIT",
        "TRIPTORRENT_MAX_CONCURRENT_TRANSFERS",
        "TRIPTORRENT_LOG",
        CHUNK_DELAY_ENV,
    ] {
        command.env_remove(name);
    }
    command
}

fn free_addresses(count: usize) -> Vec<SocketAddr> {
    let listeners: Vec<_> = (0..count)
        .map(|_| TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port"))
        .collect();
    listeners
        .iter()
        .map(|listener| listener.local_addr().expect("read local address"))
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
            "{} did not listen at {address}",
            process.label
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn run_command(logs: &Path, label: &str, args: &[&str]) -> ProcessOutput {
    let mut command = binary_command();
    command.args(args);
    ManagedChild::spawn(label, command, logs).wait_success(COMMAND_TIMEOUT)
}

fn run_command_failure(logs: &Path, label: &str, args: &[&str]) -> ProcessOutput {
    let mut command = binary_command();
    command.args(args);
    let mut child = ManagedChild::spawn(label, command, logs);
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        if let Some(status) = child
            .child
            .as_mut()
            .expect("child present")
            .try_wait()
            .expect("query child")
        {
            child.child.take();
            let output = child.output(Some(status));
            assert!(
                !status.success(),
                "{label} unexpectedly succeeded\n{}",
                output.diagnostics(label)
            );
            return output;
        }
        if Instant::now() >= deadline {
            let output = child.terminate();
            panic!("{label} timed out\n{}", output.diagnostics(label));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn json_output(output: &ProcessOutput, label: &str) -> Value {
    serde_json::from_str(&output.stdout)
        .unwrap_or_else(|error| panic!("invalid JSON from {label}: {error}\n{}", output.stdout))
}

fn init_node(
    logs: &Path,
    label: &str,
    config: &Path,
    data: &Path,
    api: SocketAddr,
    bootstrap: Option<SocketAddr>,
) {
    let mut command = binary_command();
    command
        .arg("node")
        .arg("init")
        .arg("--config")
        .arg(config)
        .arg("--data-dir")
        .arg(data)
        .arg("--api-listen")
        .arg(api.to_string());
    if let Some(bootstrap) = bootstrap {
        command.arg("--bootstrap").arg(bootstrap.to_string());
    }
    ManagedChild::spawn(label, command, logs).wait_success(COMMAND_TIMEOUT);
}

fn start_node(logs: &Path, label: &str, config: &Path, api: SocketAddr) -> ManagedChild {
    let mut command = binary_command();
    command.arg("node").arg("start").arg("--config").arg(config);
    let mut process = ManagedChild::spawn(label, command, logs);
    wait_for_listener(&mut process, api);
    process
}

fn config_arg(config: &Path) -> String {
    config.to_string_lossy().into_owned()
}

fn stop_node(logs: &Path, label: &str, config: &Path, node: ManagedChild) -> ProcessOutput {
    let config = config_arg(config);
    run_command(
        logs,
        &format!("{label}-stop-client"),
        &["node", "stop", "--config", &config],
    );
    node.wait_success(COMMAND_TIMEOUT)
}

fn raw_request(address: SocketAddr, request: &[u8]) -> (u16, Value) {
    let mut stream =
        TcpStream::connect_timeout(&address, Duration::from_secs(2)).expect("connect to local API");
    stream.write_all(request).expect("write API request");
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("finish API request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read API response");
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP response delimiter");
    let headers = std::str::from_utf8(&response[..split]).expect("HTTP response headers");
    let status = headers
        .split_whitespace()
        .nth(1)
        .expect("HTTP status")
        .parse()
        .expect("numeric HTTP status");
    let body = serde_json::from_slice(&response[split + 4..]).expect("JSON HTTP body");
    (status, body)
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
    let mut process = ManagedChild::spawn("node-bootstrap", command, logs);
    wait_for_listener(&mut process, address);
    process
}

fn start_relay(
    logs: &Path,
    label: &str,
    address: SocketAddr,
    bootstrap: SocketAddr,
) -> ManagedChild {
    let mut command = binary_command();
    command
        .arg("relay")
        .arg("--listen")
        .arg(address.to_string())
        .arg("--bootstrap")
        .arg(bootstrap.to_string())
        .arg("--id")
        .arg(label);
    let mut process = ManagedChild::spawn(label, command, logs);
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        process.assert_running();
        thread::sleep(Duration::from_millis(20));
    }
    process
}

fn start_provider(
    logs: &Path,
    label: &str,
    bootstrap: SocketAddr,
    source: &Path,
    delay_ms: Option<&str>,
) -> ManagedChild {
    let mut command = binary_command();
    command
        .arg("share")
        .arg("--bootstrap")
        .arg(bootstrap.to_string())
        .arg("--file")
        .arg(source);
    if let Some(delay) = delay_ms {
        command.env(CHUNK_DELAY_ENV, delay);
    }
    let mut provider = ManagedChild::spawn(label, command, logs);
    provider.wait_for_stderr_count(PROVIDER_READY, 1, READY_TIMEOUT);
    provider
}

fn write_source(path: &Path, seed: u8, chunks: usize) -> Vec<u8> {
    let length = chunks * 32 * 1024 - 113;
    let bytes: Vec<_> = (0..length)
        .map(|index| index.to_le_bytes()[0].wrapping_mul(31) ^ seed)
        .collect();
    fs::write(path, &bytes).expect("write deterministic source");
    bytes
}

fn fetch_via_node(logs: &Path, label: &str, config: &Path, content: &str) -> i64 {
    let config = config_arg(config);
    let output = run_command(
        logs,
        label,
        &["node", "fetch", "--config", &config, "--content", content],
    );
    json_output(&output, label)["transfer_id"]
        .as_i64()
        .expect("transfer ID")
}

fn wait_transfer(logs: &Path, label: &str, config: &Path, transfer_id: i64) -> Value {
    let deadline = Instant::now() + TRANSFER_TIMEOUT;
    let config = config_arg(config);
    let transfer = transfer_id.to_string();
    let mut attempt = 0_u64;
    loop {
        let output = run_command(
            logs,
            &format!("{label}-{attempt}"),
            &["transfer", "show", "--config", &config, &transfer],
        );
        let value = json_output(&output, label);
        match value["status"].as_str() {
            Some("completed") => return value,
            Some("failed") => panic!("transfer failed: {value}"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "transfer timed out: {value}");
        attempt += 1;
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn node_lifecycle_storage_configuration_and_api_errors() {
    let _guard = guard();
    let directory = tempdir().expect("create test directory");
    let addresses = free_addresses(3);
    let configured_api = addresses[0];
    let environment_api = addresses[1];
    let cli_api = addresses[2];
    let config = directory.path().join("node.toml");
    let data = directory.path().join("data");
    init_node(
        directory.path(),
        "lifecycle-init",
        &config,
        &data,
        configured_api,
        None,
    );
    let config_text = fs::read_to_string(&config).expect("read generated config");
    let token = config_text
        .lines()
        .find_map(|line| {
            line.strip_prefix("api_token = \"")
                .map(|v| v.trim_end_matches('"'))
        })
        .expect("generated bearer token")
        .to_owned();
    assert!(config_text.contains("127.0.0.1"));
    assert!(token.len() >= 32);

    let mut override_command = binary_command();
    override_command
        .arg("node")
        .arg("start")
        .arg("--config")
        .arg(&config)
        .arg("--api-listen")
        .arg(cli_api.to_string())
        .env("TRIPTORRENT_API_LISTEN", environment_api.to_string());
    let mut override_node =
        ManagedChild::spawn("override-node", override_command, directory.path());
    wait_for_listener(&mut override_node, cli_api);
    assert!(TcpStream::connect(environment_api).is_err());
    let (status, health) = raw_request(
        cli_api,
        b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(status, 200);
    assert_eq!(health["api_ready"], true);
    override_node.terminate();

    let mut node = start_node(
        directory.path(),
        "lifecycle-node-first",
        &config,
        configured_api,
    );
    let config_string = config_arg(&config);
    let status = json_output(
        &run_command(
            directory.path(),
            "lifecycle-status",
            &["node", "status", "--config", &config_string],
        ),
        "node status",
    );
    assert_eq!(status["store_initialized"], true);
    assert_eq!(status["api_ready"], true);
    node.assert_running();

    let source = directory.path().join("source.bin");
    let expected = write_source(&source, 0x31, 3);
    let expected_id = ContentId::digest(&expected).to_string();
    let source_string = source.to_string_lossy().into_owned();
    let added = json_output(
        &run_command(
            directory.path(),
            "content-add",
            &["content", "add", "--config", &config_string, &source_string],
        ),
        "content add",
    );
    assert_eq!(added["content_id"], expected_id);
    assert_eq!(added["length"], expected.len());
    let managed_path = PathBuf::from(added["data_path"].as_str().expect("managed data path"));
    assert_eq!(
        fs::read(&managed_path).expect("read managed copy"),
        expected
    );

    let duplicate = run_command_failure(
        directory.path(),
        "content-duplicate",
        &["content", "add", "--config", &config_string, &source_string],
    );
    assert!(duplicate.stderr.contains("already indexed"));
    let (bad_status, bad_body) = raw_request(
        configured_api,
        format!(
            "POST /v1/content HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{{"
        )
        .as_bytes(),
    );
    assert_eq!(bad_status, 400);
    assert!(bad_body["error"]["code"].is_string());
    node.assert_running();

    let first_logs = stop_node(directory.path(), "lifecycle-first", &config, node);
    let combined_logs = format!("{}{}", first_logs.stdout, first_logs.stderr);
    assert!(combined_logs.contains("node_started"));
    assert!(!combined_logs.contains(&token));

    fs::write(&managed_path, b"corrupt managed bytes").expect("corrupt managed copy");
    let mut restarted = start_node(
        directory.path(),
        "lifecycle-node-restarted",
        &config,
        configured_api,
    );
    let listed = json_output(
        &run_command(
            directory.path(),
            "content-list-after-restart",
            &["content", "list", "--config", &config_string],
        ),
        "content list",
    );
    assert_eq!(listed[0]["content_id"], expected_id);
    assert_eq!(listed[0]["integrity"], "corrupt");
    assert_eq!(listed[0]["shared"], false);
    restarted.assert_running();

    run_command(
        directory.path(),
        "remove-metadata",
        &[
            "content",
            "remove",
            "--config",
            &config_string,
            "--content",
            &expected_id,
        ],
    );
    assert!(managed_path.exists());
    assert!(source.exists());

    let readded = json_output(
        &run_command(
            directory.path(),
            "content-readd",
            &["content", "add", "--config", &config_string, &source_string],
        ),
        "content re-add",
    );
    assert_eq!(readded["integrity"], "ok");
    run_command(
        directory.path(),
        "remove-bytes",
        &[
            "content",
            "remove",
            "--config",
            &config_string,
            "--content",
            &expected_id,
            "--delete-bytes",
        ],
    );
    assert!(!managed_path.exists());
    assert!(source.exists());
    let unknown = run_command_failure(
        directory.path(),
        "remove-unknown",
        &[
            "content",
            "remove",
            "--config",
            &config_string,
            "--content",
            &expected_id,
        ],
    );
    assert!(unknown.stderr.contains("unknown content ID"));
    stop_node(directory.path(), "lifecycle-second", &config, restarted);
}

#[test]
#[allow(clippy::too_many_lines)]
fn node_resumes_then_serves_after_restart_while_fetching() {
    let _guard = guard();
    let directory = tempdir().expect("create test directory");
    let addresses = free_addresses(5);
    let bootstrap_address = addresses[0];
    let mut bootstrap = start_bootstrap(directory.path(), bootstrap_address);
    let mut relay_a = start_relay(
        directory.path(),
        "node-relay-a",
        addresses[1],
        bootstrap_address,
    );
    let mut relay_b = start_relay(
        directory.path(),
        "node-relay-b",
        addresses[2],
        bootstrap_address,
    );

    let source_a = directory.path().join("source-a.bin");
    let expected_a = write_source(&source_a, 0x52, 40);
    let content_a = ContentId::digest(&expected_a).to_string();
    let config_a = directory.path().join("node-a.toml");
    let data_a = directory.path().join("node-a-data");
    init_node(
        directory.path(),
        "node-a-init",
        &config_a,
        &data_a,
        addresses[3],
        Some(bootstrap_address),
    );
    let mut providers_a = vec![
        start_provider(
            directory.path(),
            "resume-provider-a",
            bootstrap_address,
            &source_a,
            Some("80"),
        ),
        start_provider(
            directory.path(),
            "resume-provider-b",
            bootstrap_address,
            &source_a,
            Some("80"),
        ),
    ];
    let mut node_a = start_node(
        directory.path(),
        "node-a-download-first",
        &config_a,
        addresses[3],
    );
    let transfer_a = fetch_via_node(
        directory.path(),
        "node-a-fetch-first",
        &config_a,
        &content_a,
    );
    node_a.wait_for_stderr_count("TRIPTORRENT_CHUNK_ACCEPTED", 6, TRANSFER_TIMEOUT);
    node_a.terminate();
    for provider in &mut providers_a {
        provider.terminate();
    }
    let output_a = data_a.join("content").join(&content_a).join("data");
    let partial = data_a
        .join("content")
        .join(&content_a)
        .join("data.triptorrent-part");
    let state = data_a
        .join("content")
        .join(&content_a)
        .join("data.triptorrent-state");
    assert!(partial.exists());
    assert!(state.exists());

    thread::sleep(Duration::from_millis(1_100));
    let mut resumed_providers = vec![
        start_provider(
            directory.path(),
            "resumed-provider-a",
            bootstrap_address,
            &source_a,
            None,
        ),
        start_provider(
            directory.path(),
            "resumed-provider-b",
            bootstrap_address,
            &source_a,
            None,
        ),
    ];
    let node_a = start_node(
        directory.path(),
        "node-a-download-resumed",
        &config_a,
        addresses[3],
    );
    let completed_a = wait_transfer(
        directory.path(),
        "node-a-resume-status",
        &config_a,
        transfer_a,
    );
    assert!(completed_a["resumed_chunks"].as_u64().unwrap_or(0) >= 6);
    assert_eq!(
        fs::read(&output_a).expect("read resumed output"),
        expected_a
    );
    for provider in &mut resumed_providers {
        provider.terminate();
    }
    stop_node(directory.path(), "node-a-post-download", &config_a, node_a);

    thread::sleep(Duration::from_millis(1_100));
    let mut node_a = start_node(
        directory.path(),
        "node-a-serving-restarted",
        &config_a,
        addresses[3],
    );
    node_a.wait_for_stderr_count(PROVIDER_READY, 1, READY_TIMEOUT);

    let config_b = directory.path().join("node-b.toml");
    let data_b = directory.path().join("node-b-data");
    init_node(
        directory.path(),
        "node-b-init",
        &config_b,
        &data_b,
        addresses[4],
        Some(bootstrap_address),
    );
    let node_b = start_node(directory.path(), "node-b-running", &config_b, addresses[4]);

    let source_b = directory.path().join("source-b.bin");
    let expected_b = write_source(&source_b, 0x73, 18);
    let content_b = ContentId::digest(&expected_b).to_string();
    let mut provider_b = start_provider(
        directory.path(),
        "concurrent-provider-b",
        bootstrap_address,
        &source_b,
        None,
    );
    let node_b_fetch_a = fetch_via_node(directory.path(), "node-b-fetch-a", &config_b, &content_a);
    let node_a_fetch_b = fetch_via_node(directory.path(), "node-a-fetch-b", &config_a, &content_b);
    let completed_b = wait_transfer(
        directory.path(),
        "node-b-transfer-a",
        &config_b,
        node_b_fetch_a,
    );
    let completed_on_a = wait_transfer(
        directory.path(),
        "node-a-transfer-b",
        &config_a,
        node_a_fetch_b,
    );
    assert_eq!(completed_b["status"], "completed");
    assert_eq!(completed_on_a["status"], "completed");
    assert_eq!(
        fs::read(data_b.join("content").join(&content_a).join("data"))
            .expect("read Node B content A"),
        expected_a
    );
    assert_eq!(
        fs::read(data_a.join("content").join(&content_b).join("data"))
            .expect("read Node A content B"),
        expected_b
    );
    node_a.assert_running();
    provider_b.terminate();
    let receiver_logs = stop_node(directory.path(), "node-b-final", &config_b, node_b);
    let concurrent_logs = stop_node(directory.path(), "node-a-final", &config_a, node_a);
    assert!(
        format!("{}{}", receiver_logs.stdout, receiver_logs.stderr).contains("download_completed")
    );
    assert!(
        format!("{}{}", concurrent_logs.stdout, concurrent_logs.stderr)
            .contains("download_completed")
    );
    relay_b.assert_running();
    relay_a.assert_running();
    bootstrap.assert_running();
}
