#![doc = "Experimental M1 file-transfer messages and serialization."]

use serde::{Deserialize, Serialize};
use thiserror::Error;
use triptorrent_core::{Chunk, ContentId, Manifest, PROTOCOL_VERSION, PeerId};

/// A versioned M1 application message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// Experimental wire version.
    pub version: u16,
    /// File-transfer message.
    pub message: Message,
}

/// M1 request and response messages carried inside an encrypted session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// Request content by its experimental identifier.
    Request { content_id: ContentId },
    /// Describe the requested content and its chunks.
    Manifest(Manifest),
    /// Deliver one plaintext chunk inside the encrypted session.
    Chunk(Chunk),
    /// Signal that all chunks have been sent.
    Complete,
    /// Explain why the request could not be served.
    Error(String),
    /// Begin an experimental M4 piece-scheduled session.
    SwarmRequest { content_id: ContentId },
    /// Return the authoritative manifest and this provider's piece availability.
    SwarmManifest {
        manifest: Manifest,
        availability: PieceAvailability,
    },
    /// Request one chunk by its manifest index.
    ChunkRequest { index: u32 },
    /// Report that a requested chunk is not available from this provider.
    ChunkUnavailable { index: u32 },
    /// End an M4 provider session after all needed requests have completed.
    SwarmComplete,
}

/// Compact bitfield describing the chunks a provider can serve.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PieceAvailability {
    /// Number of meaningful bits in `bits`.
    pub chunk_count: u32,
    /// Least-significant-bit-first bitfield; bit `n` represents chunk `n`.
    pub bits: Vec<u8>,
}

impl PieceAvailability {
    /// Builds a bitfield containing every chunk.
    #[must_use]
    pub fn all(chunk_count: u32) -> Self {
        let byte_count = usize::try_from(chunk_count.div_ceil(8)).unwrap_or(usize::MAX);
        let mut value = Self {
            chunk_count,
            bits: vec![0xff; byte_count],
        };
        value.clear_unused_bits();
        value
    }

    /// Builds a bitfield from explicitly available indices.
    #[must_use]
    pub fn from_indices(chunk_count: u32, indices: impl IntoIterator<Item = u32>) -> Self {
        let byte_count = usize::try_from(chunk_count.div_ceil(8)).unwrap_or(usize::MAX);
        let mut value = Self {
            chunk_count,
            bits: vec![0; byte_count],
        };
        for index in indices {
            if index < chunk_count {
                let byte = usize::try_from(index / 8).unwrap_or(usize::MAX);
                value.bits[byte] |= 1 << (index % 8);
            }
        }
        value
    }

    /// Returns whether this well-formed bitfield contains `index`.
    #[must_use]
    pub fn contains(&self, index: u32) -> bool {
        if !self.is_valid() || index >= self.chunk_count {
            return false;
        }
        let byte = usize::try_from(index / 8).unwrap_or(usize::MAX);
        self.bits[byte] & (1 << (index % 8)) != 0
    }

    /// Checks length and zero padding in the last byte.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let expected = usize::try_from(self.chunk_count.div_ceil(8)).unwrap_or(usize::MAX);
        if self.bits.len() != expected {
            return false;
        }
        let remainder = self.chunk_count % 8;
        remainder == 0
            || self
                .bits
                .last()
                .is_none_or(|last| last & !((1_u8 << remainder) - 1) == 0)
    }

    fn clear_unused_bits(&mut self) {
        let remainder = self.chunk_count % 8;
        if remainder != 0
            && let Some(last) = self.bits.last_mut()
        {
            *last &= (1_u8 << remainder) - 1;
        }
    }
}

/// A peer advertisement held under a temporary M2 lease.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PeerAdvertisement {
    /// Ephemeral identity, derived from `public_key`.
    pub peer_id: PeerId,
    /// Noise static public key used only for this sharing process.
    pub public_key: [u8; 32],
    /// Experimental transfer capabilities supported by this process.
    pub capabilities: Vec<PeerCapability>,
    /// Experimental content IDs currently offered by the peer.
    pub content_ids: Vec<ContentId>,
}

/// Experimental capabilities advertised through the M2 bootstrap.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PeerCapability {
    /// Supports the M1 chunk protocol through one automatically selected relay.
    RelayedTransferV0,
    /// Supports M4 manifest/availability exchange and requested chunks.
    SwarmTransferV0,
}

/// A relay advertisement held under a temporary M2 lease.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RelayAdvertisement {
    /// Caller-selected ephemeral relay identifier.
    pub relay_id: String,
    /// Relay TCP endpoint encoded as `host:port`.
    pub address: String,
}

/// Automatically coordinated M1 route returned by M2 discovery.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteAssignment {
    /// Peer advertising the requested content.
    pub provider_id: PeerId,
    /// Provider public key used by the receiver's Noise handshake.
    pub provider_public_key: [u8; 32],
    /// Selected relay identifier.
    pub relay_id: String,
    /// Selected relay TCP endpoint.
    pub relay_address: String,
    /// Automatically generated opaque relay route.
    pub route: String,
}

/// Experimental request sent to the temporary M2 bootstrap service.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OverlayRequest {
    /// Register or replace a peer's leased content advertisement.
    RegisterPeer(PeerAdvertisement),
    /// Renew an existing peer lease.
    HeartbeatPeer { peer_id: PeerId },
    /// Renew a peer lease and retrieve its next route assignment.
    PollPeer { peer_id: PeerId },
    /// Register or replace a relay's leased availability advertisement.
    RegisterRelay(RelayAdvertisement),
    /// Renew an existing relay lease.
    HeartbeatRelay { relay_id: String },
    /// Locate a provider and coordinate an automatic relay route.
    Discover { content_id: ContentId },
    /// Locate several providers and coordinate one independent route per provider.
    DiscoverProviders { content_id: ContentId, limit: u16 },
    /// Remove a relay that failed before a transfer could start.
    ReportRelayFailure { relay_id: String },
}

/// Experimental response from the temporary M2 bootstrap service.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OverlayResponse {
    /// Registration or heartbeat succeeded for the stated lease duration.
    Registered { lease_ms: u64 },
    /// A provider-side route assignment, when one is queued.
    Assignment(Option<RouteAssignment>),
    /// A receiver-side discovery result.
    Route(Option<RouteAssignment>),
    /// Receiver-side routes for distinct providers of one content ID.
    Routes(Vec<RouteAssignment>),
    /// The request completed without a result body.
    Acknowledged,
    /// The bootstrap rejected the request.
    Error(String),
}

#[derive(Serialize, Deserialize)]
struct OverlayRequestEnvelope {
    version: u16,
    request: OverlayRequest,
}

#[derive(Serialize, Deserialize)]
struct OverlayResponseEnvelope {
    version: u16,
    response: OverlayResponse,
}

/// Wire encoding or compatibility failure.
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// Postcard could not encode or decode the message.
    #[error("invalid wire message: {0}")]
    Codec(#[from] postcard::Error),
    /// The peer used a different experimental protocol version.
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u16),
}

/// Serializes a versioned message.
///
/// # Errors
///
/// Returns [`ProtocolError`] if serialization fails.
pub fn encode(message: Message) -> Result<Vec<u8>, ProtocolError> {
    Ok(postcard::to_allocvec(&Envelope {
        version: PROTOCOL_VERSION,
        message,
    })?)
}

/// Deserializes a message and rejects unknown versions.
///
/// # Errors
///
/// Returns [`ProtocolError`] for malformed or incompatible input.
pub fn decode(bytes: &[u8]) -> Result<Message, ProtocolError> {
    let envelope: Envelope = postcard::from_bytes(bytes)?;
    if envelope.version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(envelope.version));
    }
    Ok(envelope.message)
}

/// Serializes an experimental M2 bootstrap request.
///
/// # Errors
///
/// Returns [`ProtocolError`] if serialization fails.
pub fn encode_overlay_request(request: OverlayRequest) -> Result<Vec<u8>, ProtocolError> {
    Ok(postcard::to_allocvec(&OverlayRequestEnvelope {
        version: PROTOCOL_VERSION,
        request,
    })?)
}

/// Decodes an experimental M2 bootstrap request.
///
/// # Errors
///
/// Returns [`ProtocolError`] for malformed or incompatible input.
pub fn decode_overlay_request(bytes: &[u8]) -> Result<OverlayRequest, ProtocolError> {
    let envelope: OverlayRequestEnvelope = postcard::from_bytes(bytes)?;
    verify_version(envelope.version)?;
    Ok(envelope.request)
}

/// Serializes an experimental M2 bootstrap response.
///
/// # Errors
///
/// Returns [`ProtocolError`] if serialization fails.
pub fn encode_overlay_response(response: OverlayResponse) -> Result<Vec<u8>, ProtocolError> {
    Ok(postcard::to_allocvec(&OverlayResponseEnvelope {
        version: PROTOCOL_VERSION,
        response,
    })?)
}

/// Decodes an experimental M2 bootstrap response.
///
/// # Errors
///
/// Returns [`ProtocolError`] for malformed or incompatible input.
pub fn decode_overlay_response(bytes: &[u8]) -> Result<OverlayResponse, ProtocolError> {
    let envelope: OverlayResponseEnvelope = postcard::from_bytes(bytes)?;
    verify_version(envelope.version)?;
    Ok(envelope.response)
}

fn verify_version(version: u16) -> Result<(), ProtocolError> {
    if version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(version));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let content_id = ContentId::digest(b"requested bytes");
        let messages = [
            Message::Request { content_id },
            Message::Complete,
            Message::Error("unavailable".into()),
        ];
        for message in messages {
            assert_eq!(decode(&encode(message.clone()).unwrap()).unwrap(), message);
        }

        let (manifest, chunks) = Manifest::from_bytes(b"chunked bytes").unwrap();
        let manifest_message = Message::Manifest(manifest.clone());
        assert_eq!(
            decode(&encode(manifest_message.clone()).unwrap()).unwrap(),
            manifest_message
        );
        let chunk_message = Message::Chunk(chunks[0].clone());
        assert_eq!(
            decode(&encode(chunk_message.clone()).unwrap()).unwrap(),
            chunk_message
        );
        let availability = PieceAvailability::all(
            u32::try_from(chunks.len()).expect("test manifest has few chunks"),
        );
        let swarm_message = Message::SwarmManifest {
            manifest,
            availability,
        };
        assert_eq!(
            decode(&encode(swarm_message.clone()).unwrap()).unwrap(),
            swarm_message
        );
    }

    #[test]
    fn piece_availability_is_compact_and_validated() {
        let availability = PieceAvailability::from_indices(10, [0, 3, 9, 99]);
        assert_eq!(availability.bits.len(), 2);
        assert!(availability.is_valid());
        assert!(availability.contains(0));
        assert!(availability.contains(3));
        assert!(availability.contains(9));
        assert!(!availability.contains(1));

        let malformed = PieceAvailability {
            chunk_count: 9,
            bits: vec![0, 0x80],
        };
        assert!(!malformed.is_valid());
    }

    #[test]
    fn encoding_is_deterministic() {
        let message = Message::Request {
            content_id: ContentId::digest(b"stable encoding input"),
        };
        assert_eq!(encode(message.clone()).unwrap(), encode(message).unwrap());
    }

    #[test]
    fn malformed_messages_are_rejected() {
        assert!(decode(&[0xff, 0xff]).is_err());
    }

    #[test]
    fn unsupported_versions_are_rejected() {
        let encoded = postcard::to_allocvec(&Envelope {
            version: PROTOCOL_VERSION + 1,
            message: Message::Complete,
        })
        .unwrap();
        assert!(matches!(
            decode(&encoded),
            Err(ProtocolError::UnsupportedVersion(version)) if version == PROTOCOL_VERSION + 1
        ));
    }

    #[test]
    fn overlay_messages_round_trip() {
        let public_key = [7; 32];
        let peer_id = PeerId::from_public_key(&public_key);
        let request = OverlayRequest::RegisterPeer(PeerAdvertisement {
            peer_id,
            public_key,
            capabilities: vec![PeerCapability::RelayedTransferV0],
            content_ids: vec![ContentId::digest(b"overlay content")],
        });
        assert_eq!(
            decode_overlay_request(&encode_overlay_request(request.clone()).unwrap()).unwrap(),
            request
        );

        let response = OverlayResponse::Route(Some(RouteAssignment {
            provider_id: peer_id,
            provider_public_key: public_key,
            relay_id: "relay-a".into(),
            relay_address: "127.0.0.1:7000".into(),
            route: "m2-0001".into(),
        }));
        assert_eq!(
            decode_overlay_response(&encode_overlay_response(response.clone()).unwrap()).unwrap(),
            response
        );
    }
}
