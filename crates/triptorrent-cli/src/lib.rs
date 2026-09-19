#![doc = "Reusable M1 transfer and M2 overlay operations for the local demo."]

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use triptorrent_core::{CHUNK_SIZE, Chunk, ContentId, Manifest, PeerId};
use triptorrent_net::{EncryptedSession, RelayRole, TcpRelayTransport, generate_peer_keypair};
use triptorrent_overlay::BootstrapClient;
use triptorrent_protocol::{
    Message, PeerAdvertisement, PeerCapability, PieceAvailability, RouteAssignment,
};
use triptorrent_swarm::{RateSchedule, Scheduler};

const OVERLAY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const OVERLAY_SESSION_TIMEOUT: Duration = Duration::from_secs(5);
const FETCH_MAX_DISCOVERY_ATTEMPTS: usize = 600;
const MAX_SWARM_PROVIDERS: u16 = 8;
const WORKER_TIMEOUT: Duration = Duration::from_secs(10);
const RESUME_VERSION: u16 = 0;
const TEST_POLL_INTERVAL_ENV: &str = "TRIPTORRENT_TEST_OVERLAY_POLL_MS";
const TEST_IDLE_MARKER_ENV: &str = "TRIPTORRENT_TEST_IDLE_MARKER_AFTER";
const TEST_CORRUPT_CHUNKS_ENV: &str = "TRIPTORRENT_TEST_CORRUPT_CHUNKS";
const TEST_CHUNK_DELAY_ENV: &str = "TRIPTORRENT_TEST_CHUNK_DELAY_MS";

/// Computes the experimental content ID of a local file.
///
/// # Errors
///
/// Returns an error if the file cannot be read.
pub fn identify(path: &Path) -> Result<ContentId> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read source file {}", path.display()))?;
    Ok(ContentId::digest(&bytes))
}

/// Exposes one local file to a receiver through the relay, then exits.
///
/// # Errors
///
/// Returns an error for file, relay, session, request, or transfer failures.
pub fn share_file(
    relay: SocketAddr,
    route: &str,
    psk: &[u8; 32],
    path: &Path,
) -> Result<ContentId> {
    let (manifest, chunks) = load_content(path)?;
    let content_id = manifest.content_id;

    let transport = TcpRelayTransport::connect(relay, route, RelayRole::Sender)
        .context("failed to connect sender to relay")?;
    let mut session = EncryptedSession::initiator(transport, psk)
        .context("failed to establish encrypted peer session")?;

    send_content(&mut session, &manifest, &chunks)?;
    Ok(content_id)
}

fn send_content(
    session: &mut EncryptedSession<TcpRelayTransport>,
    manifest: &Manifest,
    chunks: &[Chunk],
) -> Result<()> {
    match session
        .receive()
        .context("failed to receive content request")?
    {
        Message::Request {
            content_id: requested,
        } if requested == manifest.content_id => {}
        Message::Request { .. } => {
            session.send(Message::Error(
                "content ID is not shared on this route".into(),
            ))?;
            bail!("receiver requested a different content ID");
        }
        _ => bail!("receiver did not begin with a content request"),
    }

    session.send(Message::Manifest(manifest.clone()))?;
    for chunk in chunks {
        session.send(Message::Chunk(chunk.clone()))?;
    }
    session.send(Message::Complete)?;
    Ok(())
}

/// Fetches, verifies, and atomically writes one file received through the relay.
///
/// # Errors
///
/// Returns an error for relay, session, protocol, integrity, or file failures.
pub fn fetch_file(
    relay: SocketAddr,
    route: &str,
    psk: &[u8; 32],
    content_id: ContentId,
    output: &Path,
) -> Result<()> {
    let transport = TcpRelayTransport::connect(relay, route, RelayRole::Receiver)
        .context("failed to connect receiver to relay")?;
    let mut session = EncryptedSession::responder(transport, psk)
        .context("failed to establish encrypted peer session")?;
    receive_content(&mut session, content_id, output)
}

fn receive_content(
    session: &mut EncryptedSession<TcpRelayTransport>,
    content_id: ContentId,
    output: &Path,
) -> Result<()> {
    session.send(Message::Request { content_id })?;

    let manifest = match session.receive()? {
        Message::Manifest(manifest) if manifest.content_id == content_id => manifest,
        Message::Manifest(_) => bail!("sender returned a different content ID"),
        Message::Error(message) => bail!("sender rejected request: {message}"),
        _ => bail!("sender did not return a manifest"),
    };
    if usize::try_from(manifest.chunk_size).ok() != Some(CHUNK_SIZE) {
        bail!("manifest uses an unsupported experimental chunk size");
    }

    let mut chunks = Vec::with_capacity(manifest.chunks.len());
    loop {
        match session.receive()? {
            Message::Chunk(chunk) => {
                manifest.verify_chunk(&chunk)?;
                chunks.push(chunk);
            }
            Message::Complete => break,
            Message::Error(message) => bail!("sender aborted transfer: {message}"),
            _ => bail!("unexpected message during chunk transfer"),
        }
    }
    let bytes = manifest.reconstruct(&chunks)?;

    let temporary = output.with_extension("triptorrent-part");
    fs::write(&temporary, bytes)
        .with_context(|| format!("failed to write temporary file {}", temporary.display()))?;
    fs::rename(&temporary, output)
        .with_context(|| format!("failed to finalize output file {}", output.display()))?;
    Ok(())
}

fn load_content(path: &Path) -> Result<(Manifest, Vec<Chunk>)> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read source file {}", path.display()))?;
    Ok(Manifest::from_bytes(&bytes)?)
}

/// Options for one M4 sharing process.
#[derive(Clone, Debug, Default)]
pub struct ShareOptions {
    /// Inclusive chunk indices/ranges such as `0-4,8`; `None` means every chunk.
    pub availability: Option<String>,
    /// Provider upload limit in bytes per second; `None` is unlimited.
    pub upload_limit: Option<u64>,
}

/// Options for one M4 receiver process.
#[derive(Clone, Copy, Debug, Default)]
pub struct FetchOptions {
    /// Aggregate request pacing in bytes per second; `None` is unlimited.
    pub download_limit: Option<u64>,
}

/// Diagnostic counters for a completed M4 transfer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferStats {
    /// Provider IDs and their accepted chunk counts.
    pub accepted_by_provider: Vec<(PeerId, usize)>,
    /// Provider IDs and their rejected request counts.
    pub rejected_by_provider: Vec<(PeerId, usize)>,
    /// Number of requests reassigned after a provider failure.
    pub retries: usize,
    /// Number of chunks validated from local resume state.
    pub resumed_chunks: usize,
}

/// Advertises and serves one file through an automatically coordinated M4 route.
///
/// # Errors
///
/// Returns an error for file, bootstrap, relay, session, availability, or transfer failures.
pub fn share_file_via_overlay(bootstrap: SocketAddr, path: &Path) -> Result<ContentId> {
    share_file_via_overlay_with_options(bootstrap, path, &ShareOptions::default())
}

/// Advertises and serves one file with explicit M4 provider options.
///
/// # Errors
///
/// Returns an error for file, bootstrap, relay, session, availability, or transfer failures.
pub fn share_file_via_overlay_with_options(
    bootstrap: SocketAddr,
    path: &Path,
    options: &ShareOptions,
) -> Result<ContentId> {
    let (manifest, chunks) = load_content(path)?;
    let content_id = manifest.content_id;
    let availability = parse_availability(options.availability.as_deref(), chunks.len())?;
    let corrupt_chunks = test_corrupt_chunks(chunks.len())?;
    let chunk_delay = test_chunk_delay();
    let limiter = RuntimeLimiter::new(options.upload_limit);
    let keypair = generate_peer_keypair()?;
    let advertisement = PeerAdvertisement {
        peer_id: PeerId::from_public_key(&keypair.public),
        public_key: keypair.public,
        capabilities: vec![PeerCapability::SwarmTransferV0],
        content_ids: vec![content_id],
    };
    let client = BootstrapClient::new(bootstrap);
    client.register_peer(advertisement.clone())?;
    eprintln!("TRIPTORRENT_PROVIDER_READY peer={}", advertisement.peer_id);
    let poll_interval = overlay_poll_interval();
    let idle_marker_after = test_idle_marker_after();
    let mut idle_polls = 0_usize;

    loop {
        let assignment = poll_until_assignment(
            || Ok(client.poll_peer(advertisement.peer_id)?),
            || {
                idle_polls = idle_polls.saturating_add(1);
                if idle_marker_after == Some(idle_polls) {
                    eprintln!("TRIPTORRENT_TEST_IDLE_POLLS_REACHED={idle_polls}");
                }
                thread::sleep(poll_interval);
            },
        )?;
        let relay = assignment
            .relay_address
            .parse::<SocketAddr>()
            .context("bootstrap returned an invalid relay address")?;
        let Ok(transport) = TcpRelayTransport::connect_with_timeout(
            relay,
            &assignment.route,
            RelayRole::Sender,
            OVERLAY_SESSION_TIMEOUT,
        ) else {
            let _ = client.report_relay_failure(&assignment.relay_id);
            continue;
        };
        let mut session = EncryptedSession::provider(transport, &keypair.private)?;
        let content = ProviderContent {
            manifest: &manifest,
            chunks: &chunks,
            availability: &availability,
            corrupt_chunks: &corrupt_chunks,
            chunk_delay,
            limiter: limiter.as_ref(),
            peer_id: advertisement.peer_id,
        };
        serve_swarm_session(&mut session, &content)?;
        return Ok(content_id);
    }
}

struct ProviderContent<'a> {
    manifest: &'a Manifest,
    chunks: &'a [Chunk],
    availability: &'a PieceAvailability,
    corrupt_chunks: &'a [bool],
    chunk_delay: Duration,
    limiter: Option<&'a RuntimeLimiter>,
    peer_id: PeerId,
}

fn serve_swarm_session(
    session: &mut EncryptedSession<TcpRelayTransport>,
    content: &ProviderContent<'_>,
) -> Result<()> {
    match session
        .receive()
        .context("failed to receive swarm request")?
    {
        Message::SwarmRequest { content_id } if content_id == content.manifest.content_id => {}
        Message::SwarmRequest { .. } => {
            session.send(Message::Error(
                "content ID is not shared on this route".into(),
            ))?;
            bail!("receiver requested a different content ID");
        }
        _ => bail!("receiver did not begin with an M4 swarm request"),
    }
    session.send(Message::SwarmManifest {
        manifest: content.manifest.clone(),
        availability: content.availability.clone(),
    })?;

    loop {
        match session.receive()? {
            Message::ChunkRequest { index } if content.availability.contains(index) => {
                let chunk_index = usize::try_from(index).context("invalid chunk index")?;
                let mut chunk = content
                    .chunks
                    .get(chunk_index)
                    .context("requested chunk is outside the manifest")?
                    .clone();
                eprintln!(
                    "TRIPTORRENT_CHUNK_REQUESTED peer={} index={index}",
                    content.peer_id
                );
                if !content.chunk_delay.is_zero() {
                    thread::sleep(content.chunk_delay);
                }
                if content
                    .corrupt_chunks
                    .get(chunk_index)
                    .copied()
                    .unwrap_or(false)
                    && !chunk.data.is_empty()
                {
                    chunk.data[0] ^= 0xff;
                }
                if let Some(limiter) = content.limiter {
                    limiter.wait(u64::try_from(chunk.data.len()).unwrap_or(u64::MAX));
                }
                session.send(Message::Chunk(chunk))?;
                eprintln!(
                    "TRIPTORRENT_CHUNK_SERVED peer={} index={index}",
                    content.peer_id
                );
            }
            Message::ChunkRequest { index } => {
                session.send(Message::ChunkUnavailable { index })?;
            }
            Message::SwarmComplete => return Ok(()),
            _ => bail!("unexpected message during M4 provider session"),
        }
    }
}

fn poll_until_assignment(
    mut poll: impl FnMut() -> Result<Option<RouteAssignment>>,
    mut wait: impl FnMut(),
) -> Result<RouteAssignment> {
    loop {
        if let Some(assignment) = poll()? {
            return Ok(assignment);
        }
        wait();
    }
}

/// Discovers providers and fetches verified chunks concurrently through relayed sessions.
///
/// # Errors
///
/// Returns an error for discovery, relay, session, integrity, or file failures.
pub fn fetch_file_via_overlay(
    bootstrap: SocketAddr,
    content_id: ContentId,
    output: &Path,
) -> Result<()> {
    fetch_file_via_overlay_with_options(bootstrap, content_id, output, FetchOptions::default())?;
    Ok(())
}

/// Runs one M4 swarm fetch and returns transfer-local diagnostic statistics.
///
/// # Errors
///
/// Returns an error for discovery, relay, session, manifest, resume, integrity, or file failures.
pub fn fetch_file_via_overlay_with_options(
    bootstrap: SocketAddr,
    content_id: ContentId,
    output: &Path,
    options: FetchOptions,
) -> Result<TransferStats> {
    let client = BootstrapClient::new(bootstrap);
    let poll_interval = overlay_poll_interval();
    let mut last_error = None;
    for _ in 0..FETCH_MAX_DISCOVERY_ATTEMPTS {
        let assignments = client.discover_providers(content_id, MAX_SWARM_PROVIDERS)?;
        if assignments.is_empty() {
            thread::sleep(poll_interval);
            continue;
        }
        let mut providers = Vec::new();
        for assignment in assignments {
            match connect_swarm_provider(&assignment, content_id) {
                Ok(provider) => providers.push(provider),
                Err(failure) => {
                    if failure.relay_unreachable {
                        let _ = client.report_relay_failure(&assignment.relay_id);
                    }
                    last_error = Some(failure.error);
                }
            }
        }
        if providers.is_empty() {
            thread::sleep(poll_interval);
            continue;
        }
        return receive_swarm(providers, content_id, output, options);
    }
    if let Some(error) = last_error {
        return Err(error.context("all discovered overlay routes failed"));
    }
    bail!("content was not discoverable before the local timeout")
}

struct ConnectedProvider {
    peer_id: PeerId,
    manifest: Manifest,
    availability: PieceAvailability,
    session: EncryptedSession<TcpRelayTransport>,
}

struct ProviderConnectFailure {
    error: anyhow::Error,
    relay_unreachable: bool,
}

fn connect_swarm_provider(
    assignment: &RouteAssignment,
    content_id: ContentId,
) -> std::result::Result<ConnectedProvider, ProviderConnectFailure> {
    let relay = assignment
        .relay_address
        .parse::<SocketAddr>()
        .map_err(|error| ProviderConnectFailure {
            error: anyhow::Error::new(error).context("bootstrap returned an invalid relay address"),
            relay_unreachable: false,
        })?;
    let transport = TcpRelayTransport::connect_with_timeout(
        relay,
        &assignment.route,
        RelayRole::Receiver,
        OVERLAY_SESSION_TIMEOUT,
    )
    .map_err(|error| ProviderConnectFailure {
        error: anyhow::Error::new(error),
        relay_unreachable: true,
    })?;
    let session_result = (|| -> Result<ConnectedProvider> {
        let mut session = EncryptedSession::receiver(transport, &assignment.provider_public_key)?;
        session.send(Message::SwarmRequest { content_id })?;
        let (manifest, availability) = match session.receive()? {
            Message::SwarmManifest {
                manifest,
                availability,
            } => (manifest, availability),
            Message::Error(message) => bail!("provider rejected swarm request: {message}"),
            _ => bail!("provider did not return an M4 manifest"),
        };
        validate_swarm_manifest(&manifest, &availability, content_id)?;
        Ok(ConnectedProvider {
            peer_id: assignment.provider_id,
            manifest,
            availability,
            session,
        })
    })();
    session_result.map_err(|error| ProviderConnectFailure {
        error,
        relay_unreachable: false,
    })
}

fn validate_swarm_manifest(
    manifest: &Manifest,
    availability: &PieceAvailability,
    content_id: ContentId,
) -> Result<()> {
    if manifest.content_id != content_id {
        bail!("provider returned a manifest for a different content ID");
    }
    if usize::try_from(manifest.chunk_size).ok() != Some(CHUNK_SIZE) {
        bail!("manifest uses an unsupported experimental chunk size");
    }
    let expected_chunks = manifest.length.div_ceil(u64::from(manifest.chunk_size));
    if usize::try_from(expected_chunks).ok() != Some(manifest.chunks.len()) {
        bail!("manifest length and chunk count are inconsistent");
    }
    if availability.chunk_count != u32::try_from(manifest.chunks.len()).unwrap_or(u32::MAX)
        || !availability.is_valid()
    {
        bail!("provider returned malformed piece availability");
    }
    Ok(())
}

enum WorkerCommand {
    Fetch { index: u32, expected_bytes: u64 },
    Finish,
}

struct WorkerResult {
    provider: usize,
    requested_index: u32,
    response: Result<Chunk>,
}

struct ProviderWorker {
    peer_id: PeerId,
    sender: mpsc::Sender<WorkerCommand>,
    handle: Option<JoinHandle<()>>,
}

impl ProviderWorker {
    fn finish(&mut self) {
        let _ = self.sender.send(WorkerCommand::Finish);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ProviderWorker {
    fn drop(&mut self) {
        self.finish();
    }
}

fn spawn_provider_worker(
    provider: ConnectedProvider,
    slot: usize,
    results: mpsc::Sender<WorkerResult>,
    limiter: Option<Arc<RuntimeLimiter>>,
) -> ProviderWorker {
    let (sender, commands) = mpsc::channel();
    let peer_id = provider.peer_id;
    let handle = thread::spawn(move || {
        let mut session = provider.session;
        while let Ok(command) = commands.recv() {
            match command {
                WorkerCommand::Fetch {
                    index,
                    expected_bytes,
                } => {
                    if let Some(limiter) = &limiter {
                        limiter.wait(expected_bytes);
                    }
                    let response = session
                        .send(Message::ChunkRequest { index })
                        .map_err(anyhow::Error::new)
                        .and_then(|()| match session.receive().map_err(anyhow::Error::new)? {
                            Message::Chunk(chunk) => Ok(chunk),
                            Message::ChunkUnavailable { index: unavailable }
                                if unavailable == index =>
                            {
                                bail!("provider does not have requested chunk {index}")
                            }
                            message => bail!("unexpected provider response: {message:?}"),
                        });
                    let failed = response.is_err();
                    if results
                        .send(WorkerResult {
                            provider: slot,
                            requested_index: index,
                            response,
                        })
                        .is_err()
                        || failed
                    {
                        break;
                    }
                }
                WorkerCommand::Finish => {
                    let _ = session.send(Message::SwarmComplete);
                    break;
                }
            }
        }
    });
    ProviderWorker {
        peer_id,
        sender,
        handle: Some(handle),
    }
}

fn receive_swarm(
    providers: Vec<ConnectedProvider>,
    content_id: ContentId,
    output: &Path,
    options: FetchOptions,
) -> Result<TransferStats> {
    let manifest = providers[0].manifest.clone();
    if providers
        .iter()
        .any(|provider| provider.manifest != manifest)
    {
        bail!("providers returned inconsistent manifests");
    }
    let (mut resume, mut partial, resumed_chunks) = ResumeDownload::open(output, &manifest)?;
    let availability: Vec<_> = providers
        .iter()
        .map(|provider| provider.availability.clone())
        .collect();
    let scheduler = Scheduler::new(
        manifest.chunks.len(),
        availability,
        &resume.record.completed,
    )?;
    if !scheduler.can_finish() {
        bail!("discovered providers do not cover every missing chunk");
    }

    let limiter = options
        .download_limit
        .filter(|rate| *rate > 0)
        .map(RuntimeLimiter::enabled)
        .map(Arc::new);
    let (result_sender, results) = mpsc::channel();
    let mut workers: Vec<_> = providers
        .into_iter()
        .enumerate()
        .map(|(slot, provider)| {
            spawn_provider_worker(provider, slot, result_sender.clone(), limiter.clone())
        })
        .collect();
    drop(result_sender);

    let scheduler = download_chunks(
        &workers,
        &results,
        &manifest,
        &mut resume,
        &mut partial,
        scheduler,
    )?;

    for worker in &mut workers {
        worker.finish();
    }
    let stats = TransferStats {
        accepted_by_provider: workers
            .iter()
            .enumerate()
            .map(|(slot, worker)| {
                (
                    worker.peer_id,
                    scheduler
                        .provider_stats(slot)
                        .map_or(0, |stats| stats.accepted),
                )
            })
            .collect(),
        rejected_by_provider: workers
            .iter()
            .enumerate()
            .map(|(slot, worker)| {
                (
                    worker.peer_id,
                    scheduler
                        .provider_stats(slot)
                        .map_or(0, |stats| stats.rejected),
                )
            })
            .collect(),
        retries: scheduler.retries(),
        resumed_chunks,
    };
    resume.finish(partial, output, content_id)?;
    emit_stats(&stats);
    Ok(stats)
}

fn download_chunks(
    workers: &[ProviderWorker],
    results: &mpsc::Receiver<WorkerResult>,
    manifest: &Manifest,
    resume: &mut ResumeDownload,
    partial: &mut File,
    mut scheduler: Scheduler,
) -> Result<Scheduler> {
    while !scheduler.is_complete() {
        let mut assigned = false;
        for (slot, worker) in workers.iter().enumerate() {
            if let Some(index) = scheduler.assign(slot) {
                let expected_bytes = expected_chunk_length(manifest, index)?;
                if worker
                    .sender
                    .send(WorkerCommand::Fetch {
                        index,
                        expected_bytes,
                    })
                    .is_err()
                {
                    scheduler.reject(slot);
                } else {
                    assigned = true;
                }
            }
        }
        if scheduler.is_complete() {
            break;
        }
        if !scheduler.has_in_flight() && !assigned {
            bail!("no healthy provider can supply the remaining chunks");
        }

        let result = results
            .recv_timeout(WORKER_TIMEOUT)
            .context("timed out waiting for a provider chunk")?;
        match result.response {
            Ok(chunk)
                if chunk.index == result.requested_index
                    && manifest.verify_chunk(&chunk).is_ok() =>
            {
                resume.write_verified_chunk(partial, manifest, &chunk)?;
                scheduler.accept(result.provider, chunk.index)?;
                eprintln!(
                    "TRIPTORRENT_CHUNK_ACCEPTED provider={} index={} resumed=false",
                    workers[result.provider].peer_id, chunk.index
                );
            }
            _ => {
                eprintln!(
                    "TRIPTORRENT_CHUNK_REJECTED provider={} index={}",
                    workers[result.provider].peer_id, result.requested_index
                );
                scheduler.reject(result.provider);
                if !scheduler.can_finish() {
                    bail!("no honest provider remains for a required chunk");
                }
            }
        }
    }

    Ok(scheduler)
}

fn emit_stats(stats: &TransferStats) {
    let accepted = stats
        .accepted_by_provider
        .iter()
        .map(|(peer, count)| format!("{peer}:{count}"))
        .collect::<Vec<_>>()
        .join(",");
    let rejected = stats
        .rejected_by_provider
        .iter()
        .map(|(peer, count)| format!("{peer}:{count}"))
        .collect::<Vec<_>>()
        .join(",");
    eprintln!(
        "TRIPTORRENT_SWARM_STATS accepted={accepted} rejected={rejected} retries={} resumed={}",
        stats.retries, stats.resumed_chunks
    );
}

#[derive(Debug, Serialize, Deserialize)]
struct ResumeRecord {
    version: u16,
    manifest: Manifest,
    completed: Vec<bool>,
}

struct ResumeDownload {
    part_path: PathBuf,
    state_path: PathBuf,
    record: ResumeRecord,
}

impl ResumeDownload {
    fn open(output: &Path, manifest: &Manifest) -> Result<(Self, File, usize)> {
        let part_path = append_suffix(output, ".triptorrent-part");
        let state_path = append_suffix(output, ".triptorrent-state");
        let state_exists = state_path.exists();
        let part_exists = part_path.exists();
        if state_exists != part_exists {
            bail!("resume data is incomplete; both partial file and state are required");
        }

        if state_exists {
            let encoded = fs::read(&state_path)
                .with_context(|| format!("failed to read resume state {}", state_path.display()))?;
            let record: ResumeRecord = postcard::from_bytes(&encoded)
                .context("resume state is malformed or incompatible")?;
            if record.version != RESUME_VERSION || record.manifest != *manifest {
                bail!("resume state does not match the requested content manifest");
            }
            if record.completed.len() != manifest.chunks.len() {
                bail!("resume state has an invalid chunk count");
            }
            let mut partial = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&part_path)
                .with_context(|| format!("failed to open partial file {}", part_path.display()))?;
            if partial.metadata()?.len() != manifest.length {
                bail!("partial file length does not match the resume manifest");
            }
            validate_resumed_chunks(&mut partial, manifest, &record.completed)?;
            let resumed = record.completed.iter().filter(|done| **done).count();
            for (index, done) in record.completed.iter().enumerate() {
                if *done {
                    eprintln!("TRIPTORRENT_CHUNK_ACCEPTED index={index} resumed=true");
                }
            }
            Ok((
                Self {
                    part_path,
                    state_path,
                    record,
                },
                partial,
                resumed,
            ))
        } else {
            let partial = OpenOptions::new()
                .create(true)
                .truncate(true)
                .read(true)
                .write(true)
                .open(&part_path)
                .with_context(|| {
                    format!("failed to create partial file {}", part_path.display())
                })?;
            partial.set_len(manifest.length)?;
            let resume = Self {
                part_path,
                state_path,
                record: ResumeRecord {
                    version: RESUME_VERSION,
                    manifest: manifest.clone(),
                    completed: vec![false; manifest.chunks.len()],
                },
            };
            resume.save_state()?;
            Ok((resume, partial, 0))
        }
    }

    fn write_verified_chunk(
        &mut self,
        partial: &mut File,
        manifest: &Manifest,
        chunk: &Chunk,
    ) -> Result<()> {
        manifest.verify_chunk(chunk)?;
        let offset = u64::from(chunk.index).saturating_mul(u64::from(manifest.chunk_size));
        partial.seek(SeekFrom::Start(offset))?;
        partial.write_all(&chunk.data)?;
        partial.sync_data()?;
        let index = usize::try_from(chunk.index).context("chunk index does not fit usize")?;
        self.record.completed[index] = true;
        self.save_state()
    }

    fn save_state(&self) -> Result<()> {
        let encoded = postcard::to_allocvec(&self.record)?;
        let temporary = append_suffix(&self.state_path, ".tmp");
        fs::write(&temporary, encoded)
            .with_context(|| format!("failed to write resume state {}", temporary.display()))?;
        replace_file(&temporary, &self.state_path)?;
        Ok(())
    }

    fn finish(self, mut partial: File, output: &Path, content_id: ContentId) -> Result<()> {
        partial.flush()?;
        partial.seek(SeekFrom::Start(0))?;
        let mut bytes =
            Vec::with_capacity(usize::try_from(self.record.manifest.length).unwrap_or(0));
        partial.read_to_end(&mut bytes)?;
        if u64::try_from(bytes.len()).ok() != Some(self.record.manifest.length)
            || ContentId::digest(&bytes) != content_id
        {
            bail!("completed partial file failed content-ID verification");
        }
        drop(partial);
        replace_file(&self.part_path, output)
            .with_context(|| format!("failed to finalize output file {}", output.display()))?;
        fs::remove_file(&self.state_path).with_context(|| {
            format!(
                "failed to remove resume state {}",
                self.state_path.display()
            )
        })?;
        Ok(())
    }
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

fn validate_resumed_chunks(
    partial: &mut File,
    manifest: &Manifest,
    completed: &[bool],
) -> Result<()> {
    for (index, done) in completed.iter().enumerate() {
        if !done {
            continue;
        }
        let index_u32 = u32::try_from(index).context("resume chunk index overflow")?;
        let length = usize::try_from(expected_chunk_length(manifest, index_u32)?)?;
        let offset = u64::from(index_u32).saturating_mul(u64::from(manifest.chunk_size));
        partial.seek(SeekFrom::Start(offset))?;
        let mut data = vec![0; length];
        partial.read_exact(&mut data)?;
        let chunk = Chunk {
            index: index_u32,
            id: manifest.chunks[index],
            data,
        };
        manifest
            .verify_chunk(&chunk)
            .with_context(|| format!("resume chunk {index} failed integrity verification"))?;
    }
    Ok(())
}

fn expected_chunk_length(manifest: &Manifest, index: u32) -> Result<u64> {
    if usize::try_from(index)
        .ok()
        .is_none_or(|value| value >= manifest.chunks.len())
    {
        bail!("chunk index {index} is outside the manifest");
    }
    let offset = u64::from(index).saturating_mul(u64::from(manifest.chunk_size));
    Ok((manifest.length - offset).min(u64::from(manifest.chunk_size)))
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value: OsString = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}

struct RuntimeLimiter {
    epoch: Instant,
    schedule: Mutex<RateSchedule>,
}

impl RuntimeLimiter {
    fn new(rate: Option<u64>) -> Option<Self> {
        rate.filter(|value| *value > 0).map(Self::enabled)
    }

    fn enabled(rate: u64) -> Self {
        Self {
            epoch: Instant::now(),
            schedule: Mutex::new(RateSchedule::new(rate)),
        }
    }

    fn wait(&self, bytes: u64) {
        let delay = self.schedule.lock().map_or(Duration::ZERO, |mut schedule| {
            schedule.reserve(self.epoch.elapsed(), bytes)
        });
        if !delay.is_zero() {
            thread::sleep(delay);
        }
    }
}

fn parse_availability(spec: Option<&str>, chunk_count: usize) -> Result<PieceAvailability> {
    let chunk_count_u32 = u32::try_from(chunk_count).context("too many chunks")?;
    let Some(spec) = spec else {
        return Ok(PieceAvailability::all(chunk_count_u32));
    };
    if spec.eq_ignore_ascii_case("all") {
        return Ok(PieceAvailability::all(chunk_count_u32));
    }
    let mut indices = Vec::new();
    for item in spec.split(',').filter(|item| !item.is_empty()) {
        if let Some((start, end)) = item.split_once('-') {
            let start: u32 = start.parse().context("invalid availability range start")?;
            let end: u32 = end.parse().context("invalid availability range end")?;
            if start > end || end >= chunk_count_u32 {
                bail!("availability range {item} is outside the manifest");
            }
            indices.extend(start..=end);
        } else {
            let index: u32 = item.parse().context("invalid availability index")?;
            if index >= chunk_count_u32 {
                bail!("availability index {index} is outside the manifest");
            }
            indices.push(index);
        }
    }
    if indices.is_empty() && chunk_count > 0 {
        bail!("availability must contain at least one chunk");
    }
    Ok(PieceAvailability::from_indices(chunk_count_u32, indices))
}

fn test_corrupt_chunks(chunk_count: usize) -> Result<Vec<bool>> {
    let mut corrupt = vec![false; chunk_count];
    let Ok(spec) = std::env::var(TEST_CORRUPT_CHUNKS_ENV) else {
        return Ok(corrupt);
    };
    for item in spec.split(',').filter(|item| !item.is_empty()) {
        let index: usize = item.parse().context("invalid test corrupt-chunk index")?;
        let flag = corrupt
            .get_mut(index)
            .with_context(|| format!("test corrupt-chunk index {index} is outside the manifest"))?;
        *flag = true;
    }
    Ok(corrupt)
}

fn test_chunk_delay() -> Duration {
    std::env::var(TEST_CHUNK_DELAY_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map_or(Duration::ZERO, Duration::from_millis)
}

fn overlay_poll_interval() -> Duration {
    std::env::var(TEST_POLL_INTERVAL_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map_or(OVERLAY_POLL_INTERVAL, Duration::from_millis)
}

fn test_idle_marker_after() -> Option<usize> {
    std::env::var(TEST_IDLE_MARKER_ENV)
        .ok()
        .and_then(|value| value.parse().ok())
}

/// Parses a 32-byte hexadecimal pre-shared session key.
///
/// # Errors
///
/// Returns an error when the value is not exactly 64 hexadecimal characters.
pub fn parse_psk(value: &str) -> Result<[u8; 32], String> {
    let decoded =
        hex::decode(value).map_err(|error| format!("invalid hexadecimal key: {error}"))?;
    decoded
        .try_into()
        .map_err(|_| "key must contain exactly 64 hexadecimal characters".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn overlay_share_keeps_polling_past_fetch_discovery_limit() {
        let public_key = [7; 32];
        let expected = RouteAssignment {
            provider_id: PeerId::from_public_key(&public_key),
            provider_public_key: public_key,
            relay_id: "relay-a".into(),
            relay_address: "127.0.0.1:7000".into(),
            route: "test-route".into(),
        };
        let mut polls = 0;

        let assignment = poll_until_assignment(
            || {
                polls += 1;
                Ok((polls > FETCH_MAX_DISCOVERY_ATTEMPTS).then(|| expected.clone()))
            },
            || {},
        )
        .expect("polling should continue until an assignment is available");

        assert_eq!(polls, FETCH_MAX_DISCOVERY_ATTEMPTS + 1);
        assert_eq!(assignment, expected);
    }

    #[test]
    fn resume_rejects_corrupted_validated_chunk() {
        let directory = tempdir().unwrap();
        let output = directory.path().join("output.bin");
        let (manifest, chunks) = Manifest::from_bytes(&vec![7; CHUNK_SIZE + 3]).unwrap();
        let (mut resume, mut partial, _) = ResumeDownload::open(&output, &manifest).unwrap();
        resume
            .write_verified_chunk(&mut partial, &manifest, &chunks[0])
            .unwrap();
        drop(partial);
        drop(resume);

        let part_path = append_suffix(&output, ".triptorrent-part");
        let mut file = OpenOptions::new().write(true).open(part_path).unwrap();
        file.write_all(b"corrupt").unwrap();
        drop(file);

        assert!(ResumeDownload::open(&output, &manifest).is_err());
    }

    #[test]
    fn resume_is_bound_to_the_exact_manifest() {
        let directory = tempdir().unwrap();
        let output = directory.path().join("output.bin");
        let first = Manifest::from_bytes(b"first content").unwrap().0;
        let second = Manifest::from_bytes(b"second content").unwrap().0;
        let (resume, partial, _) = ResumeDownload::open(&output, &first).unwrap();
        drop(partial);
        drop(resume);
        assert!(ResumeDownload::open(&output, &second).is_err());
    }
}
