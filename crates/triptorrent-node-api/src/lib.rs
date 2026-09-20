//! Typed data transfer objects and client for the loopback node API.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const API_TIMEOUT: Duration = Duration::from_secs(5);

/// Connection information read from the persistent-node configuration.
#[derive(Clone, Deserialize)]
pub struct LocalApiConnection {
    /// Loopback HTTP endpoint.
    pub api_listen: SocketAddr,
    /// Secret used only in mutation request headers.
    pub api_token: String,
}

impl std::fmt::Debug for LocalApiConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalApiConnection")
            .field("api_listen", &self.api_listen)
            .field("api_token", &"[REDACTED]")
            .finish()
    }
}

impl LocalApiConnection {
    /// Reads only the local API fields from a node TOML file.
    ///
    /// # Errors
    ///
    /// Returns an error when the file is unavailable, malformed, non-loopback,
    /// or does not contain an adequate token.
    pub fn from_config(path: &Path) -> Result<Self> {
        let encoded = fs::read_to_string(path)
            .with_context(|| format!("failed to read node config {}", path.display()))?;
        let connection: Self = toml::from_str(&encoded)
            .with_context(|| format!("invalid node config {}", path.display()))?;
        if !connection.api_listen.ip().is_loopback() {
            bail!("local API address must be loopback");
        }
        if connection.api_token.len() < 32 {
            bail!("node API token is missing or invalid");
        }
        Ok(connection)
    }
}

/// Namespaced identity presented by a local content record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContentIdentity {
    /// Identifier namespace, currently `triptorrent` for stored content.
    pub namespace: String,
    /// Namespace-specific identifier value.
    pub value: String,
    /// Verification state such as `verified` or `unverified_metadata`.
    pub verification: String,
}

/// Persistent content metadata returned through the local API.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContentRecord {
    pub content_id: String,
    #[serde(default)]
    pub identities: Vec<ContentIdentity>,
    pub length: u64,
    pub chunk_count: usize,
    pub data_path: PathBuf,
    pub shared: bool,
    #[serde(default)]
    pub advertised: bool,
    pub complete: bool,
    pub integrity: String,
}

/// Per-provider final verified contribution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderContribution {
    pub provider_id: String,
    pub verified_chunks: usize,
    pub rejected_requests: usize,
}

/// Persistent transfer metadata returned through the local API.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TransferRecord {
    pub id: i64,
    pub content_id: String,
    pub direction: String,
    pub status: String,
    pub bytes_total: u64,
    /// Bytes already verified against the manifest.
    pub bytes_transferred: u64,
    #[serde(default)]
    pub verified_chunks: usize,
    #[serde(default)]
    pub total_chunks: usize,
    #[serde(default)]
    pub current_rate_bps: u64,
    #[serde(default)]
    pub provider_count: usize,
    #[serde(default)]
    pub retry_count: usize,
    #[serde(default)]
    pub rejected_chunks: usize,
    #[serde(default)]
    pub provider_contributions: Vec<ProviderContribution>,
    pub resumed_chunks: usize,
    pub error: Option<String>,
    pub share_on_complete: bool,
    #[serde(default = "default_network_path")]
    pub network_path: String,
}

fn default_network_path() -> String {
    "encrypted_relayed_swarm".into()
}

/// Liveness response from `/v1/health`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HealthStatus {
    pub alive: bool,
    pub store_initialized: bool,
    pub api_ready: bool,
}

/// Runtime counters from `/v1/status`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct NodeStatus {
    pub uptime_ms: u64,
    pub store_initialized: bool,
    pub api_ready: bool,
    pub stored_content: usize,
    pub shared_content: usize,
    pub active_downloads: usize,
    pub active_uploads: usize,
    pub bytes_downloaded: u64,
    pub bytes_uploaded: u64,
    pub completed_transfers: usize,
    pub failed_transfers: usize,
}

/// Non-secret configuration summary from `/v1/node`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NodeSummary {
    pub api_listen: SocketAddr,
    pub data_dir: PathBuf,
    pub bootstrap: Option<SocketAddr>,
    pub protocol_peer_identity: String,
}

/// Diagnostics safe to present in the desktop client.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Diagnostics {
    pub bootstrap: Option<SocketAddr>,
    pub bootstrap_configured: bool,
    pub provider_workers: usize,
    pub known_relays: Option<usize>,
    pub api_version: String,
    pub node_version: String,
}

/// Structured statement of the currently implemented network path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NetworkPrivacyStatus {
    pub network_mode: String,
    pub discovery: String,
    pub data_path: String,
    pub direct_peer_connection: bool,
    pub relay_visibility: Vec<String>,
    pub bootstrap_visibility: Vec<String>,
    pub compatibility: CompatibilityStatus,
    pub anonymity_guarantee: bool,
}

/// Implementation availability for network modes adjacent to the current path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompatibilityStatus {
    pub classic_bittorrent_active: bool,
    pub m3_private_discovery_implemented: bool,
}

impl NetworkPrivacyStatus {
    /// Truthful description of the currently implemented M2/M4 path.
    #[must_use]
    pub fn current_prototype() -> Self {
        Self {
            network_mode: "triptorrent_prototype".into(),
            discovery: "temporary_centralized_m2_bootstrap".into(),
            data_path: "end_to_end_encrypted_relayed_swarm".into(),
            direct_peer_connection: false,
            relay_visibility: vec![
                "peer_endpoints".into(),
                "route".into(),
                "timing".into(),
                "traffic_volume".into(),
            ],
            bootstrap_visibility: vec![
                "peer_ip_addresses".into(),
                "content_advertisements_and_queries".into(),
                "routes".into(),
                "timing".into(),
            ],
            compatibility: CompatibilityStatus {
                classic_bittorrent_active: false,
                m3_private_discovery_implemented: false,
            },
            anonymity_guarantee: false,
        }
    }
}

/// Typed synchronous client. Call it from a worker, never a GUI render thread.
#[derive(Clone, Debug)]
pub struct LocalApiClient {
    connection: LocalApiConnection,
    timeout: Duration,
}

impl LocalApiClient {
    /// Creates a client from already validated connection data.
    #[must_use]
    pub const fn new(connection: LocalApiConnection) -> Self {
        Self {
            connection,
            timeout: API_TIMEOUT,
        }
    }

    /// Reads connection data from the node TOML file.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid configuration.
    pub fn from_config(path: &Path) -> Result<Self> {
        Ok(Self::new(LocalApiConnection::from_config(path)?))
    }

    /// Returns the loopback endpoint without exposing the bearer token.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.connection.api_listen
    }

    /// Reads daemon liveness.
    ///
    /// # Errors
    /// Returns an error for connection, HTTP, or response-decoding failures.
    pub fn health(&self) -> Result<HealthStatus> {
        self.get("/v1/health")
    }

    /// Reads node counters.
    ///
    /// # Errors
    /// Returns an error for connection, HTTP, or response-decoding failures.
    pub fn status(&self) -> Result<NodeStatus> {
        self.get("/v1/status")
    }

    /// Reads the non-secret node summary.
    ///
    /// # Errors
    /// Returns an error for connection, HTTP, or response-decoding failures.
    pub fn node(&self) -> Result<NodeSummary> {
        self.get("/v1/node")
    }

    /// Lists persistent content.
    ///
    /// # Errors
    /// Returns an error for connection, HTTP, or response-decoding failures.
    pub fn content(&self) -> Result<Vec<ContentRecord>> {
        self.get("/v1/content")
    }

    /// Imports one local file through the authenticated API.
    ///
    /// # Errors
    /// Returns an error when the request, import, or response fails.
    pub fn add_content(&self, path: &Path, shared: bool) -> Result<ContentRecord> {
        self.send(
            "POST",
            "/v1/content",
            Some(json!({"path": path, "shared": shared})),
        )
    }

    /// Removes content metadata and optionally its managed copy.
    ///
    /// # Errors
    /// Returns an error when authentication, removal, or transport fails.
    pub fn remove_content(&self, content_id: &str, delete_bytes: bool) -> Result<Value> {
        self.send(
            "DELETE",
            &format!("/v1/content/{content_id}?delete_bytes={delete_bytes}"),
            None,
        )
    }

    /// Starts one persistent `TripTorrent` fetch.
    ///
    /// # Errors
    /// Returns an error for an invalid ID, unavailable network, or API failure.
    pub fn start_fetch(&self, content_id: &str, shared: bool) -> Result<i64> {
        #[derive(Deserialize)]
        struct Started {
            transfer_id: i64,
        }
        Ok(self
            .send::<Started>(
                "POST",
                "/v1/fetches",
                Some(json!({"content_id": content_id, "shared": shared})),
            )?
            .transfer_id)
    }

    /// Lists persistent transfers.
    ///
    /// # Errors
    /// Returns an error for connection, HTTP, or response-decoding failures.
    pub fn transfers(&self) -> Result<Vec<TransferRecord>> {
        self.get("/v1/transfers")
    }

    /// Reads one transfer.
    ///
    /// # Errors
    /// Returns an error when the transfer is unknown or the API request fails.
    pub fn transfer(&self, id: i64) -> Result<TransferRecord> {
        self.get(&format!("/v1/transfers/{id}"))
    }

    /// Requests a cooperative pause.
    ///
    /// # Errors
    /// Returns an error when the transfer cannot be paused or the API fails.
    pub fn pause_transfer(&self, id: i64) -> Result<Value> {
        self.send(
            "POST",
            &format!("/v1/transfers/{id}/pause"),
            Some(json!({})),
        )
    }

    /// Resumes an explicitly paused transfer.
    ///
    /// # Errors
    /// Returns an error when the transfer cannot resume or the API fails.
    pub fn resume_transfer(&self, id: i64) -> Result<Value> {
        self.send(
            "POST",
            &format!("/v1/transfers/{id}/resume"),
            Some(json!({})),
        )
    }

    /// Reads redacted operational diagnostics.
    ///
    /// # Errors
    /// Returns an error for connection, HTTP, or response-decoding failures.
    pub fn diagnostics(&self) -> Result<Diagnostics> {
        self.get("/v1/diagnostics")
    }

    /// Reads the structured network/privacy model.
    ///
    /// # Errors
    /// Returns an error for connection, HTTP, or response-decoding failures.
    pub fn network_privacy(&self) -> Result<NetworkPrivacyStatus> {
        self.get("/v1/network-privacy")
    }

    /// Requests a clean daemon shutdown.
    ///
    /// # Errors
    /// Returns an error for authentication, transport, or HTTP failures.
    pub fn shutdown(&self) -> Result<Value> {
        self.send("POST", "/v1/shutdown", Some(json!({})))
    }

    /// Raw JSON request retained for the command-line presentation layer.
    ///
    /// # Errors
    ///
    /// Returns an error for transport, HTTP, or JSON failures.
    pub fn request_value(&self, method: &str, target: &str, body: Option<Value>) -> Result<Value> {
        self.request(method, target, body)
    }

    fn get<T: DeserializeOwned>(&self, target: &str) -> Result<T> {
        self.send("GET", target, None)
    }

    fn send<T: DeserializeOwned>(
        &self,
        method: &str,
        target: &str,
        body: Option<Value>,
    ) -> Result<T> {
        serde_json::from_value(self.request(method, target, body)?)
            .context("local API returned an incompatible response")
    }

    fn request(&self, method: &str, target: &str, body: Option<Value>) -> Result<Value> {
        let mut stream = TcpStream::connect_timeout(&self.connection.api_listen, self.timeout)
            .with_context(|| {
                format!(
                    "failed to connect to local API at {}",
                    self.connection.api_listen
                )
            })?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        let encoded = body.map_or_else(Vec::new, |value| {
            serde_json::to_vec(&value).unwrap_or_default()
        });
        write!(
            stream,
            "{method} {target} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.connection.api_listen,
            self.connection.api_token,
            encoded.len()
        )?;
        stream.write_all(&encoded)?;
        stream.flush()?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        let split =
            find_subslice(&response, b"\r\n\r\n").context("local API returned malformed HTTP")?;
        let headers = std::str::from_utf8(&response[..split])?;
        let status: u16 = headers
            .split_whitespace()
            .nth(1)
            .context("local API omitted status")?
            .parse()?;
        let value: Value = serde_json::from_slice(&response[split + 4..])
            .context("local API returned invalid JSON")?;
        if !(200..300).contains(&status) {
            let message = value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("local API request failed");
            bail!("local API returned HTTP {status}: {message}");
        }
        Ok(value)
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
