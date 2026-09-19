#![doc = "Experimental M1 file-transfer messages and serialization."]

use serde::{Deserialize, Serialize};
use thiserror::Error;
use triptorrent_core::{Chunk, ContentId, Manifest, PROTOCOL_VERSION};

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
}
