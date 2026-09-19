//! Persistent M5 node runtime and loopback control API.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use triptorrent_core::{ContentId, Manifest};

const API_MAX_REQUEST: usize = 1024 * 1024;
const API_TIMEOUT: Duration = Duration::from_secs(5);
const SCHEMA_VERSION: i64 = 1;

/// Transfer result needed by the persistent node state machine.
#[derive(Clone, Copy, Debug, Default)]
pub struct PersistentFetchStats {
    /// Chunks reused from a previous partial transfer.
    pub resumed_chunks: usize,
}

/// Boundary between the persistent node and the current network implementation.
///
/// Front-ends provide this adapter so the node crate remains reusable by future
/// applications without depending on CLI command parsing.
pub trait TransferEngine: Send + Sync {
    /// Advertises and serves one stored content item until one transfer finishes
    /// or `keep_running` becomes false.
    ///
    /// # Errors
    ///
    /// Returns an error when discovery, relay setup, or transfer fails.
    fn serve_once(
        &self,
        bootstrap: SocketAddr,
        path: &Path,
        upload_limit: u64,
        keep_running: &AtomicBool,
    ) -> Result<()>;

    /// Fetches content into the managed output path.
    ///
    /// # Errors
    ///
    /// Returns an error when discovery, transfer, or integrity verification fails.
    fn fetch(
        &self,
        bootstrap: SocketAddr,
        content_id: ContentId,
        output: &Path,
        download_limit: u64,
    ) -> Result<PersistentFetchStats>;
}

/// Persistent daemon configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct NodeConfig {
    /// Managed state and content root.
    pub data_dir: PathBuf,
    /// Loopback HTTP API endpoint.
    pub api_listen: SocketAddr,
    /// Temporary M2 bootstrap used by the M5 runtime.
    pub bootstrap: Option<SocketAddr>,
    /// Aggregate receiver limit in bytes per second; zero is unlimited.
    pub download_limit: u64,
    /// Provider limit in bytes per second; zero is unlimited.
    pub upload_limit: u64,
    /// Maximum simultaneous daemon downloads.
    pub max_concurrent_transfers: usize,
    /// `tracing` filter directive.
    pub log_level: String,
    /// Bearer token required for mutating API calls.
    pub api_token: String,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("triptorrent-data"),
            api_listen: SocketAddr::from(([127, 0, 0, 1], 7331)),
            bootstrap: None,
            download_limit: 0,
            upload_limit: 0,
            max_concurrent_transfers: 4,
            log_level: "info".into(),
            api_token: String::new(),
        }
    }
}

/// Highest-precedence CLI configuration values.
#[derive(Clone, Debug, Default)]
pub struct ConfigOverrides {
    pub data_dir: Option<PathBuf>,
    pub api_listen: Option<SocketAddr>,
    pub bootstrap: Option<SocketAddr>,
    pub download_limit: Option<u64>,
    pub upload_limit: Option<u64>,
    pub max_concurrent_transfers: Option<usize>,
    pub log_level: Option<String>,
}

impl NodeConfig {
    /// Creates a safe local configuration with a fresh bearer token.
    ///
    /// # Errors
    ///
    /// Returns an error if secure random key generation fails.
    pub fn initialized(data_dir: PathBuf) -> Result<Self> {
        let mut token_bytes = [0_u8; 32];
        getrandom::fill(&mut token_bytes).context("failed to generate API token")?;
        let token = hex::encode(token_bytes);
        Ok(Self {
            data_dir,
            api_token: token,
            ..Self::default()
        })
    }

    /// Loads TOML, then applies environment and CLI overrides.
    ///
    /// # Errors
    ///
    /// Returns an error for missing, malformed, unsafe, or incomplete configuration.
    pub fn load(path: &Path, overrides: &ConfigOverrides) -> Result<Self> {
        let encoded = fs::read_to_string(path)
            .with_context(|| format!("failed to read node config {}", path.display()))?;
        let config: Self = toml::from_str(&encoded)
            .with_context(|| format!("invalid node config {}", path.display()))?;
        Self::merge(config, &environment_values(), overrides)
    }

    fn merge(
        mut config: Self,
        environment: &HashMap<String, String>,
        overrides: &ConfigOverrides,
    ) -> Result<Self> {
        if let Some(value) = environment.get("TRIPTORRENT_DATA_DIR") {
            config.data_dir = PathBuf::from(value);
        }
        if let Some(value) = environment.get("TRIPTORRENT_API_LISTEN") {
            config.api_listen = value.parse().context("invalid TRIPTORRENT_API_LISTEN")?;
        }
        if let Some(value) = environment.get("TRIPTORRENT_BOOTSTRAP") {
            config.bootstrap = Some(value.parse().context("invalid TRIPTORRENT_BOOTSTRAP")?);
        }
        if let Some(value) = environment.get("TRIPTORRENT_DOWNLOAD_LIMIT") {
            config.download_limit = value
                .parse()
                .context("invalid TRIPTORRENT_DOWNLOAD_LIMIT")?;
        }
        if let Some(value) = environment.get("TRIPTORRENT_UPLOAD_LIMIT") {
            config.upload_limit = value.parse().context("invalid TRIPTORRENT_UPLOAD_LIMIT")?;
        }
        if let Some(value) = environment.get("TRIPTORRENT_MAX_CONCURRENT_TRANSFERS") {
            config.max_concurrent_transfers = value
                .parse()
                .context("invalid TRIPTORRENT_MAX_CONCURRENT_TRANSFERS")?;
        }
        if let Some(value) = environment.get("TRIPTORRENT_LOG") {
            config.log_level.clone_from(value);
        }
        if let Some(value) = &overrides.data_dir {
            config.data_dir.clone_from(value);
        }
        if let Some(value) = overrides.api_listen {
            config.api_listen = value;
        }
        if let Some(value) = overrides.bootstrap {
            config.bootstrap = Some(value);
        }
        if let Some(value) = overrides.download_limit {
            config.download_limit = value;
        }
        if let Some(value) = overrides.upload_limit {
            config.upload_limit = value;
        }
        if let Some(value) = overrides.max_concurrent_transfers {
            config.max_concurrent_transfers = value;
        }
        if let Some(value) = &overrides.log_level {
            config.log_level.clone_from(value);
        }
        config.validate()?;
        Ok(config)
    }

    /// Writes a new TOML file and restricts permissions on Unix where practical.
    ///
    /// # Errors
    ///
    /// Returns an error if the file already exists or cannot be written.
    pub fn create(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if path.exists() {
            bail!("config already exists: {}", path.display());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let encoded = toml::to_string_pretty(self)?;
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        file.write_all(encoded.as_bytes())?;
        file.flush()?;
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if !self.api_listen.ip().is_loopback() {
            bail!("local API address must be loopback");
        }
        if self.api_token.len() < 32 {
            bail!("api_token must contain at least 32 characters");
        }
        if self.max_concurrent_transfers == 0 {
            bail!("max_concurrent_transfers must be at least one");
        }
        Ok(())
    }
}

fn environment_values() -> HashMap<String, String> {
    [
        "TRIPTORRENT_DATA_DIR",
        "TRIPTORRENT_API_LISTEN",
        "TRIPTORRENT_BOOTSTRAP",
        "TRIPTORRENT_DOWNLOAD_LIMIT",
        "TRIPTORRENT_UPLOAD_LIMIT",
        "TRIPTORRENT_MAX_CONCURRENT_TRANSFERS",
        "TRIPTORRENT_LOG",
    ]
    .into_iter()
    .filter_map(|name| std::env::var(name).ok().map(|value| (name.into(), value)))
    .collect()
}

/// Persistent content metadata returned through the local API.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContentRecord {
    pub content_id: String,
    pub length: u64,
    pub chunk_count: usize,
    pub data_path: PathBuf,
    pub shared: bool,
    pub complete: bool,
    pub integrity: String,
}

/// Persistent transfer metadata returned through the local API.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TransferRecord {
    pub id: i64,
    pub content_id: String,
    pub direction: String,
    pub status: String,
    pub bytes_total: u64,
    pub bytes_transferred: u64,
    pub resumed_chunks: usize,
    pub error: Option<String>,
    pub share_on_complete: bool,
}

struct Store {
    root: PathBuf,
    connection: Mutex<Connection>,
}

impl Store {
    fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root.join("content"))?;
        fs::create_dir_all(root.join("state"))?;
        let database = root.join("state").join("node.sqlite3");
        let connection = Connection::open(&database)
            .with_context(|| format!("failed to open SQLite index {}", database.display()))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS schema_version(version INTEGER NOT NULL);
             INSERT INTO schema_version(version)
               SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM schema_version);
             CREATE TABLE IF NOT EXISTS content(
               content_id TEXT PRIMARY KEY,
               length INTEGER NOT NULL,
               chunk_count INTEGER NOT NULL,
               data_path TEXT NOT NULL,
               manifest_path TEXT NOT NULL,
               shared INTEGER NOT NULL,
               complete INTEGER NOT NULL,
               integrity TEXT NOT NULL,
               created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS transfers(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               content_id TEXT NOT NULL,
               direction TEXT NOT NULL,
               status TEXT NOT NULL,
               bytes_total INTEGER NOT NULL DEFAULT 0,
               bytes_transferred INTEGER NOT NULL DEFAULT 0,
               resumed_chunks INTEGER NOT NULL DEFAULT 0,
               error TEXT,
               share_on_complete INTEGER NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );",
        )?;
        let version: i64 =
            connection.query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })?;
        if version != SCHEMA_VERSION {
            bail!("unsupported node index schema version {version}");
        }
        let store = Self {
            root: root.to_owned(),
            connection: Mutex::new(connection),
        };
        store.verify_all()?;
        Ok(store)
    }

    fn add_content(&self, source: &Path, shared: bool) -> Result<ContentRecord> {
        let bytes = fs::read(source)
            .with_context(|| format!("failed to read imported file {}", source.display()))?;
        let (manifest, _) = Manifest::from_bytes(&bytes)?;
        let content_id = manifest.content_id.to_string();
        if self.content_exists(&content_id)? {
            bail!("content {content_id} is already indexed");
        }
        let directory = self.content_directory(&content_id);
        fs::create_dir_all(&directory)?;
        let data_path = directory.join("data");
        let temporary = directory.join("data.importing");
        fs::write(&temporary, &bytes)?;
        if ContentId::digest(&fs::read(&temporary)?) != manifest.content_id {
            bail!("managed copy failed content-ID verification");
        }
        replace_file(&temporary, &data_path)?;
        let manifest_path = directory.join("manifest.postcard");
        fs::write(&manifest_path, postcard::to_allocvec(&manifest)?)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
        connection.execute(
            "INSERT INTO content(content_id,length,chunk_count,data_path,manifest_path,shared,complete,integrity,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,1,'ok',?7)",
            params![
                content_id,
                i64::try_from(manifest.length)?,
                i64::try_from(manifest.chunks.len())?,
                relative_string(&self.root, &data_path)?,
                relative_string(&self.root, &manifest_path)?,
                i64::from(shared),
                unix_time(),
            ],
        )?;
        drop(connection);
        self.get_content(&content_id)?
            .context("new content was not indexed")
    }

    fn complete_download(&self, content_id: ContentId, shared: bool) -> Result<ContentRecord> {
        let id = content_id.to_string();
        let data_path = self.content_directory(&id).join("data");
        let bytes = fs::read(&data_path)?;
        let (manifest, _) = Manifest::from_bytes(&bytes)?;
        if manifest.content_id != content_id {
            bail!("completed download does not match requested content ID");
        }
        let manifest_path = self.content_directory(&id).join("manifest.postcard");
        fs::write(&manifest_path, postcard::to_allocvec(&manifest)?)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
        connection.execute(
            "INSERT INTO content(content_id,length,chunk_count,data_path,manifest_path,shared,complete,integrity,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,1,'ok',?7)
             ON CONFLICT(content_id) DO UPDATE SET length=excluded.length,chunk_count=excluded.chunk_count,
             data_path=excluded.data_path,manifest_path=excluded.manifest_path,shared=excluded.shared,
             complete=1,integrity='ok'",
            params![
                id,
                i64::try_from(manifest.length)?,
                i64::try_from(manifest.chunks.len())?,
                relative_string(&self.root, &data_path)?,
                relative_string(&self.root, &manifest_path)?,
                i64::from(shared),
                unix_time(),
            ],
        )?;
        drop(connection);
        self.get_content(&id)?
            .context("completed content was not indexed")
    }

    fn list_content(&self) -> Result<Vec<ContentRecord>> {
        self.verify_all()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
        let mut statement = connection.prepare(
            "SELECT content_id,length,chunk_count,data_path,shared,complete,integrity FROM content ORDER BY content_id",
        )?;
        let records = statement
            .query_map([], |row| content_from_row(row, &self.root))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(records)
    }

    fn get_content(&self, content_id: &str) -> Result<Option<ContentRecord>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
        connection
            .query_row(
                "SELECT content_id,length,chunk_count,data_path,shared,complete,integrity FROM content WHERE content_id=?1",
                [content_id],
                |row| content_from_row(row, &self.root),
            )
            .optional()
            .map_err(Into::into)
    }

    fn remove_content(&self, content_id: &str, delete_bytes: bool) -> Result<()> {
        if self.get_content(content_id)?.is_none() {
            bail!("unknown content ID {content_id}");
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
        connection.execute("DELETE FROM content WHERE content_id=?1", [content_id])?;
        drop(connection);
        if delete_bytes {
            let directory = self.content_directory(content_id);
            if directory.exists() {
                fs::remove_dir_all(directory)?;
            }
        }
        Ok(())
    }

    fn content_exists(&self, content_id: &str) -> Result<bool> {
        Ok(self.get_content(content_id)?.is_some())
    }

    fn verify_all(&self) -> Result<()> {
        let rows: Vec<(String, String, String)> = {
            let connection = self
                .connection
                .lock()
                .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
            let mut statement = connection.prepare(
                "SELECT content_id,data_path,manifest_path FROM content WHERE complete=1",
            )?;
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        for (content_id, data_relative, manifest_relative) in rows {
            let data_path = managed_path(&self.root, &data_relative)
                .with_context(|| format!("invalid data path indexed for {content_id}"))?;
            let manifest_path = managed_path(&self.root, &manifest_relative)
                .with_context(|| format!("invalid manifest path indexed for {content_id}"))?;
            let integrity = verify_stored_content(&data_path, &manifest_path, &content_id);
            let (state, shared) = match integrity {
                Ok(()) => ("ok", None),
                Err(IntegrityFailure::Missing) => {
                    warn!(
                        event = "storage_integrity_failed",
                        content_id,
                        state = "missing"
                    );
                    ("missing", Some(0_i64))
                }
                Err(IntegrityFailure::Corrupt) => {
                    warn!(
                        event = "storage_integrity_failed",
                        content_id,
                        state = "corrupt"
                    );
                    ("corrupt", Some(0_i64))
                }
            };
            let connection = self
                .connection
                .lock()
                .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
            if let Some(shared) = shared {
                connection.execute(
                    "UPDATE content SET integrity=?1,shared=?2 WHERE content_id=?3",
                    params![state, shared, content_id],
                )?;
            } else {
                connection.execute(
                    "UPDATE content SET integrity=?1 WHERE content_id=?2",
                    params![state, content_id],
                )?;
            }
        }
        Ok(())
    }

    fn counts(&self) -> Result<(usize, usize)> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
        let stored: i64 =
            connection.query_row("SELECT COUNT(*) FROM content", [], |row| row.get(0))?;
        let shared: i64 = connection.query_row(
            "SELECT COUNT(*) FROM content WHERE shared=1 AND integrity='ok'",
            [],
            |row| row.get(0),
        )?;
        Ok((usize::try_from(stored)?, usize::try_from(shared)?))
    }

    fn create_transfer(&self, content_id: &str, share: bool) -> Result<i64> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
        let active: i64 = connection.query_row(
            "SELECT COUNT(*) FROM transfers WHERE content_id=?1 AND status IN ('running','interrupted')",
            [content_id],
            |row| row.get(0),
        )?;
        if active != 0 {
            bail!("a resumable transfer already exists for {content_id}");
        }
        let now = unix_time();
        connection.execute(
            "INSERT INTO transfers(content_id,direction,status,share_on_complete,created_at,updated_at)
             VALUES(?1,'download','running',?2,?3,?3)",
            params![content_id, i64::from(share), now],
        )?;
        Ok(connection.last_insert_rowid())
    }

    fn update_transfer(
        &self,
        id: i64,
        status: &str,
        bytes_total: u64,
        bytes_transferred: u64,
        resumed_chunks: usize,
        error: Option<&str>,
    ) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
        connection.execute(
            "UPDATE transfers SET status=?1,bytes_total=?2,bytes_transferred=?3,resumed_chunks=?4,error=?5,updated_at=?6 WHERE id=?7",
            params![
                status,
                i64::try_from(bytes_total)?,
                i64::try_from(bytes_transferred)?,
                i64::try_from(resumed_chunks)?,
                error,
                unix_time(),
                id,
            ],
        )?;
        Ok(())
    }

    fn list_transfers(&self) -> Result<Vec<TransferRecord>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
        let mut statement = connection.prepare(
            "SELECT id,content_id,direction,status,bytes_total,bytes_transferred,resumed_chunks,error,share_on_complete
             FROM transfers ORDER BY id",
        )?;
        Ok(statement
            .query_map([], transfer_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    fn get_transfer(&self, id: i64) -> Result<Option<TransferRecord>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
        connection
            .query_row(
                "SELECT id,content_id,direction,status,bytes_total,bytes_transferred,resumed_chunks,error,share_on_complete
                 FROM transfers WHERE id=?1",
                [id],
                transfer_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn recoverable_transfers(&self) -> Result<Vec<TransferRecord>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
        connection.execute(
            "UPDATE transfers SET status='interrupted',updated_at=?1 WHERE status='running'",
            [unix_time()],
        )?;
        let mut statement = connection.prepare(
            "SELECT id,content_id,direction,status,bytes_total,bytes_transferred,resumed_chunks,error,share_on_complete
             FROM transfers WHERE status='interrupted' ORDER BY id",
        )?;
        Ok(statement
            .query_map([], transfer_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    fn mark_running_interrupted(&self) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("index lock poisoned"))?;
        connection.execute(
            "UPDATE transfers SET status='interrupted',updated_at=?1 WHERE status='running'",
            [unix_time()],
        )?;
        Ok(())
    }

    fn content_directory(&self, content_id: &str) -> PathBuf {
        self.root.join("content").join(content_id)
    }
}

fn content_from_row(row: &rusqlite::Row<'_>, root: &Path) -> rusqlite::Result<ContentRecord> {
    let length: i64 = row.get(1)?;
    let chunks: i64 = row.get(2)?;
    let relative: String = row.get(3)?;
    let data_path = managed_path(root, &relative).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "indexed content path escapes managed storage",
            )
            .into(),
        )
    })?;
    Ok(ContentRecord {
        content_id: row.get(0)?,
        length: u64::try_from(length).unwrap_or(0),
        chunk_count: usize::try_from(chunks).unwrap_or(0),
        data_path,
        shared: row.get::<_, i64>(4)? != 0,
        complete: row.get::<_, i64>(5)? != 0,
        integrity: row.get(6)?,
    })
}

fn managed_path(root: &Path, relative: &str) -> Option<PathBuf> {
    let relative = Path::new(relative);
    if relative
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        Some(root.join(relative))
    } else {
        None
    }
}

fn transfer_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TransferRecord> {
    Ok(TransferRecord {
        id: row.get(0)?,
        content_id: row.get(1)?,
        direction: row.get(2)?,
        status: row.get(3)?,
        bytes_total: u64::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
        bytes_transferred: u64::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
        resumed_chunks: usize::try_from(row.get::<_, i64>(6)?).unwrap_or(0),
        error: row.get(7)?,
        share_on_complete: row.get::<_, i64>(8)? != 0,
    })
}

enum IntegrityFailure {
    Missing,
    Corrupt,
}

fn verify_stored_content(
    data_path: &Path,
    manifest_path: &Path,
    content_id: &str,
) -> std::result::Result<(), IntegrityFailure> {
    let bytes = fs::read(data_path).map_err(|_| IntegrityFailure::Missing)?;
    let encoded = fs::read(manifest_path).map_err(|_| IntegrityFailure::Missing)?;
    let manifest: Manifest =
        postcard::from_bytes(&encoded).map_err(|_| IntegrityFailure::Corrupt)?;
    if manifest.content_id.to_string() != content_id
        || manifest
            .reconstruct(
                &Manifest::from_bytes(&bytes)
                    .map_err(|_| IntegrityFailure::Corrupt)?
                    .1,
            )
            .is_err()
        || ContentId::digest(&bytes) != manifest.content_id
    {
        return Err(IntegrityFailure::Corrupt);
    }
    Ok(())
}

fn relative_string(root: &Path, path: &Path) -> Result<String> {
    Ok(path
        .strip_prefix(root)
        .context("managed path escaped the data directory")?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(_) if destination.exists() => {
            fs::remove_file(destination)?;
            fs::rename(source, destination)
        }
        Err(error) => Err(error),
    }
}

fn unix_time() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(i64::MAX)
}

struct NodeRuntime {
    config: NodeConfig,
    store: Arc<Store>,
    engine: Arc<dyn TransferEngine>,
    started: Instant,
    shutdown: AtomicBool,
    providers: Mutex<HashMap<String, Arc<AtomicBool>>>,
    active_downloads: AtomicUsize,
    active_uploads: AtomicUsize,
    bytes_downloaded: AtomicU64,
    bytes_uploaded: AtomicU64,
    completed_transfers: AtomicUsize,
    failed_transfers: AtomicUsize,
}

impl NodeRuntime {
    fn create(config: NodeConfig, engine: Arc<dyn TransferEngine>) -> Result<Arc<Self>> {
        let store = Arc::new(Store::open(&config.data_dir).inspect_err(|failure| {
            error!(event = "storage_open_failed", error = %failure);
        })?);
        let runtime = Arc::new(Self {
            config,
            store,
            engine,
            started: Instant::now(),
            shutdown: AtomicBool::new(false),
            providers: Mutex::new(HashMap::new()),
            active_downloads: AtomicUsize::new(0),
            active_uploads: AtomicUsize::new(0),
            bytes_downloaded: AtomicU64::new(0),
            bytes_uploaded: AtomicU64::new(0),
            completed_transfers: AtomicUsize::new(0),
            failed_transfers: AtomicUsize::new(0),
        });
        for content in runtime.store.list_content()? {
            if content.shared && content.integrity == "ok" {
                runtime.start_provider(content)?;
            }
        }
        for transfer in runtime.store.recoverable_transfers()? {
            if runtime.config.bootstrap.is_some() {
                info!(
                    event = "download_recovered",
                    transfer_id = transfer.id,
                    content_id = %transfer.content_id
                );
                runtime.spawn_download(transfer)?;
            }
        }
        Ok(runtime)
    }

    fn add_content(self: &Arc<Self>, path: &Path, shared: bool) -> Result<ContentRecord> {
        let content = self.store.add_content(path, shared)?;
        info!(event = "content_added", content_id = %content.content_id, length = content.length);
        if shared {
            self.start_provider(content.clone())?;
        }
        Ok(content)
    }

    fn remove_content(self: &Arc<Self>, content_id: &str, delete_bytes: bool) -> Result<()> {
        self.stop_provider(content_id);
        self.store.remove_content(content_id, delete_bytes)?;
        info!(event = "content_removed", content_id, delete_bytes);
        Ok(())
    }

    fn start_provider(self: &Arc<Self>, content: ContentRecord) -> Result<()> {
        let Some(bootstrap) = self.config.bootstrap else {
            return Ok(());
        };
        let mut providers = self
            .providers
            .lock()
            .map_err(|_| anyhow::anyhow!("provider lock poisoned"))?;
        if providers.contains_key(&content.content_id) {
            return Ok(());
        }
        let enabled = Arc::new(AtomicBool::new(true));
        providers.insert(content.content_id.clone(), Arc::clone(&enabled));
        drop(providers);
        let runtime = Arc::clone(self);
        thread::spawn(move || {
            while enabled.load(Ordering::Relaxed) && !runtime.shutdown.load(Ordering::Relaxed) {
                runtime.active_uploads.fetch_add(1, Ordering::Relaxed);
                let result = runtime.engine.serve_once(
                    bootstrap,
                    &content.data_path,
                    runtime.config.upload_limit,
                    &enabled,
                );
                runtime.active_uploads.fetch_sub(1, Ordering::Relaxed);
                match result {
                    Ok(()) => {
                        runtime
                            .bytes_uploaded
                            .fetch_add(content.length, Ordering::Relaxed);
                        runtime.completed_transfers.fetch_add(1, Ordering::Relaxed);
                        info!(event = "upload_completed", content_id = %content.content_id);
                    }
                    Err(_) if !enabled.load(Ordering::Relaxed) => break,
                    Err(error) => {
                        warn!(event = "upload_failed", content_id = %content.content_id, error = %error);
                        thread::sleep(Duration::from_millis(250));
                    }
                }
            }
        });
        Ok(())
    }

    fn stop_provider(&self, content_id: &str) {
        if let Ok(mut providers) = self.providers.lock()
            && let Some(enabled) = providers.remove(content_id)
        {
            enabled.store(false, Ordering::Relaxed);
        }
    }

    fn start_fetch(self: &Arc<Self>, content_id: ContentId, share: bool) -> Result<i64> {
        if self.config.bootstrap.is_none() {
            bail!("node has no bootstrap configured");
        }
        if self.store.content_exists(&content_id.to_string())? {
            bail!("content is already stored");
        }
        if self.active_downloads.load(Ordering::Relaxed) >= self.config.max_concurrent_transfers {
            bail!("maximum concurrent transfers reached");
        }
        let id = self.store.create_transfer(&content_id.to_string(), share)?;
        let transfer = self
            .store
            .get_transfer(id)?
            .context("new transfer missing")?;
        self.spawn_download(transfer)?;
        Ok(id)
    }

    fn spawn_download(self: &Arc<Self>, transfer: TransferRecord) -> Result<()> {
        let bootstrap = self
            .config
            .bootstrap
            .context("node has no bootstrap configured")?;
        let content_id = ContentId::from_str(&transfer.content_id)?;
        let directory = self.store.content_directory(&transfer.content_id);
        fs::create_dir_all(&directory)?;
        let output = directory.join("data");
        self.store.update_transfer(
            transfer.id,
            "running",
            transfer.bytes_total,
            transfer.bytes_transferred,
            transfer.resumed_chunks,
            None,
        )?;
        self.active_downloads.fetch_add(1, Ordering::Relaxed);
        let runtime = Arc::clone(self);
        thread::spawn(move || {
            info!(event = "download_started", transfer_id = transfer.id, content_id = %transfer.content_id);
            let result = runtime.engine.fetch(
                bootstrap,
                content_id,
                &output,
                runtime.config.download_limit,
            );
            match result {
                Ok(stats) => match runtime
                    .store
                    .complete_download(content_id, transfer.share_on_complete)
                {
                    Ok(content) => {
                        let _ = runtime.store.update_transfer(
                            transfer.id,
                            "completed",
                            content.length,
                            content.length,
                            stats.resumed_chunks,
                            None,
                        );
                        runtime
                            .bytes_downloaded
                            .fetch_add(content.length, Ordering::Relaxed);
                        runtime.completed_transfers.fetch_add(1, Ordering::Relaxed);
                        if content.shared {
                            let _ = runtime.start_provider(content);
                        }
                        info!(
                            event = "download_completed",
                            transfer_id = transfer.id,
                            resumed_chunks = stats.resumed_chunks
                        );
                    }
                    Err(error) => runtime.fail_transfer(transfer.id, &error),
                },
                Err(error) => runtime.fail_transfer(transfer.id, &error),
            }
            runtime.active_downloads.fetch_sub(1, Ordering::Relaxed);
        });
        Ok(())
    }

    fn fail_transfer(&self, id: i64, failure: &anyhow::Error) {
        let message = failure.to_string();
        let _ = self
            .store
            .update_transfer(id, "failed", 0, 0, 0, Some(&message));
        self.failed_transfers.fetch_add(1, Ordering::Relaxed);
        error!(event = "download_failed", transfer_id = id, error = %failure);
    }

    fn status(&self) -> Result<Value> {
        let (stored, shared) = self.store.counts()?;
        Ok(json!({
            "uptime_ms": u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "store_initialized": true,
            "api_ready": true,
            "stored_content": stored,
            "shared_content": shared,
            "active_downloads": self.active_downloads.load(Ordering::Relaxed),
            "active_uploads": self.active_uploads.load(Ordering::Relaxed),
            "bytes_downloaded": self.bytes_downloaded.load(Ordering::Relaxed),
            "bytes_uploaded": self.bytes_uploaded.load(Ordering::Relaxed),
            "completed_transfers": self.completed_transfers.load(Ordering::Relaxed),
            "failed_transfers": self.failed_transfers.load(Ordering::Relaxed),
        }))
    }

    fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Ok(providers) = self.providers.lock() {
            for enabled in providers.values() {
                enabled.store(false, Ordering::Relaxed);
            }
        }
        let _ = self.store.mark_running_interrupted();
    }
}

/// Runs the long-lived M5 node until the local API requests shutdown.
///
/// # Errors
///
/// Returns an error if configuration, storage, logging, or the API listener fails.
pub fn run_daemon(config: &NodeConfig, engine: Arc<dyn TransferEngine>) -> Result<()> {
    config.validate()?;
    let filter = EnvFilter::try_new(&config.log_level).context("invalid log_level directive")?;
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .try_init();
    let runtime = NodeRuntime::create(config.clone(), engine)?;
    let listener = TcpListener::bind(config.api_listen)
        .with_context(|| format!("failed to bind local API at {}", config.api_listen))?;
    listener.set_nonblocking(true)?;
    info!(event = "node_started", api = %config.api_listen, data_dir = %config.data_dir.display());
    while !runtime.shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, address)) => {
                let runtime = Arc::clone(&runtime);
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, address.ip(), &runtime) {
                        warn!(event = "api_error", error = %error);
                    }
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error.into()),
        }
    }
    runtime.shutdown();
    info!(event = "node_stopped");
    Ok(())
}

struct HttpRequest {
    method: String,
    target: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

fn handle_connection(
    mut stream: TcpStream,
    remote: IpAddr,
    runtime: &Arc<NodeRuntime>,
) -> Result<()> {
    if !remote.is_loopback() {
        write_response(
            &mut stream,
            403,
            &error_json("forbidden", "loopback clients only"),
        )?;
        return Ok(());
    }
    stream.set_read_timeout(Some(API_TIMEOUT))?;
    stream.set_write_timeout(Some(API_TIMEOUT))?;
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err(error) => {
            write_response(
                &mut stream,
                400,
                &error_json("malformed_request", &error.to_string()),
            )?;
            return Ok(());
        }
    };
    info!(
        event = "api_request",
        method = %request.method,
        target = %request.target,
        remote = %remote
    );
    let mutating = matches!(request.method.as_str(), "POST" | "DELETE");
    if mutating
        && request.authorization.as_deref() != Some(&format!("Bearer {}", runtime.config.api_token))
    {
        write_response(
            &mut stream,
            401,
            &error_json("unauthorized", "valid bearer token required"),
        )?;
        return Ok(());
    }
    let (status, body) = route_request(runtime, &request);
    write_response(&mut stream, status, &body)?;
    Ok(())
}

fn route_request(runtime: &Arc<NodeRuntime>, request: &HttpRequest) -> (u16, Value) {
    match route_request_inner(runtime, request) {
        Ok(result) => result,
        Err(error) => (400, error_json("invalid_operation", &error.to_string())),
    }
}

fn route_request_inner(runtime: &Arc<NodeRuntime>, request: &HttpRequest) -> Result<(u16, Value)> {
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((&request.target, ""));
    match (request.method.as_str(), path) {
        ("GET", "/v1/health") => Ok((
            200,
            json!({"alive": true, "store_initialized": true, "api_ready": true}),
        )),
        ("GET", "/v1/status") => Ok((200, runtime.status()?)),
        ("GET", "/v1/node") => Ok((
            200,
            json!({
                "api_listen": runtime.config.api_listen,
                "data_dir": runtime.config.data_dir,
                "bootstrap": runtime.config.bootstrap,
                "protocol_peer_identity": "ephemeral-per-advertisement"
            }),
        )),
        ("GET", "/v1/content") => Ok((200, serde_json::to_value(runtime.store.list_content()?)?)),
        ("POST", "/v1/content") => {
            #[derive(Deserialize)]
            struct Add {
                path: PathBuf,
                #[serde(default = "default_true")]
                shared: bool,
            }
            let body: Add =
                serde_json::from_slice(&request.body).context("invalid content-add JSON")?;
            let record = runtime.add_content(&body.path, body.shared)?;
            Ok((201, serde_json::to_value(record)?))
        }
        ("POST", "/v1/fetches") => {
            #[derive(Deserialize)]
            struct Fetch {
                content_id: String,
                #[serde(default = "default_true")]
                shared: bool,
            }
            let body: Fetch =
                serde_json::from_slice(&request.body).context("invalid fetch JSON")?;
            let content_id = ContentId::from_str(&body.content_id)?;
            let id = runtime.start_fetch(content_id, body.shared)?;
            Ok((202, json!({"transfer_id": id, "status": "running"})))
        }
        ("GET", "/v1/transfers") => {
            Ok((200, serde_json::to_value(runtime.store.list_transfers()?)?))
        }
        ("GET", "/v1/diagnostics") => Ok((
            200,
            json!({
                "bootstrap": runtime.config.bootstrap,
                "provider_workers": runtime.providers.lock().map_or(0, |p| p.len()),
                "known_relays": Value::Null,
            }),
        )),
        ("POST", "/v1/shutdown") => {
            runtime.shutdown.store(true, Ordering::Relaxed);
            Ok((202, json!({"status": "stopping"})))
        }
        ("DELETE", _) if path.starts_with("/v1/content/") => {
            let content_id = path.trim_start_matches("/v1/content/");
            let delete_bytes = query.split('&').any(|item| item == "delete_bytes=true");
            runtime.remove_content(content_id, delete_bytes)?;
            Ok((
                200,
                json!({"removed": content_id, "deleted_managed_bytes": delete_bytes}),
            ))
        }
        ("GET", _) if path.starts_with("/v1/transfers/") => {
            let id: i64 = path
                .trim_start_matches("/v1/transfers/")
                .parse()
                .context("invalid transfer ID")?;
            match runtime.store.get_transfer(id)? {
                Some(record) => Ok((200, serde_json::to_value(record)?)),
                None => Ok((404, error_json("not_found", "unknown transfer"))),
            }
        }
        _ => Ok((404, error_json("not_found", "unknown API route"))),
    }
}

const fn default_true() -> bool {
    true
}

fn error_json(code: &str, message: &str) -> Value {
    json!({"error": {"code": code, "message": message}})
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            bail!("client closed before sending headers");
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > API_MAX_REQUEST {
            bail!("API request is too large");
        }
        if let Some(index) = find_subslice(&bytes, b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers =
        std::str::from_utf8(&bytes[..header_end]).context("HTTP headers are not UTF-8")?;
    let mut lines = headers.split("\r\n");
    let request_line = lines.next().context("missing HTTP request line")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().context("missing HTTP method")?.to_owned();
    let target = parts.next().context("missing HTTP target")?.to_owned();
    let mut content_length = 0_usize;
    let mut authorization = None;
    for line in lines.filter(|line| !line.is_empty()) {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().context("invalid Content-Length")?;
            } else if name.eq_ignore_ascii_case("authorization") {
                authorization = Some(value.trim().to_owned());
            }
        }
    }
    if header_end.saturating_add(content_length) > API_MAX_REQUEST {
        bail!("API request is too large");
    }
    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            bail!("client closed before sending request body");
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(HttpRequest {
        method,
        target,
        authorization,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

fn write_response(stream: &mut TcpStream, status: u16, body: &Value) -> Result<()> {
    let encoded = serde_json::to_vec(&body)?;
    let reason = match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        encoded.len()
    )?;
    stream.write_all(&encoded)?;
    stream.flush()?;
    Ok(())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Sends one request to a configured local node and returns its JSON body.
///
/// # Errors
///
/// Returns an error for transport, malformed responses, or non-success status codes.
pub fn api_request(
    config: &NodeConfig,
    method: &str,
    target: &str,
    body: Option<Value>,
) -> Result<Value> {
    let mut stream = TcpStream::connect_timeout(&config.api_listen, API_TIMEOUT)
        .with_context(|| format!("failed to connect to local API at {}", config.api_listen))?;
    stream.set_read_timeout(Some(API_TIMEOUT))?;
    stream.set_write_timeout(Some(API_TIMEOUT))?;
    let encoded = body.map_or_else(Vec::new, |value| {
        serde_json::to_vec(&value).unwrap_or_default()
    });
    write!(
        stream,
        "{method} {target} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        config.api_listen,
        config.api_token,
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
        bail!("local API returned HTTP {status}: {value}");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn config_precedence_and_loopback_validation() {
        let mut environment = HashMap::new();
        environment.insert("TRIPTORRENT_DOWNLOAD_LIMIT".into(), "200".into());
        environment.insert("TRIPTORRENT_API_LISTEN".into(), "127.0.0.1:7444".into());
        let base = NodeConfig {
            api_token: "a".repeat(64),
            download_limit: 100,
            ..NodeConfig::default()
        };
        let merged = NodeConfig::merge(
            base,
            &environment,
            &ConfigOverrides {
                download_limit: Some(300),
                ..ConfigOverrides::default()
            },
        )
        .unwrap();
        assert_eq!(merged.download_limit, 300);
        assert_eq!(merged.api_listen.port(), 7444);
        let unsafe_config = NodeConfig {
            api_listen: "0.0.0.0:7331".parse().unwrap(),
            ..merged
        };
        assert!(unsafe_config.validate().is_err());
    }

    #[test]
    fn store_detects_modified_and_missing_content() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source.bin");
        fs::write(&source, b"persistent bytes").unwrap();
        let store = Store::open(&directory.path().join("data")).unwrap();
        let first = store.add_content(&source, true).unwrap();
        fs::write(&first.data_path, b"modified").unwrap();
        let records = store.list_content().unwrap();
        assert_eq!(records[0].integrity, "corrupt");
        assert!(!records[0].shared);

        let source_two = directory.path().join("source-two.bin");
        fs::write(&source_two, b"second item").unwrap();
        let second = store.add_content(&source_two, true).unwrap();
        fs::remove_file(&second.data_path).unwrap();
        let records = store.list_content().unwrap();
        assert!(records.iter().any(|record| record.integrity == "missing"));
    }

    #[test]
    fn removing_metadata_preserves_bytes_unless_explicit() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source.bin");
        fs::write(&source, b"managed bytes").unwrap();
        let store = Store::open(&directory.path().join("data")).unwrap();
        let first = store.add_content(&source, false).unwrap();
        store.remove_content(&first.content_id, false).unwrap();
        assert!(first.data_path.exists());

        let second = store.add_content(&source, false).unwrap();
        store.remove_content(&second.content_id, true).unwrap();
        assert!(!second.data_path.exists());
        assert!(source.exists());
    }

    #[test]
    fn corrupt_sqlite_index_fails_closed() {
        let directory = tempdir().unwrap();
        let data = directory.path().join("data");
        drop(Store::open(&data).unwrap());
        fs::write(data.join("state").join("node.sqlite3"), b"not sqlite").unwrap();
        assert!(Store::open(&data).is_err());
    }

    #[test]
    fn indexed_path_cannot_escape_managed_storage() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source.bin");
        fs::write(&source, b"managed bytes").unwrap();
        let data = directory.path().join("data");
        let store = Store::open(&data).unwrap();
        store.add_content(&source, true).unwrap();
        {
            let connection = store.connection.lock().unwrap();
            connection
                .execute("UPDATE content SET data_path='../outside'", [])
                .unwrap();
        }
        drop(store);
        assert!(Store::open(&data).is_err());
    }
}
