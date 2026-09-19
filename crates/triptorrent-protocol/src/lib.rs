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
        let manifest_message = Message::Manifest(manifest);
        assert_eq!(
            decode(&encode(manifest_message.clone()).unwrap()).unwrap(),
            manifest_message
        );
        let chunk_message = Message::Chunk(chunks[0].clone());
        assert_eq!(
            decode(&encode(chunk_message.clone()).unwrap()).unwrap(),
            chunk_message
        );
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
