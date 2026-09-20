#![doc = "Protocol-independent content types for the `TripTorrent` prototype."]

use core::fmt;
use core::str::FromStr;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Placeholder protocol version while the specification is pre-alpha.
pub const PROTOCOL_VERSION: u16 = 0;

/// Fixed chunk size used by the experimental M1 content model.
pub const CHUNK_SIZE: usize = 32 * 1024;

/// Maximum chunk count accepted by the experimental manifest implementation.
pub const MAX_MANIFEST_CHUNKS: usize = 1_048_576;

/// Maximum content length represented by one experimental manifest.
pub const MAX_CONTENT_LENGTH: u64 = 32 * 1024 * 1_048_576;

/// A temporary BLAKE3 identifier for complete file bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ContentId([u8; 32]);

impl ContentId {
    /// Computes the experimental content identifier for `bytes`.
    #[must_use]
    pub fn digest(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Returns the raw 32-byte digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

impl FromStr for ContentId {
    type Err = ParseIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let decoded = hex::decode(value).map_err(ParseIdError::Hex)?;
        let bytes = decoded
            .try_into()
            .map_err(|_| ParseIdError::Length(value.len()))?;
        Ok(Self(bytes))
    }
}

/// An ephemeral, experimental peer identifier derived from a Noise public key.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PeerId([u8; 32]);

impl PeerId {
    /// Derives a peer identifier from an advertised public key.
    #[must_use]
    pub fn from_public_key(public_key: &[u8; 32]) -> Self {
        Self(*blake3::hash(public_key).as_bytes())
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

/// A BLAKE3 identifier for one chunk.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ChunkId([u8; 32]);

impl ChunkId {
    /// Computes a chunk identifier.
    #[must_use]
    pub fn digest(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }
}

/// Metadata needed to verify and reconstruct one file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Experimental identifier of the complete file.
    pub content_id: ContentId,
    /// File length in bytes.
    pub length: u64,
    /// Deterministic chunk size used by the producer.
    pub chunk_size: u32,
    /// Expected chunk identifiers, in file order.
    pub chunks: Vec<ChunkId>,
}

/// A numbered file chunk and its claimed identifier.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    /// Zero-based position in the manifest.
    pub index: u32,
    /// Claimed digest of `data`.
    pub id: ChunkId,
    /// Plaintext chunk bytes. These only travel inside the encrypted session.
    pub data: Vec<u8>,
}

/// Content parsing or integrity failure.
#[derive(Debug, Error)]
pub enum ContentError {
    /// A chunk index was outside the manifest.
    #[error("chunk index {0} is outside the manifest")]
    InvalidIndex(u32),
    /// A received chunk did not match its claimed or expected digest.
    #[error("chunk {0} failed integrity verification")]
    InvalidChunk(u32),
    /// The chunk collection was incomplete or out of order.
    #[error("expected {expected} chunks, received {actual}")]
    WrongChunkCount { expected: usize, actual: usize },
    /// Reconstructed bytes did not match the manifest.
    #[error("reconstructed content failed integrity verification")]
    InvalidContent,
    /// A file had more chunks than the M1 index can represent.
    #[error("content has more than u32::MAX chunks")]
    TooManyChunks,
    /// The manifest does not use the fixed experimental chunk size.
    #[error("manifest uses an unsupported chunk size")]
    InvalidChunkSize,
    /// Length and chunk count do not describe the same byte sequence.
    #[error("manifest length and chunk count are inconsistent")]
    InvalidManifestShape,
    /// The manifest exceeds the explicit prototype content bound.
    #[error("content exceeds the experimental size limit")]
    ContentTooLarge,
}

/// Invalid textual content identifier.
#[derive(Debug, Error)]
pub enum ParseIdError {
    /// The identifier was not hexadecimal.
    #[error("content ID is not valid hexadecimal: {0}")]
    Hex(hex::FromHexError),
    /// The identifier was not a 32-byte digest.
    #[error("content ID must contain 64 hexadecimal characters, got {0}")]
    Length(usize),
}

impl Manifest {
    /// Builds a manifest and deterministic chunks from complete file bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ContentError::TooManyChunks`] when M1's 32-bit index is insufficient.
    pub fn from_bytes(bytes: &[u8]) -> Result<(Self, Vec<Chunk>), ContentError> {
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CONTENT_LENGTH {
            return Err(ContentError::ContentTooLarge);
        }
        let chunks: Vec<Chunk> = bytes
            .chunks(CHUNK_SIZE)
            .enumerate()
            .map(|(index, data)| {
                Ok(Chunk {
                    index: u32::try_from(index).map_err(|_| ContentError::TooManyChunks)?,
                    id: ChunkId::digest(data),
                    data: data.to_vec(),
                })
            })
            .collect::<Result<_, ContentError>>()?;
        let manifest = Self {
            content_id: ContentId::digest(bytes),
            length: u64::try_from(bytes.len()).map_err(|_| ContentError::TooManyChunks)?,
            chunk_size: 32 * 1024,
            chunks: chunks.iter().map(|chunk| chunk.id).collect(),
        };
        Ok((manifest, chunks))
    }

    /// Checks the structural invariants required before allocating transfer state.
    ///
    /// # Errors
    ///
    /// Returns [`ContentError`] for unsupported size, chunk size, or inconsistent length.
    pub fn validate_shape(&self) -> Result<(), ContentError> {
        if usize::try_from(self.chunk_size).ok() != Some(CHUNK_SIZE) {
            return Err(ContentError::InvalidChunkSize);
        }
        if self.length > MAX_CONTENT_LENGTH || self.chunks.len() > MAX_MANIFEST_CHUNKS {
            return Err(ContentError::ContentTooLarge);
        }
        let expected = self.length.div_ceil(u64::from(self.chunk_size));
        if usize::try_from(expected).ok() != Some(self.chunks.len()) {
            return Err(ContentError::InvalidManifestShape);
        }
        Ok(())
    }

    /// Verifies the index and digest of one chunk.
    ///
    /// # Errors
    ///
    /// Returns [`ContentError`] when the index or digest is invalid.
    pub fn verify_chunk(&self, chunk: &Chunk) -> Result<(), ContentError> {
        let index =
            usize::try_from(chunk.index).map_err(|_| ContentError::InvalidIndex(chunk.index))?;
        let expected = self
            .chunks
            .get(index)
            .ok_or(ContentError::InvalidIndex(chunk.index))?;
        if chunk.id != *expected || ChunkId::digest(&chunk.data) != *expected {
            return Err(ContentError::InvalidChunk(chunk.index));
        }
        Ok(())
    }

    /// Verifies ordered chunks and reconstructs the original bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ContentError`] for missing, reordered, corrupt, or mismatched content.
    pub fn reconstruct(&self, chunks: &[Chunk]) -> Result<Vec<u8>, ContentError> {
        if chunks.len() != self.chunks.len() {
            return Err(ContentError::WrongChunkCount {
                expected: self.chunks.len(),
                actual: chunks.len(),
            });
        }
        let mut bytes = Vec::with_capacity(usize::try_from(self.length).unwrap_or(0));
        for (index, chunk) in chunks.iter().enumerate() {
            if usize::try_from(chunk.index).ok() != Some(index) {
                return Err(ContentError::InvalidIndex(chunk.index));
            }
            self.verify_chunk(chunk)?;
            bytes.extend_from_slice(&chunk.data);
        }
        if u64::try_from(bytes.len()).ok() != Some(self.length)
            || ContentId::digest(&bytes) != self.content_id
        {
            return Err(ContentError::InvalidContent);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_ids_are_deterministic() {
        let first = Manifest::from_bytes(b"repeatable content").unwrap().0;
        let second = Manifest::from_bytes(b"repeatable content").unwrap().0;
        assert_eq!(first, second);
        assert_eq!(first.content_id.to_string().len(), 64);
    }

    #[test]
    fn valid_chunks_reconstruct_exact_content() {
        let data = vec![42; CHUNK_SIZE + 17];
        let (manifest, chunks) = Manifest::from_bytes(&data).unwrap();
        assert_eq!(manifest.reconstruct(&chunks).unwrap(), data);
    }

    #[test]
    fn corrupted_chunks_are_rejected() {
        let (manifest, mut chunks) = Manifest::from_bytes(b"integrity matters").unwrap();
        chunks[0].data[0] ^= 0xff;
        assert!(matches!(
            manifest.verify_chunk(&chunks[0]),
            Err(ContentError::InvalidChunk(0))
        ));
    }

    #[test]
    fn empty_content_round_trips() {
        let (manifest, chunks) = Manifest::from_bytes(&[]).unwrap();
        assert!(chunks.is_empty());
        assert_eq!(manifest.reconstruct(&chunks).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn hostile_manifest_shape_is_rejected_before_transfer_allocation() {
        let content_id = ContentId::digest(b"shape");
        let invalid = Manifest {
            content_id,
            length: 1,
            chunk_size: 0,
            chunks: Vec::new(),
        };
        assert!(matches!(
            invalid.validate_shape(),
            Err(ContentError::InvalidChunkSize)
        ));

        let oversized = Manifest {
            content_id,
            length: MAX_CONTENT_LENGTH + 1,
            chunk_size: u32::try_from(CHUNK_SIZE).unwrap(),
            chunks: Vec::new(),
        };
        assert!(matches!(
            oversized.validate_shape(),
            Err(ContentError::ContentTooLarge)
        ));
    }
}
