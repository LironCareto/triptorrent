#![doc = "Reusable M1 transfer and M2 overlay operations for the local demo."]

use anyhow::{Context, Result, bail};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::thread;
use std::time::Duration;
use triptorrent_core::{CHUNK_SIZE, Chunk, ContentId, Manifest, PeerId};
use triptorrent_net::{EncryptedSession, RelayRole, TcpRelayTransport, generate_peer_keypair};
use triptorrent_overlay::BootstrapClient;
use triptorrent_protocol::{Message, PeerAdvertisement, PeerCapability, RouteAssignment};

const OVERLAY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const OVERLAY_SESSION_TIMEOUT: Duration = Duration::from_secs(5);
const FETCH_MAX_DISCOVERY_ATTEMPTS: usize = 600;

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

/// Advertises and serves one file through an automatically coordinated M2 route.
///
/// # Errors
///
/// Returns an error for file, bootstrap, relay, session, or transfer failures.
pub fn share_file_via_overlay(bootstrap: SocketAddr, path: &Path) -> Result<ContentId> {
    let (manifest, chunks) = load_content(path)?;
    let content_id = manifest.content_id;
    let keypair = generate_peer_keypair()?;
    let advertisement = PeerAdvertisement {
        peer_id: PeerId::from_public_key(&keypair.public),
        public_key: keypair.public,
        capabilities: vec![PeerCapability::RelayedTransferV0],
        content_ids: vec![content_id],
    };
    let client = BootstrapClient::new(bootstrap);
    client.register_peer(advertisement.clone())?;

    loop {
        let assignment = poll_until_assignment(
            || Ok(client.poll_peer(advertisement.peer_id)?),
            || thread::sleep(OVERLAY_POLL_INTERVAL),
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
        send_content(&mut session, &manifest, &chunks)?;
        return Ok(content_id);
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

/// Discovers a provider and relay, then fetches and verifies content through M1 framing.
///
/// # Errors
///
/// Returns an error for discovery, relay, session, integrity, or file failures.
pub fn fetch_file_via_overlay(
    bootstrap: SocketAddr,
    content_id: ContentId,
    output: &Path,
) -> Result<()> {
    let client = BootstrapClient::new(bootstrap);
    let mut last_error = None;
    for _ in 0..FETCH_MAX_DISCOVERY_ATTEMPTS {
        let Some(assignment) = client.discover(content_id)? else {
            thread::sleep(OVERLAY_POLL_INTERVAL);
            continue;
        };
        let relay = assignment
            .relay_address
            .parse::<SocketAddr>()
            .context("bootstrap returned an invalid relay address")?;
        let transport = match TcpRelayTransport::connect_with_timeout(
            relay,
            &assignment.route,
            RelayRole::Receiver,
            OVERLAY_SESSION_TIMEOUT,
        ) {
            Ok(transport) => transport,
            Err(error) => {
                let _ = client.report_relay_failure(&assignment.relay_id);
                last_error = Some(anyhow::Error::new(error));
                thread::sleep(OVERLAY_POLL_INTERVAL);
                continue;
            }
        };
        let mut session = EncryptedSession::receiver(transport, &assignment.provider_public_key)?;
        receive_content(&mut session, content_id, output)?;
        return Ok(());
    }
    if let Some(error) = last_error {
        return Err(error.context("all discovered overlay routes failed"));
    }
    bail!("content was not discoverable before the local timeout")
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
}
