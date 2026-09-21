use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use triptorrent_core::{ContentId, Manifest, PeerId};
use triptorrent_protocol::{
    Message, OverlayRequest, OverlayResponse, PeerAdvertisement, PeerCapability,
    RelayAdvertisement, RouteAssignment, decode, decode_overlay_request, decode_overlay_response,
    encode, encode_overlay_request, encode_overlay_response,
};

#[derive(Deserialize)]
struct Suite {
    protocol_version: u16,
    network: String,
    canonical_artifact: Artifact,
    positive: Vec<Vector>,
    negative: Vec<Vector>,
}
#[derive(Deserialize)]
struct Artifact {
    path: String,
    size: usize,
    content_id: String,
    sha256: String,
    chunk_size: u32,
    chunk_count: usize,
}
#[derive(Deserialize)]
struct Vector {
    name: String,
    surface: String,
    hex: Option<String>,
    expect: String,
    sequence: Option<Vec<String>>,
    declared_length: Option<usize>,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn load() -> Suite {
    serde_json::from_slice(&fs::read(root().join("test-vectors/m9-testnet-v1.json")).unwrap())
        .unwrap()
}

fn assignment() -> RouteAssignment {
    let key = [7; 32];
    RouteAssignment {
        provider_id: PeerId::from_public_key(&key),
        provider_public_key: key,
        relay_id: "relay-a".into(),
        relay_address: "127.0.0.1:7001".into(),
        route: "m9-route".into(),
    }
}

fn encoded_values(
    manifest: &Manifest,
    chunks: &[triptorrent_core::Chunk],
) -> Vec<(&'static str, Vec<u8>)> {
    let key = [7; 32];
    vec![
        (
            "swarm-request",
            encode(Message::SwarmRequest {
                content_id: manifest.content_id,
            })
            .unwrap(),
        ),
        (
            "swarm-manifest",
            encode(Message::SwarmManifest {
                manifest: manifest.clone(),
                availability: triptorrent_protocol::PieceAvailability::all(1),
            })
            .unwrap(),
        ),
        (
            "chunk-request",
            encode(Message::ChunkRequest { index: 0 }).unwrap(),
        ),
        (
            "chunk-response",
            encode(Message::Chunk(chunks[0].clone())).unwrap(),
        ),
        (
            "unavailable",
            encode(Message::ChunkUnavailable { index: 0 }).unwrap(),
        ),
        ("completion", encode(Message::SwarmComplete).unwrap()),
        (
            "structured-error",
            encode(Message::Error("test error".into())).unwrap(),
        ),
        (
            "peer-registration",
            encode_overlay_request(OverlayRequest::RegisterPeer(PeerAdvertisement {
                peer_id: PeerId::from_public_key(&key),
                public_key: key,
                capabilities: vec![PeerCapability::SwarmTransferV1],
                content_ids: vec![manifest.content_id],
            }))
            .unwrap(),
        ),
        (
            "relay-advertisement",
            encode_overlay_request(OverlayRequest::RegisterRelay(RelayAdvertisement {
                relay_id: "relay-a".into(),
                address: "127.0.0.1:7001".into(),
            }))
            .unwrap(),
        ),
        (
            "provider-discovery",
            encode_overlay_request(OverlayRequest::DiscoverProviders {
                content_id: manifest.content_id,
                limit: 1,
            })
            .unwrap(),
        ),
        (
            "bootstrap-health",
            encode_overlay_request(OverlayRequest::Health).unwrap(),
        ),
        (
            "route-assignment",
            encode_overlay_response(OverlayResponse::Routes(vec![assignment()])).unwrap(),
        ),
        ("relay-registration", relay_registration("m9-route")),
    ]
}

fn relay_registration(route: &str) -> Vec<u8> {
    let mut value = Vec::new();
    value.extend_from_slice(b"TTR1");
    value.extend_from_slice(&triptorrent_core::PROTOCOL_VERSION.to_be_bytes());
    value.push(u8::try_from(triptorrent_core::NETWORK_ID.len()).unwrap());
    value.extend_from_slice(triptorrent_core::NETWORK_ID.as_bytes());
    value.push(1);
    value.extend_from_slice(&u16::try_from(route.len()).unwrap().to_be_bytes());
    value.extend_from_slice(route.as_bytes());
    value
}

#[test]
fn rust_consumes_checked_in_testnet_v1_vectors() {
    let suite = load();
    assert_eq!(suite.protocol_version, triptorrent_core::PROTOCOL_VERSION);
    assert_eq!(suite.network, triptorrent_core::NETWORK_ID);
    let payload = fs::read(root().join(&suite.canonical_artifact.path)).unwrap();
    assert_eq!(payload.len(), suite.canonical_artifact.size);
    assert_eq!(
        ContentId::digest(&payload).to_string(),
        suite.canonical_artifact.content_id
    );
    assert_eq!(suite.canonical_artifact.chunk_size, 32_768);
    let (manifest, chunks) = Manifest::from_bytes(&payload).unwrap();
    assert_eq!(chunks.len(), suite.canonical_artifact.chunk_count);
    assert_eq!(
        hex::encode(Sha256::digest(&payload)),
        suite.canonical_artifact.sha256
    );
    let encoded = encoded_values(&manifest, &chunks);
    assert_eq!(encoded.len(), suite.positive.len());
    for (name, encoded) in encoded {
        let vector = suite
            .positive
            .iter()
            .find(|value| value.name == name)
            .unwrap();
        assert_eq!(
            hex::encode(encoded),
            vector.hex.as_deref().unwrap(),
            "{name}"
        );
    }
    for vector in &suite.positive {
        let bytes = hex::decode(vector.hex.as_deref().unwrap()).unwrap();
        match vector.surface.as_str() {
            "application" => assert!(decode(&bytes).is_ok(), "{}", vector.name),
            "overlay_request" => assert!(decode_overlay_request(&bytes).is_ok(), "{}", vector.name),
            "overlay_response" => {
                assert!(decode_overlay_response(&bytes).is_ok(), "{}", vector.name);
            }
            "relay_registration" => {}
            other => panic!("unknown surface {other}"),
        }
    }
    for vector in &suite.negative {
        if vector.surface == "application" {
            assert!(
                decode(&hex::decode(vector.hex.as_deref().unwrap()).unwrap()).is_err(),
                "{}",
                vector.name
            );
        } else if vector.surface == "overlay_request" {
            assert!(
                decode_overlay_request(&hex::decode(vector.hex.as_deref().unwrap()).unwrap())
                    .is_err(),
                "{}",
                vector.name
            );
        } else if vector.surface == "state" {
            assert_ne!(
                vector
                    .sequence
                    .as_deref()
                    .and_then(|value| value.first())
                    .map(String::as_str),
                Some("swarm_request")
            );
        } else if vector.surface == "framing" {
            assert!(
                vector.declared_length.unwrap()
                    > triptorrent_protocol::MAX_APPLICATION_MESSAGE_BYTES
            );
        }
        assert_ne!(vector.expect, "accept");
    }
}
