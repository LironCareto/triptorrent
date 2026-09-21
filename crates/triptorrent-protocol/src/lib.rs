#![doc = "Normative `TripTorrent` Testnet v1 wire messages and binary codec."]

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use thiserror::Error;
use triptorrent_core::{
    CHUNK_SIZE, Chunk, ChunkId, ContentId, Manifest, NETWORK_ID, PROTOCOL_VERSION, PeerId,
};

/// Maximum plaintext application record accepted by the Noise transport.
pub const MAX_APPLICATION_MESSAGE_BYTES: usize = 65_519;
pub const MAX_OVERLAY_MESSAGE_BYTES: usize = 256 * 1024;
pub const MAX_ADVERTISED_CONTENT_IDS: usize = 256;
pub const WIRE_MAGIC: &[u8; 4] = b"TTP1";
const MAX_ERROR_TEXT_BYTES: usize = 1024;
const MAX_CAPABILITIES: usize = 8;
const MAX_DISCOVERY_ROUTES: usize = 16;
/// Largest manifest that fits a canonical Testnet v1 swarm-manifest record.
pub const MAX_TESTNET_MANIFEST_CHUNKS: usize = 2_036;
const DOMAIN_APPLICATION: u8 = 1;
const DOMAIN_OVERLAY_REQUEST: u8 = 2;
const DOMAIN_OVERLAY_RESPONSE: u8 = 3;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Message {
    Request {
        content_id: ContentId,
    },
    Manifest(Manifest),
    Chunk(Chunk),
    Complete,
    Error(String),
    SwarmRequest {
        content_id: ContentId,
    },
    SwarmManifest {
        manifest: Manifest,
        availability: PieceAvailability,
    },
    ChunkRequest {
        index: u32,
    },
    ChunkUnavailable {
        index: u32,
    },
    SwarmComplete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PieceAvailability {
    pub chunk_count: u32,
    pub bits: Vec<u8>,
}

impl PieceAvailability {
    #[must_use]
    pub fn all(chunk_count: u32) -> Self {
        let mut value = Self {
            chunk_count,
            bits: vec![0xff; usize::try_from(chunk_count.div_ceil(8)).unwrap_or(usize::MAX)],
        };
        value.clear_unused_bits();
        value
    }
    #[must_use]
    pub fn from_indices(chunk_count: u32, indices: impl IntoIterator<Item = u32>) -> Self {
        let mut value = Self {
            chunk_count,
            bits: vec![0; usize::try_from(chunk_count.div_ceil(8)).unwrap_or(usize::MAX)],
        };
        for index in indices {
            if index < chunk_count {
                value.bits[usize::try_from(index / 8).unwrap_or(usize::MAX)] |= 1 << (index % 8);
            }
        }
        value
    }
    #[must_use]
    pub fn contains(&self, index: u32) -> bool {
        self.is_valid()
            && index < self.chunk_count
            && self.bits[usize::try_from(index / 8).unwrap_or(usize::MAX)] & (1 << (index % 8)) != 0
    }
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PeerAdvertisement {
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
    pub capabilities: Vec<PeerCapability>,
    pub content_ids: Vec<ContentId>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PeerCapability {
    RelayedTransferV1,
    SwarmTransferV1,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RelayAdvertisement {
    pub relay_id: String,
    pub address: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteAssignment {
    pub provider_id: PeerId,
    pub provider_public_key: [u8; 32],
    pub relay_id: String,
    pub relay_address: String,
    pub route: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OverlayRequest {
    RegisterPeer(PeerAdvertisement),
    HeartbeatPeer { peer_id: PeerId },
    PollPeer { peer_id: PeerId },
    RegisterRelay(RelayAdvertisement),
    HeartbeatRelay { relay_id: String },
    Discover { content_id: ContentId },
    DiscoverProviders { content_id: ContentId, limit: u16 },
    ReportRelayFailure { relay_id: String },
    Health,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OverlayResponse {
    Registered { lease_ms: u64 },
    Assignment(Option<RouteAssignment>),
    Route(Option<RouteAssignment>),
    Routes(Vec<RouteAssignment>),
    Acknowledged,
    Error(String),
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ProtocolError {
    #[error("malformed testnet wire message: {0}")]
    Malformed(&'static str),
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u16),
    #[error("wrong network {0}")]
    WrongNetwork(String),
    #[error("unsupported required capability {0}")]
    UnsupportedCapability(u16),
    #[error("unknown message type {0}")]
    UnknownMessage(u8),
    #[error("invalid protocol message: {0}")]
    InvalidMessage(&'static str),
    #[error("encoded protocol message exceeds {limit} bytes")]
    MessageTooLarge { limit: usize },
}

/// Encodes one canonical Testnet v1 application record.
///
/// # Errors
/// Returns an error when the message violates bounds or cannot be represented.
pub fn encode(message: Message) -> Result<Vec<u8>, ProtocolError> {
    validate_message(&message)?;
    let mut w = Writer::new(DOMAIN_APPLICATION);
    match message {
        Message::Request { content_id } => {
            w.u8(1);
            w.content_id(content_id);
        }
        Message::Manifest(value) => {
            w.u8(2);
            w.manifest(&value)?;
        }
        Message::Chunk(value) => {
            w.u8(3);
            w.chunk(&value)?;
        }
        Message::Complete => w.u8(4),
        Message::Error(value) => {
            w.u8(5);
            w.string(&value)?;
        }
        Message::SwarmRequest { content_id } => {
            w.u8(16);
            w.content_id(content_id);
        }
        Message::SwarmManifest {
            manifest,
            availability,
        } => {
            w.u8(17);
            w.manifest(&manifest)?;
            w.u32(availability.chunk_count);
            w.bytes(&availability.bits)?;
        }
        Message::ChunkRequest { index } => {
            w.u8(18);
            w.u32(index);
        }
        Message::ChunkUnavailable { index } => {
            w.u8(19);
            w.u32(index);
        }
        Message::SwarmComplete => w.u8(20),
    }
    w.finish(MAX_APPLICATION_MESSAGE_BYTES)
}

/// Decodes one complete Testnet v1 application record.
///
/// # Errors
/// Returns an error for malformed, incompatible, unknown, or invalid input.
pub fn decode(bytes: &[u8]) -> Result<Message, ProtocolError> {
    ensure_size(bytes, MAX_APPLICATION_MESSAGE_BYTES)?;
    let mut r = Reader::new(bytes, DOMAIN_APPLICATION)?;
    let value = match r.u8()? {
        1 => Message::Request {
            content_id: r.content_id()?,
        },
        2 => Message::Manifest(r.manifest()?),
        3 => Message::Chunk(r.chunk()?),
        4 => Message::Complete,
        5 => Message::Error(r.string(MAX_ERROR_TEXT_BYTES)?),
        16 => Message::SwarmRequest {
            content_id: r.content_id()?,
        },
        17 => Message::SwarmManifest {
            manifest: r.manifest()?,
            availability: PieceAvailability {
                chunk_count: r.u32()?,
                bits: r.bytes(MAX_APPLICATION_MESSAGE_BYTES)?,
            },
        },
        18 => Message::ChunkRequest { index: r.u32()? },
        19 => Message::ChunkUnavailable { index: r.u32()? },
        20 => Message::SwarmComplete,
        kind => return Err(ProtocolError::UnknownMessage(kind)),
    };
    r.end()?;
    validate_message(&value)?;
    Ok(value)
}

/// Encodes one canonical testnet discovery request.
///
/// # Errors
/// Returns an error when the request violates bounds or cannot be represented.
pub fn encode_overlay_request(request: OverlayRequest) -> Result<Vec<u8>, ProtocolError> {
    validate_overlay_request(&request)?;
    let mut w = Writer::new(DOMAIN_OVERLAY_REQUEST);
    match request {
        OverlayRequest::RegisterPeer(peer) => {
            w.u8(1);
            w.peer_id(peer.peer_id);
            w.raw(&peer.public_key);
            w.u8(u8::try_from(peer.capabilities.len())
                .map_err(|_| ProtocolError::InvalidMessage("too many capabilities"))?);
            for capability in peer.capabilities {
                w.u16(match capability {
                    PeerCapability::RelayedTransferV1 => 1,
                    PeerCapability::SwarmTransferV1 => 2,
                });
            }
            w.u16(
                u16::try_from(peer.content_ids.len())
                    .map_err(|_| ProtocolError::InvalidMessage("too many content identifiers"))?,
            );
            for id in peer.content_ids {
                w.content_id(id);
            }
        }
        OverlayRequest::HeartbeatPeer { peer_id } => {
            w.u8(2);
            w.peer_id(peer_id);
        }
        OverlayRequest::PollPeer { peer_id } => {
            w.u8(3);
            w.peer_id(peer_id);
        }
        OverlayRequest::RegisterRelay(value) => {
            w.u8(4);
            w.string(&value.relay_id)?;
            w.string(&value.address)?;
        }
        OverlayRequest::HeartbeatRelay { relay_id } => {
            w.u8(5);
            w.string(&relay_id)?;
        }
        OverlayRequest::Discover { content_id } => {
            w.u8(6);
            w.content_id(content_id);
        }
        OverlayRequest::DiscoverProviders { content_id, limit } => {
            w.u8(7);
            w.content_id(content_id);
            w.u16(limit);
        }
        OverlayRequest::ReportRelayFailure { relay_id } => {
            w.u8(8);
            w.string(&relay_id)?;
        }
        OverlayRequest::Health => w.u8(9),
    }
    w.finish(MAX_OVERLAY_MESSAGE_BYTES)
}

/// Decodes one complete testnet discovery request.
///
/// # Errors
/// Returns an error for malformed, incompatible, unknown, or invalid input.
pub fn decode_overlay_request(bytes: &[u8]) -> Result<OverlayRequest, ProtocolError> {
    ensure_size(bytes, MAX_OVERLAY_MESSAGE_BYTES)?;
    let mut r = Reader::new(bytes, DOMAIN_OVERLAY_REQUEST)?;
    let value = match r.u8()? {
        1 => {
            let peer_id = r.peer_id()?;
            let public_key = r.array()?;
            let count = usize::from(r.u8()?);
            if count > MAX_CAPABILITIES {
                return Err(ProtocolError::InvalidMessage("too many capabilities"));
            }
            let mut capabilities = Vec::with_capacity(count);
            for _ in 0..count {
                capabilities.push(match r.u16()? {
                    1 => PeerCapability::RelayedTransferV1,
                    2 => PeerCapability::SwarmTransferV1,
                    code => return Err(ProtocolError::UnsupportedCapability(code)),
                });
            }
            let count = usize::from(r.u16()?);
            if count > MAX_ADVERTISED_CONTENT_IDS {
                return Err(ProtocolError::InvalidMessage(
                    "too many content identifiers",
                ));
            }
            let mut content_ids = Vec::with_capacity(count);
            for _ in 0..count {
                content_ids.push(r.content_id()?);
            }
            OverlayRequest::RegisterPeer(PeerAdvertisement {
                peer_id,
                public_key,
                capabilities,
                content_ids,
            })
        }
        2 => OverlayRequest::HeartbeatPeer {
            peer_id: r.peer_id()?,
        },
        3 => OverlayRequest::PollPeer {
            peer_id: r.peer_id()?,
        },
        4 => OverlayRequest::RegisterRelay(RelayAdvertisement {
            relay_id: r.string(64)?,
            address: r.string(128)?,
        }),
        5 => OverlayRequest::HeartbeatRelay {
            relay_id: r.string(64)?,
        },
        6 => OverlayRequest::Discover {
            content_id: r.content_id()?,
        },
        7 => OverlayRequest::DiscoverProviders {
            content_id: r.content_id()?,
            limit: r.u16()?,
        },
        8 => OverlayRequest::ReportRelayFailure {
            relay_id: r.string(64)?,
        },
        9 => OverlayRequest::Health,
        kind => return Err(ProtocolError::UnknownMessage(kind)),
    };
    r.end()?;
    validate_overlay_request(&value)?;
    Ok(value)
}

/// Encodes one canonical testnet discovery response.
///
/// # Errors
/// Returns an error when the response violates bounds or cannot be represented.
pub fn encode_overlay_response(response: OverlayResponse) -> Result<Vec<u8>, ProtocolError> {
    validate_overlay_response(&response)?;
    let mut w = Writer::new(DOMAIN_OVERLAY_RESPONSE);
    match response {
        OverlayResponse::Registered { lease_ms } => {
            w.u8(1);
            w.u64(lease_ms);
        }
        OverlayResponse::Assignment(value) => {
            w.u8(2);
            w.optional_assignment(value.as_ref())?;
        }
        OverlayResponse::Route(value) => {
            w.u8(3);
            w.optional_assignment(value.as_ref())?;
        }
        OverlayResponse::Routes(values) => {
            w.u8(4);
            w.u16(
                u16::try_from(values.len())
                    .map_err(|_| ProtocolError::InvalidMessage("too many routes"))?,
            );
            for value in &values {
                w.assignment(value)?;
            }
        }
        OverlayResponse::Acknowledged => w.u8(5),
        OverlayResponse::Error(value) => {
            w.u8(255);
            w.string(&value)?;
        }
    }
    w.finish(MAX_OVERLAY_MESSAGE_BYTES)
}

/// Decodes one complete testnet discovery response.
///
/// # Errors
/// Returns an error for malformed, incompatible, unknown, or invalid input.
pub fn decode_overlay_response(bytes: &[u8]) -> Result<OverlayResponse, ProtocolError> {
    ensure_size(bytes, MAX_OVERLAY_MESSAGE_BYTES)?;
    let mut r = Reader::new(bytes, DOMAIN_OVERLAY_RESPONSE)?;
    let value = match r.u8()? {
        1 => OverlayResponse::Registered { lease_ms: r.u64()? },
        2 => OverlayResponse::Assignment(r.optional_assignment()?),
        3 => OverlayResponse::Route(r.optional_assignment()?),
        4 => {
            let count = usize::from(r.u16()?);
            if count > MAX_DISCOVERY_ROUTES {
                return Err(ProtocolError::InvalidMessage("too many route assignments"));
            }
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                values.push(r.assignment()?);
            }
            OverlayResponse::Routes(values)
        }
        5 => OverlayResponse::Acknowledged,
        255 => OverlayResponse::Error(r.string(MAX_ERROR_TEXT_BYTES)?),
        kind => return Err(ProtocolError::UnknownMessage(kind)),
    };
    r.end()?;
    validate_overlay_response(&value)?;
    Ok(value)
}

fn validate_message(value: &Message) -> Result<(), ProtocolError> {
    match value {
        Message::Manifest(value) => validate_manifest(value),
        Message::Chunk(value) if value.data.len() > CHUNK_SIZE => Err(
            ProtocolError::InvalidMessage("chunk payload exceeds fixed chunk size"),
        ),
        Message::Error(value) if value.len() > MAX_ERROR_TEXT_BYTES => {
            Err(ProtocolError::InvalidMessage("error text exceeds bound"))
        }
        Message::SwarmManifest {
            manifest,
            availability,
        } => {
            validate_manifest(manifest)?;
            if availability.chunk_count != u32::try_from(manifest.chunks.len()).unwrap_or(u32::MAX)
                || !availability.is_valid()
            {
                return Err(ProtocolError::InvalidMessage(
                    "piece availability does not match manifest",
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
fn validate_manifest(value: &Manifest) -> Result<(), ProtocolError> {
    value
        .validate_shape()
        .map_err(|_| ProtocolError::InvalidMessage("malformed manifest shape"))?;
    if value.chunks.len() > MAX_TESTNET_MANIFEST_CHUNKS {
        return Err(ProtocolError::InvalidMessage(
            "manifest exceeds Testnet v1 record bound",
        ));
    }
    Ok(())
}
fn valid_identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
fn validate_assignment(value: &RouteAssignment) -> Result<(), ProtocolError> {
    if !valid_identifier(&value.relay_id, 64)
        || value.relay_address.len() > 128
        || value.relay_address.parse::<SocketAddr>().is_err()
        || !valid_identifier(&value.route, 128)
    {
        Err(ProtocolError::InvalidMessage(
            "invalid route assignment fields",
        ))
    } else {
        Ok(())
    }
}
fn validate_overlay_request(value: &OverlayRequest) -> Result<(), ProtocolError> {
    match value {
        OverlayRequest::RegisterPeer(peer) => {
            if peer.peer_id != PeerId::from_public_key(&peer.public_key) {
                return Err(ProtocolError::InvalidMessage(
                    "peer identity does not match key",
                ));
            }
            if peer.content_ids.is_empty()
                || peer.content_ids.len() > MAX_ADVERTISED_CONTENT_IDS
                || peer.capabilities.is_empty()
                || peer.capabilities.len() > MAX_CAPABILITIES
            {
                return Err(ProtocolError::InvalidMessage(
                    "peer advertisement exceeds bounds",
                ));
            }
            Ok(())
        }
        OverlayRequest::RegisterRelay(value) => {
            if !valid_identifier(&value.relay_id, 64)
                || value.address.len() > 128
                || value.address.parse::<SocketAddr>().is_err()
            {
                Err(ProtocolError::InvalidMessage("invalid relay advertisement"))
            } else {
                Ok(())
            }
        }
        OverlayRequest::HeartbeatRelay { relay_id }
        | OverlayRequest::ReportRelayFailure { relay_id } => {
            if valid_identifier(relay_id, 64) {
                Ok(())
            } else {
                Err(ProtocolError::InvalidMessage("invalid relay identifier"))
            }
        }
        OverlayRequest::DiscoverProviders { limit, .. }
            if *limit == 0 || usize::from(*limit) > MAX_DISCOVERY_ROUTES =>
        {
            Err(ProtocolError::InvalidMessage(
                "invalid provider discovery limit",
            ))
        }
        _ => Ok(()),
    }
}
fn validate_overlay_response(value: &OverlayResponse) -> Result<(), ProtocolError> {
    match value {
        OverlayResponse::Assignment(Some(value)) | OverlayResponse::Route(Some(value)) => {
            validate_assignment(value)
        }
        OverlayResponse::Routes(values) => {
            if values.len() > MAX_DISCOVERY_ROUTES {
                return Err(ProtocolError::InvalidMessage("too many route assignments"));
            }
            for value in values {
                validate_assignment(value)?;
            }
            Ok(())
        }
        OverlayResponse::Error(value) if value.len() > MAX_ERROR_TEXT_BYTES => {
            Err(ProtocolError::InvalidMessage("error text exceeds bound"))
        }
        _ => Ok(()),
    }
}
fn ensure_size(bytes: &[u8], limit: usize) -> Result<(), ProtocolError> {
    if bytes.len() > limit {
        Err(ProtocolError::MessageTooLarge { limit })
    } else {
        Ok(())
    }
}

struct Writer {
    bytes: Vec<u8>,
}
impl Writer {
    fn new(domain: u8) -> Self {
        let mut value = Self { bytes: Vec::new() };
        value.raw(WIRE_MAGIC);
        value.u16(PROTOCOL_VERSION);
        value.u8(u8::try_from(NETWORK_ID.len()).expect("network ID fits u8"));
        value.raw(NETWORK_ID.as_bytes());
        value.u8(domain);
        value
    }
    fn finish(self, limit: usize) -> Result<Vec<u8>, ProtocolError> {
        ensure_size(&self.bytes, limit)?;
        Ok(self.bytes)
    }
    fn raw(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.raw(&value.to_be_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.raw(&value.to_be_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.raw(&value.to_be_bytes());
    }
    fn bytes(&mut self, value: &[u8]) -> Result<(), ProtocolError> {
        self.u32(
            u32::try_from(value.len())
                .map_err(|_| ProtocolError::Malformed("byte string too long"))?,
        );
        self.raw(value);
        Ok(())
    }
    fn string(&mut self, value: &str) -> Result<(), ProtocolError> {
        self.u16(
            u16::try_from(value.len()).map_err(|_| ProtocolError::Malformed("string too long"))?,
        );
        self.raw(value.as_bytes());
        Ok(())
    }
    fn content_id(&mut self, value: ContentId) {
        self.raw(value.as_bytes());
    }
    fn peer_id(&mut self, value: PeerId) {
        self.raw(value.as_bytes());
    }
    fn manifest(&mut self, value: &Manifest) -> Result<(), ProtocolError> {
        self.content_id(value.content_id);
        self.u64(value.length);
        self.u32(value.chunk_size);
        self.u32(
            u32::try_from(value.chunks.len())
                .map_err(|_| ProtocolError::InvalidMessage("too many manifest chunks"))?,
        );
        for id in &value.chunks {
            self.raw(id.as_bytes());
        }
        Ok(())
    }
    fn chunk(&mut self, value: &Chunk) -> Result<(), ProtocolError> {
        self.u32(value.index);
        self.raw(value.id.as_bytes());
        self.bytes(&value.data)
    }
    fn assignment(&mut self, value: &RouteAssignment) -> Result<(), ProtocolError> {
        self.peer_id(value.provider_id);
        self.raw(&value.provider_public_key);
        self.string(&value.relay_id)?;
        self.string(&value.relay_address)?;
        self.string(&value.route)
    }
    fn optional_assignment(
        &mut self,
        value: Option<&RouteAssignment>,
    ) -> Result<(), ProtocolError> {
        if let Some(value) = value {
            self.u8(1);
            self.assignment(value)
        } else {
            self.u8(0);
            Ok(())
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], domain: u8) -> Result<Self, ProtocolError> {
        let mut value = Self { bytes, offset: 0 };
        if value.take(4)? != WIRE_MAGIC {
            return Err(ProtocolError::Malformed("invalid magic"));
        }
        let version = value.u16()?;
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(version));
        }
        let length = usize::from(value.u8()?);
        let network = std::str::from_utf8(value.take(length)?)
            .map_err(|_| ProtocolError::Malformed("network identifier is not UTF-8"))?;
        if network != NETWORK_ID {
            return Err(ProtocolError::WrongNetwork(network.to_owned()));
        }
        if value.u8()? != domain {
            return Err(ProtocolError::Malformed("wrong message domain"));
        }
        Ok(value)
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(ProtocolError::Malformed("length overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(ProtocolError::Malformed("truncated input"))?;
        self.offset = end;
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        self.take(N)?
            .try_into()
            .map_err(|_| ProtocolError::Malformed("truncated fixed field"))
    }
    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ProtocolError> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, ProtocolError> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, ProtocolError> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn bytes(&mut self, maximum: usize) -> Result<Vec<u8>, ProtocolError> {
        let length = usize::try_from(self.u32()?)
            .map_err(|_| ProtocolError::Malformed("invalid byte length"))?;
        if length > maximum {
            return Err(ProtocolError::InvalidMessage("byte string exceeds bound"));
        }
        Ok(self.take(length)?.to_vec())
    }
    fn string(&mut self, maximum: usize) -> Result<String, ProtocolError> {
        let length = usize::from(self.u16()?);
        if length > maximum {
            return Err(ProtocolError::InvalidMessage("string exceeds bound"));
        }
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| ProtocolError::Malformed("string is not UTF-8"))
    }
    fn content_id(&mut self) -> Result<ContentId, ProtocolError> {
        Ok(ContentId::from_bytes(self.array()?))
    }
    fn peer_id(&mut self) -> Result<PeerId, ProtocolError> {
        Ok(PeerId::from_bytes(self.array()?))
    }
    fn manifest(&mut self) -> Result<Manifest, ProtocolError> {
        let content_id = self.content_id()?;
        let length = self.u64()?;
        let chunk_size = self.u32()?;
        let count = usize::try_from(self.u32()?)
            .map_err(|_| ProtocolError::Malformed("invalid chunk count"))?;
        if count > MAX_TESTNET_MANIFEST_CHUNKS {
            return Err(ProtocolError::InvalidMessage("too many manifest chunks"));
        }
        let mut chunks = Vec::with_capacity(count);
        for _ in 0..count {
            chunks.push(ChunkId::from_bytes(self.array()?));
        }
        Ok(Manifest {
            content_id,
            length,
            chunk_size,
            chunks,
        })
    }
    fn chunk(&mut self) -> Result<Chunk, ProtocolError> {
        Ok(Chunk {
            index: self.u32()?,
            id: ChunkId::from_bytes(self.array()?),
            data: self.bytes(CHUNK_SIZE)?,
        })
    }
    fn assignment(&mut self) -> Result<RouteAssignment, ProtocolError> {
        Ok(RouteAssignment {
            provider_id: self.peer_id()?,
            provider_public_key: self.array()?,
            relay_id: self.string(64)?,
            relay_address: self.string(128)?,
            route: self.string(128)?,
        })
    }
    fn optional_assignment(&mut self) -> Result<Option<RouteAssignment>, ProtocolError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.assignment()?)),
            _ => Err(ProtocolError::Malformed("invalid optional tag")),
        }
    }
    fn end(&self) -> Result<(), ProtocolError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(ProtocolError::Malformed("trailing bytes"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn messages_round_trip() {
        let id = ContentId::digest(b"testnet-v1");
        for message in [
            Message::SwarmRequest { content_id: id },
            Message::ChunkRequest { index: 7 },
            Message::SwarmComplete,
        ] {
            assert_eq!(decode(&encode(message.clone()).unwrap()).unwrap(), message);
        }
    }
    #[test]
    fn overlay_round_trip() {
        let key = [7; 32];
        let value = OverlayRequest::RegisterPeer(PeerAdvertisement {
            peer_id: PeerId::from_public_key(&key),
            public_key: key,
            capabilities: vec![PeerCapability::SwarmTransferV1],
            content_ids: vec![ContentId::digest(b"x")],
        });
        assert_eq!(
            decode_overlay_request(&encode_overlay_request(value.clone()).unwrap()).unwrap(),
            value
        );
    }
    #[test]
    fn negotiation_fails_closed() {
        let value = encode(Message::Complete).unwrap();
        let mut version = value.clone();
        version[4..6].copy_from_slice(&2_u16.to_be_bytes());
        assert_eq!(decode(&version), Err(ProtocolError::UnsupportedVersion(2)));
        let mut network = value;
        network[7] ^= 1;
        assert!(matches!(
            decode(&network),
            Err(ProtocolError::WrongNetwork(_))
        ));
        assert!(decode(&[0, 1]).is_err());
    }
    #[test]
    fn unknown_and_trailing_fail_closed() {
        let mut value = encode(Message::Complete).unwrap();
        let last = value.len() - 1;
        value[last] = 254;
        assert_eq!(decode(&value), Err(ProtocolError::UnknownMessage(254)));
        let mut value = encode(Message::Complete).unwrap();
        value.push(0);
        assert!(decode(&value).is_err());
    }
    #[test]
    fn availability_is_canonical() {
        let value = PieceAvailability::from_indices(10, [0, 3, 9]);
        assert!(value.is_valid() && value.contains(9));
        assert!(
            !PieceAvailability {
                chunk_count: 9,
                bits: vec![0, 0x80]
            }
            .is_valid()
        );
    }
}
