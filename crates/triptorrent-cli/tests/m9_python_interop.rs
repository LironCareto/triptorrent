use std::fs::{self, File};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use triptorrent_core::ContentId;

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30);

struct ChildProcess {
    name: &'static str,
    child: Option<Child>,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl ChildProcess {
    fn spawn(name: &'static str, mut command: Command, directory: &Path) -> Self {
        let stdout = directory.join(format!("{name}.stdout"));
        let stderr = directory.join(format!("{name}.stderr"));
        command
            .stdin(Stdio::null())
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap());
        let child = command
            .spawn()
            .unwrap_or_else(|error| panic!("failed to start {name}: {error}"));
        Self {
            name,
            child: Some(child),
            stdout,
            stderr,
        }
    }

    fn running(&mut self) -> bool {
        self.child
            .as_mut()
            .is_some_and(|child| child.try_wait().unwrap().is_none())
    }

    fn wait_for_stderr(&mut self, marker: &str) {
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            assert!(
                self.running(),
                "{} exited early\n{}",
                self.name,
                self.logs()
            );
            if fs::read_to_string(&self.stderr)
                .unwrap_or_default()
                .contains(marker)
            {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("{} did not become ready\n{}", self.name, self.logs());
    }

    fn wait_success(&mut self) {
        let deadline = Instant::now() + TRANSFER_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                self.child.take();
                assert!(
                    status.success(),
                    "{} failed: {status}\n{}",
                    self.name,
                    self.logs()
                );
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("{} timed out\n{}", self.name, self.logs());
    }

    fn logs(&self) -> String {
        format!(
            "stdout:\n{}\nstderr:\n{}",
            fs::read_to_string(&self.stdout).unwrap_or_default(),
            fs::read_to_string(&self.stderr).unwrap_or_default()
        )
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn rust_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_triptorrent"))
}

fn wait_listener(process: &mut ChildProcess, address: SocketAddr) {
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        assert!(
            process.running(),
            "{} exited early\n{}",
            process.name,
            process.logs()
        );
        if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "{} did not listen on {address}\n{}",
        process.name,
        process.logs()
    );
}

#[test]
fn python_receiver_fetches_verified_content_from_rust_provider() {
    let directory = tempdir().unwrap();
    let bootstrap_address = address();
    let relay_address = address();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = root.join("test-data/m9-canonical.txt");
    let expected = fs::read(&source).unwrap();
    let output = directory.path().join("python-output.bin");
    let content_id = ContentId::digest(&expected).to_string();

    let mut bootstrap_command = rust_command();
    bootstrap_command.args(["bootstrap", "--listen", &bootstrap_address.to_string()]);
    let mut bootstrap = ChildProcess::spawn("m9-bootstrap", bootstrap_command, directory.path());
    wait_listener(&mut bootstrap, bootstrap_address);

    let mut relay_command = rust_command();
    relay_command.args([
        "relay",
        "--listen",
        &relay_address.to_string(),
        "--bootstrap",
        &bootstrap_address.to_string(),
        "--id",
        "m9-relay",
    ]);
    let mut relay = ChildProcess::spawn("m9-relay", relay_command, directory.path());
    thread::sleep(Duration::from_millis(150));
    assert!(relay.running(), "relay exited early\n{}", relay.logs());

    let mut provider_command = rust_command();
    provider_command
        .arg("share")
        .arg("--bootstrap")
        .arg(bootstrap_address.to_string())
        .arg("--file")
        .arg(&source);
    let mut provider = ChildProcess::spawn("m9-provider", provider_command, directory.path());
    provider.wait_for_stderr("TRIPTORRENT_PROVIDER_READY");

    let script = root.join("interop/python/triptorrent_v1.py");
    let mut python_command = Command::new("python");
    python_command
        .arg(script)
        .arg("fetch")
        .arg("--bootstrap")
        .arg(bootstrap_address.to_string())
        .arg("--content")
        .arg(&content_id)
        .arg("--output")
        .arg(&output);
    let mut python = ChildProcess::spawn("m9-python", python_command, directory.path());
    python.wait_success();
    provider.wait_success();

    assert_eq!(fs::read(output).unwrap(), expected);
    assert!(python.logs().contains("PASS content="));
    assert!(bootstrap.running());
    assert!(relay.running());
}

#[test]
fn testnet_probe_validates_local_ephemeral_infrastructure() {
    let directory = tempdir().unwrap();
    let bootstrap_address = address();
    let relay_address = address();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = root.join("test-data/m9-canonical.txt");
    let expected = fs::read(&source).unwrap();
    let output = directory.path().join("probe-output.bin");

    let mut bootstrap_command = rust_command();
    bootstrap_command.args(["bootstrap", "--listen", &bootstrap_address.to_string()]);
    let mut bootstrap = ChildProcess::spawn("probe-bootstrap", bootstrap_command, directory.path());
    wait_listener(&mut bootstrap, bootstrap_address);

    let mut relay_command = rust_command();
    relay_command.args([
        "relay",
        "--listen",
        &relay_address.to_string(),
        "--bootstrap",
        &bootstrap_address.to_string(),
        "--id",
        "probe-relay",
    ]);
    let mut relay = ChildProcess::spawn("probe-relay", relay_command, directory.path());
    thread::sleep(Duration::from_millis(150));
    assert!(relay.running(), "relay exited early\n{}", relay.logs());

    let mut provider_command = rust_command();
    provider_command
        .arg("share")
        .arg("--bootstrap")
        .arg(bootstrap_address.to_string())
        .arg("--file")
        .arg(&source);
    let mut provider = ChildProcess::spawn("probe-provider", provider_command, directory.path());
    provider.wait_for_stderr("TRIPTORRENT_PROVIDER_READY");

    let mut probe_command = rust_command();
    probe_command
        .arg("testnet")
        .arg("probe")
        .arg("--bootstrap")
        .arg(bootstrap_address.to_string())
        .arg("--output")
        .arg(&output);
    let mut probe = ChildProcess::spawn("probe-client", probe_command, directory.path());
    probe.wait_success();
    provider.wait_success();
    assert_eq!(fs::read(output).unwrap(), expected);
    assert!(probe.logs().contains("\"status\": \"pass\""));
    assert!(bootstrap.running());
    assert!(relay.running());
}
