#![doc = "Reusable file-transfer operations for the local M1 command-line demo."]

use anyhow::{Context, Result, bail};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use triptorrent_core::{CHUNK_SIZE, ContentId, Manifest};
use triptorrent_net::{EncryptedSession, RelayRole, TcpRelayTransport};
use triptorrent_protocol::Message;

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
    let bytes =
        fs::read(path).with_context(|| format!("failed to read source file {}", path.display()))?;
    let (manifest, chunks) = Manifest::from_bytes(&bytes)?;
    let content_id = manifest.content_id;

    let transport = TcpRelayTransport::connect(relay, route, RelayRole::Sender)
        .context("failed to connect sender to relay")?;
    let mut session = EncryptedSession::initiator(transport, psk)
        .context("failed to establish encrypted peer session")?;

    match session
        .receive()
        .context("failed to receive content request")?
    {
        Message::Request {
            content_id: requested,
        } if requested == content_id => {}
        Message::Request { .. } => {
            session.send(Message::Error(
                "content ID is not shared on this route".into(),
            ))?;
            bail!("receiver requested a different content ID");
        }
        _ => bail!("receiver did not begin with a content request"),
    }

    session.send(Message::Manifest(manifest))?;
    for chunk in chunks {
        session.send(Message::Chunk(chunk))?;
    }
    session.send(Message::Complete)?;
    Ok(content_id)
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
