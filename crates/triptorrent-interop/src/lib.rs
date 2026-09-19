//! No-I/O research helpers for M6 `BitTorrent` interoperability.
//!
//! This crate validates deterministic mapping conclusions. It is not a
//! production `.torrent` importer or a `BitTorrent` network implementation.

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::collections::BTreeMap;
use thiserror::Error;
use triptorrent_core::ContentId;
use unicode_normalization::UnicodeNormalization;

const MAX_BENCODE_DEPTH: usize = 64;
const V2_BLOCK_SIZE: usize = 16 * 1024;

/// A strictly parsed bencoded value with its byte range in the source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BencodeNode<'a> {
    /// Parsed value.
    pub value: BencodeValue<'a>,
    /// Inclusive source offset.
    pub start: usize,
    /// Exclusive source offset.
    pub end: usize,
}

/// Borrowing bencode representation used by the M6 mapping tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BencodeValue<'a> {
    /// Arbitrary byte string.
    Bytes(&'a [u8]),
    /// Prototype-sized integer.
    Integer(i64),
    /// Ordered list.
    List(Vec<BencodeNode<'a>>),
    /// Raw-byte-sorted dictionary.
    Dictionary(Vec<(&'a [u8], BencodeNode<'a>)>),
}

/// Strict bencoding failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BencodeError {
    /// Input ended before the current value did.
    #[error("truncated bencoding")]
    Truncated,
    /// A token or number was not canonical bencoding.
    #[error("invalid bencoding at byte {0}")]
    Invalid(usize),
    /// A dictionary key was duplicated or not strictly byte-sorted.
    #[error("dictionary keys are not strictly sorted at byte {0}")]
    UnsortedDictionary(usize),
    /// The input contains bytes after the root value.
    #[error("trailing bytes after bencoded value at byte {0}")]
    TrailingBytes(usize),
    /// Nesting exceeded the research parser's bound.
    #[error("bencoding exceeds maximum nesting depth")]
    TooDeep,
}

/// Strictly parses one complete bencoded value.
///
/// Dictionary keys must be strictly sorted and integers and lengths must use
/// their canonical representation. This makes hashing a validated raw `info`
/// slice safe, as required by BEP 3 and BEP 52.
///
/// # Errors
///
/// Returns [`BencodeError`] for malformed, non-canonical, trailing, or deeply
/// nested input.
pub fn parse_bencode(input: &[u8]) -> Result<BencodeNode<'_>, BencodeError> {
    let mut parser = Parser { input, position: 0 };
    let value = parser.parse_node(0)?;
    if parser.position != input.len() {
        return Err(BencodeError::TrailingBytes(parser.position));
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    fn parse_node(&mut self, depth: usize) -> Result<BencodeNode<'a>, BencodeError> {
        if depth > MAX_BENCODE_DEPTH {
            return Err(BencodeError::TooDeep);
        }
        let start = self.position;
        let token = *self
            .input
            .get(self.position)
            .ok_or(BencodeError::Truncated)?;
        let value = match token {
            b'i' => self.parse_integer()?,
            b'l' => self.parse_list(depth)?,
            b'd' => self.parse_dictionary(depth)?,
            b'0'..=b'9' => BencodeValue::Bytes(self.parse_bytes()?),
            _ => return Err(BencodeError::Invalid(self.position)),
        };
        Ok(BencodeNode {
            value,
            start,
            end: self.position,
        })
    }

    fn parse_integer(&mut self) -> Result<BencodeValue<'a>, BencodeError> {
        self.position += 1;
        let start = self.position;
        let end = self.find_terminator(b'e')?;
        let encoded = &self.input[start..end];
        if encoded.is_empty()
            || encoded == b"-0"
            || (encoded[0] == b'0' && encoded.len() > 1)
            || (encoded.starts_with(b"-0") && encoded.len() > 2)
            || (encoded[0] == b'-' && encoded.len() == 1)
            || !encoded
                .iter()
                .enumerate()
                .all(|(index, byte)| byte.is_ascii_digit() || (index == 0 && *byte == b'-'))
        {
            return Err(BencodeError::Invalid(start));
        }
        let text = std::str::from_utf8(encoded).map_err(|_| BencodeError::Invalid(start))?;
        let value = text
            .parse::<i64>()
            .map_err(|_| BencodeError::Invalid(start))?;
        self.position = end + 1;
        Ok(BencodeValue::Integer(value))
    }

    fn parse_bytes(&mut self) -> Result<&'a [u8], BencodeError> {
        let length_start = self.position;
        let colon = self.find_terminator(b':')?;
        let encoded = &self.input[length_start..colon];
        if encoded.is_empty()
            || (encoded[0] == b'0' && encoded.len() > 1)
            || !encoded.iter().all(u8::is_ascii_digit)
        {
            return Err(BencodeError::Invalid(length_start));
        }
        let length = std::str::from_utf8(encoded)
            .map_err(|_| BencodeError::Invalid(length_start))?
            .parse::<usize>()
            .map_err(|_| BencodeError::Invalid(length_start))?;
        let value_start = colon + 1;
        let value_end = value_start
            .checked_add(length)
            .filter(|end| *end <= self.input.len())
            .ok_or(BencodeError::Truncated)?;
        self.position = value_end;
        Ok(&self.input[value_start..value_end])
    }

    fn parse_list(&mut self, depth: usize) -> Result<BencodeValue<'a>, BencodeError> {
        self.position += 1;
        let mut values = Vec::new();
        while self.input.get(self.position) != Some(&b'e') {
            values.push(self.parse_node(depth + 1)?);
        }
        self.position += 1;
        Ok(BencodeValue::List(values))
    }

    fn parse_dictionary(&mut self, depth: usize) -> Result<BencodeValue<'a>, BencodeError> {
        self.position += 1;
        let mut entries = Vec::new();
        let mut previous: Option<&[u8]> = None;
        while self.input.get(self.position) != Some(&b'e') {
            let key_start = self.position;
            if !self
                .input
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                return Err(BencodeError::Invalid(self.position));
            }
            let key = self.parse_bytes()?;
            if previous.is_some_and(|candidate| candidate >= key) {
                return Err(BencodeError::UnsortedDictionary(key_start));
            }
            previous = Some(key);
            let value = self.parse_node(depth + 1)?;
            entries.push((key, value));
        }
        self.position += 1;
        Ok(BencodeValue::Dictionary(entries))
    }

    fn find_terminator(&self, token: u8) -> Result<usize, BencodeError> {
        self.input[self.position..]
            .iter()
            .position(|byte| *byte == token)
            .map(|relative| self.position + relative)
            .ok_or(BencodeError::Truncated)
    }
}

/// `BitTorrent` metadata generation represented by one metainfo file.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TorrentKind {
    /// BEP 3 metadata only.
    V1,
    /// BEP 52 metadata only.
    V2,
    /// One validated info dictionary containing both forms.
    Hybrid,
}

/// Namespaced `BitTorrent` aliases calculated from validated raw metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TorrentIdentity {
    /// Metadata generation.
    pub kind: TorrentKind,
    /// SHA-1 BEP 3 infohash, when v1 metadata is present.
    pub btih: Option<String>,
    /// Full SHA-256 BEP 52 infohash, when v2 metadata is present.
    pub btmh_sha256: Option<String>,
}

/// Invalid or unsupported research metainfo.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MetainfoError {
    /// Strict bencoding failed.
    #[error(transparent)]
    Bencode(#[from] BencodeError),
    /// The root or info value has the wrong type.
    #[error("metainfo must contain one dictionary-valued info key")]
    MissingInfo,
    /// Recognized fields do not form v1, v2, or hybrid metadata.
    #[error("info dictionary is neither supported v1 nor v2 metadata")]
    Unsupported,
    /// A recognized field is malformed.
    #[error("invalid metainfo field: {0}")]
    InvalidField(&'static str),
}

/// Calculates v1/v2 aliases from the exact validated `info` byte slice.
///
/// This intentionally does not calculate a `TripTorrent` identifier because
/// `BitTorrent` metadata is not proof that payload bytes are locally present.
///
/// # Errors
///
/// Returns [`MetainfoError`] for malformed bencoding or inconsistent minimum
/// v1/v2 structure.
pub fn torrent_identity(metainfo: &[u8]) -> Result<TorrentIdentity, MetainfoError> {
    let root = parse_bencode(metainfo)?;
    let root_entries = dictionary(&root).ok_or(MetainfoError::MissingInfo)?;
    let info = find(root_entries, b"info").ok_or(MetainfoError::MissingInfo)?;
    let info_entries = dictionary(info).ok_or(MetainfoError::MissingInfo)?;

    let piece_length = integer_field(info_entries, b"piece length")
        .filter(|length| *length > 0)
        .ok_or(MetainfoError::InvalidField("piece length"))?;
    let _ = piece_length;

    let v1_pieces = bytes_field(info_entries, b"pieces");
    let v1_shape = find(info_entries, b"length").is_some() ^ find(info_entries, b"files").is_some();
    let is_v1 =
        v1_pieces.is_some_and(|pieces| !pieces.is_empty() && pieces.len() % 20 == 0) && v1_shape;

    let meta_version = integer_field(info_entries, b"meta version");
    let is_v2 = meta_version == Some(2)
        && find(info_entries, b"file tree")
            .and_then(dictionary)
            .is_some();
    if meta_version.is_some() && meta_version != Some(2) {
        return Err(MetainfoError::InvalidField("meta version"));
    }
    if v1_pieces.is_some() && !is_v1 {
        return Err(MetainfoError::InvalidField("v1 pieces or file layout"));
    }
    if meta_version.is_some() && !is_v2 {
        return Err(MetainfoError::InvalidField("v2 file tree"));
    }

    let kind = match (is_v1, is_v2) {
        (true, false) => TorrentKind::V1,
        (false, true) => TorrentKind::V2,
        (true, true) => TorrentKind::Hybrid,
        (false, false) => return Err(MetainfoError::Unsupported),
    };
    let raw_info = &metainfo[info.start..info.end];
    Ok(TorrentIdentity {
        kind,
        btih: is_v1.then(|| hex::encode(Sha1::digest(raw_info))),
        btmh_sha256: is_v2.then(|| hex::encode(Sha256::digest(raw_info))),
    })
}

fn dictionary<'a>(node: &'a BencodeNode<'a>) -> Option<&'a [(&'a [u8], BencodeNode<'a>)]> {
    match &node.value {
        BencodeValue::Dictionary(entries) => Some(entries),
        _ => None,
    }
}

fn find<'a>(entries: &'a [(&'a [u8], BencodeNode<'a>)], key: &[u8]) -> Option<&'a BencodeNode<'a>> {
    entries
        .binary_search_by_key(&key, |(candidate, _)| *candidate)
        .ok()
        .map(|index| &entries[index].1)
}

fn bytes_field<'a>(entries: &'a [(&'a [u8], BencodeNode<'a>)], key: &[u8]) -> Option<&'a [u8]> {
    match &find(entries, key)?.value {
        BencodeValue::Bytes(bytes) => Some(bytes),
        _ => None,
    }
}

fn integer_field(entries: &[(&[u8], BencodeNode<'_>)], key: &[u8]) -> Option<i64> {
    match find(entries, key)?.value {
        BencodeValue::Integer(value) => Some(value),
        _ => None,
    }
}

/// Computes the current experimental `TripTorrent` identifier after payload
/// bytes have been acquired and verified.
#[must_use]
pub fn triptorrent_payload_id(payload: &[u8]) -> String {
    ContentId::digest(payload).to_string()
}

/// Computes a BEP 52 per-file Merkle root over 16 KiB SHA-256 leaves.
///
/// # Errors
///
/// Empty files have no `pieces root` and return [`MerkleError::EmptyFile`].
pub fn v2_file_root(payload: &[u8]) -> Result<[u8; 32], MerkleError> {
    if payload.is_empty() {
        return Err(MerkleError::EmptyFile);
    }
    let mut layer: Vec<[u8; 32]> = payload
        .chunks(V2_BLOCK_SIZE)
        .map(|block| Sha256::digest(block).into())
        .collect();
    let target = layer.len().next_power_of_two();
    layer.resize(target, [0_u8; 32]);
    while layer.len() > 1 {
        layer = layer
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                let mut hasher = Sha256::new();
                hasher.update(pair[0]);
                hasher.update(pair[1]);
                hasher.finalize().into()
            })
            .collect();
    }
    Ok(layer[0])
}

/// BEP 52 Merkle construction failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MerkleError {
    /// Empty v2 files omit the pieces root.
    #[error("empty v2 files do not have a pieces root")]
    EmptyFile,
}

/// Parsed compatibility hints from a magnet URI.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Magnet {
    /// Canonical lowercase BEP 3 SHA-1 aliases.
    pub btih: Vec<String>,
    /// Canonical lowercase full BEP 52 SHA-256 aliases.
    pub btmh_sha256: Vec<String>,
    /// Unrecognized exact topics preserved as hints.
    pub other_exact_topics: Vec<String>,
    /// Display names; they are never trusted as paths.
    pub display_names: Vec<String>,
    /// Classic tracker hints.
    pub trackers: Vec<String>,
    /// Classic direct-peer hints.
    pub explicit_peers: Vec<String>,
    /// Web-seed hints seen in deployed magnets.
    pub web_seeds: Vec<String>,
    /// Acceptable-source hints seen in deployed magnets.
    pub acceptable_sources: Vec<String>,
    /// Exact-source hints seen in deployed magnets.
    pub exact_sources: Vec<String>,
    /// Other query parameters, retaining repeated values.
    pub other: BTreeMap<String, Vec<String>>,
}

/// Magnet parsing failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MagnetError {
    /// URI is not a magnet query.
    #[error("URI is not a magnet link")]
    Scheme,
    /// Percent encoding is malformed or not UTF-8.
    #[error("invalid magnet percent encoding")]
    Encoding,
    /// No recognized or preserved exact topic is present.
    #[error("magnet link has no xt parameter")]
    MissingExactTopic,
    /// A recognized `BitTorrent` exact topic is malformed.
    #[error("invalid BitTorrent exact topic")]
    InvalidExactTopic,
}

/// Parses repeatable v1/v2/hybrid `BitTorrent` magnet hints without performing
/// discovery or assigning `TripTorrent` privacy semantics.
///
/// # Errors
///
/// Returns [`MagnetError`] for a wrong scheme, malformed escaping, missing
/// exact topic, or invalid recognized `BitTorrent` hash.
pub fn parse_magnet(uri: &str) -> Result<Magnet, MagnetError> {
    let query = uri.strip_prefix("magnet:?").ok_or(MagnetError::Scheme)?;
    let mut magnet = Magnet::default();
    let mut exact_topics = 0_usize;
    for parameter in query.split('&').filter(|part| !part.is_empty()) {
        let (raw_name, raw_value) = parameter.split_once('=').unwrap_or((parameter, ""));
        let name = percent_decode(raw_name)?;
        let value = percent_decode(raw_value)?;
        match name.as_str() {
            "xt" => {
                exact_topics += 1;
                parse_exact_topic(&value, &mut magnet)?;
            }
            "dn" => magnet.display_names.push(value),
            "tr" => magnet.trackers.push(value),
            "x.pe" => magnet.explicit_peers.push(value),
            "ws" => magnet.web_seeds.push(value),
            "as" => magnet.acceptable_sources.push(value),
            "xs" => magnet.exact_sources.push(value),
            _ => magnet.other.entry(name).or_default().push(value),
        }
    }
    if exact_topics == 0 {
        return Err(MagnetError::MissingExactTopic);
    }
    Ok(magnet)
}

fn parse_exact_topic(value: &str, magnet: &mut Magnet) -> Result<(), MagnetError> {
    let lower = value.to_ascii_lowercase();
    if let Some(encoded) = lower.strip_prefix("urn:btih:") {
        let bytes = if encoded.len() == 40 {
            hex::decode(encoded).map_err(|_| MagnetError::InvalidExactTopic)?
        } else if encoded.len() == 32 {
            decode_base32(encoded)?
        } else {
            return Err(MagnetError::InvalidExactTopic);
        };
        if bytes.len() != 20 {
            return Err(MagnetError::InvalidExactTopic);
        }
        magnet.btih.push(hex::encode(bytes));
    } else if let Some(encoded) = lower.strip_prefix("urn:btmh:") {
        let bytes = hex::decode(encoded).map_err(|_| MagnetError::InvalidExactTopic)?;
        if bytes.len() != 34 || bytes[..2] != [0x12, 0x20] {
            return Err(MagnetError::InvalidExactTopic);
        }
        magnet.btmh_sha256.push(hex::encode(&bytes[2..]));
    } else {
        magnet.other_exact_topics.push(value.to_owned());
    }
    Ok(())
}

fn percent_decode(value: &str) -> Result<String, MagnetError> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = *bytes.get(index + 1).ok_or(MagnetError::Encoding)?;
            let low = *bytes.get(index + 2).ok_or(MagnetError::Encoding)?;
            decoded.push((hex_digit(high)? << 4) | hex_digit(low)?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| MagnetError::Encoding)
}

fn hex_digit(value: u8) -> Result<u8, MagnetError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(MagnetError::Encoding),
    }
}

fn decode_base32(value: &str) -> Result<Vec<u8>, MagnetError> {
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    let mut decoded = Vec::with_capacity(value.len() * 5 / 8);
    for byte in value.bytes() {
        let symbol = match byte {
            b'a'..=b'z' => byte - b'a',
            b'2'..=b'7' => byte - b'2' + 26,
            _ => return Err(MagnetError::InvalidExactTopic),
        };
        accumulator = (accumulator << 5) | u32::from(symbol);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            decoded.push((accumulator >> bits).to_le_bytes()[0]);
            accumulator &= (1_u32 << bits).saturating_sub(1);
        }
    }
    if bits != 0 && accumulator != 0 {
        return Err(MagnetError::InvalidExactTopic);
    }
    Ok(decoded)
}

/// Unsafe torrent path component.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PathError {
    /// Empty, dot, or dot-dot component.
    #[error("empty and dot path components are forbidden")]
    Structural,
    /// Separators, control characters, or cross-platform forbidden characters.
    #[error("path component contains forbidden characters")]
    Character,
    /// Windows device name or trailing dot/space.
    #[error("path component is reserved on Windows")]
    WindowsReserved,
}

/// Normalizes one metadata path component to NFC and rejects traversal,
/// separators, controls, and Windows-reserved names.
///
/// Importers must additionally detect collisions between the returned sibling
/// names before creating files.
///
/// # Errors
///
/// Returns [`PathError`] when a component cannot be represented safely in the
/// cross-platform managed store.
pub fn sanitize_path_component(component: &str) -> Result<String, PathError> {
    let normalized: String = component.nfc().collect();
    if normalized.is_empty() || normalized == "." || normalized == ".." {
        return Err(PathError::Structural);
    }
    if normalized
        .chars()
        .any(|character| character.is_control() || "\\/:*?\"<>|".contains(character))
    {
        return Err(PathError::Character);
    }
    if normalized.ends_with([' ', '.']) || is_windows_device_name(&normalized) {
        return Err(PathError::WindowsReserved);
    }
    Ok(normalized)
}

fn is_windows_device_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Vectors {
        payload_hex: String,
        triptorrent_content_id: String,
        torrents: Vec<TorrentVector>,
        magnets: Vec<MagnetVector>,
        malformed_bencoding_hex: Vec<String>,
        paths: Vec<PathVector>,
    }

    #[derive(Deserialize)]
    struct TorrentVector {
        name: String,
        metainfo_hex: String,
        kind: TorrentKind,
        btih: Option<String>,
        btmh_sha256: Option<String>,
    }

    #[derive(Deserialize)]
    struct MagnetVector {
        uri: String,
        btih_count: usize,
        btmh_count: usize,
    }

    #[derive(Deserialize)]
    struct PathVector {
        input: String,
        safe: bool,
        normalized: Option<String>,
    }

    fn vectors() -> Vectors {
        serde_json::from_str(include_str!(
            "../../../test-vectors/m6-bittorrent-interop.json"
        ))
        .unwrap()
    }

    #[test]
    fn strict_bencoding_rejects_noncanonical_vectors() {
        for encoded in vectors().malformed_bencoding_hex {
            assert!(parse_bencode(&hex::decode(encoded).unwrap()).is_err());
        }
    }

    #[test]
    fn v1_v2_and_hybrid_identities_match_vectors() {
        for vector in vectors().torrents {
            let identity = torrent_identity(&hex::decode(&vector.metainfo_hex).unwrap())
                .unwrap_or_else(|error| panic!("{}: {error}", vector.name));
            assert_eq!(identity.kind, vector.kind, "{}", vector.name);
            assert_eq!(identity.btih, vector.btih, "{}", vector.name);
            assert_eq!(identity.btmh_sha256, vector.btmh_sha256, "{}", vector.name);
        }
    }

    #[test]
    fn same_payload_can_have_distinct_v1_identities() {
        let vectors = vectors();
        let first = &vectors.torrents[0];
        let renamed = &vectors.torrents[1];
        assert_ne!(first.btih, renamed.btih);
        let payload = hex::decode(vectors.payload_hex).unwrap();
        assert_eq!(
            triptorrent_payload_id(&payload),
            vectors.triptorrent_content_id
        );
    }

    #[test]
    fn magnets_preserve_multiple_namespaced_topics_and_hints() {
        for vector in vectors().magnets {
            let magnet = parse_magnet(&vector.uri).unwrap();
            assert_eq!(magnet.btih.len(), vector.btih_count);
            assert_eq!(magnet.btmh_sha256.len(), vector.btmh_count);
        }
    }

    #[test]
    fn unsafe_paths_are_rejected_and_unicode_is_normalized() {
        for vector in vectors().paths {
            let result = sanitize_path_component(&vector.input);
            assert_eq!(result.is_ok(), vector.safe, "{}", vector.input);
            if let Some(normalized) = vector.normalized {
                assert_eq!(result.unwrap(), normalized);
            }
        }
    }

    #[test]
    fn v2_merkle_root_uses_sixteen_kibibyte_leaves() {
        let payload = vec![0x5a; V2_BLOCK_SIZE + 1];
        let first = Sha256::digest(&payload[..V2_BLOCK_SIZE]);
        let second = Sha256::digest(&payload[V2_BLOCK_SIZE..]);
        let mut expected = Sha256::new();
        expected.update(first);
        expected.update(second);
        assert_eq!(
            v2_file_root(&payload).unwrap(),
            <[u8; 32]>::from(expected.finalize())
        );
        assert_eq!(v2_file_root(&[]), Err(MerkleError::EmptyFile));
    }
}
