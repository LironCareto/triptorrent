"""Consumes the checked-in TripTorrent Testnet v1 conformance vectors."""

import json
import struct
from pathlib import Path

from blake3 import blake3
from triptorrent_v1 import (
    DOMAIN_APPLICATION, DOMAIN_REQUEST, DOMAIN_RESPONSE, MAX_FRAME, NETWORK, VERSION,
    Reader, app_chunk_request, app_complete, app_request, header, relay_registration,
)

ROOT = Path(__file__).resolve().parents[2]


def text(value: str) -> bytes:
    data = value.encode()
    return struct.pack(">H", len(data)) + data


def blob(value: bytes) -> bytes:
    return struct.pack(">I", len(value)) + value


def expected_positive(name: str, payload: bytes) -> bytes:
    content_id = blake3(payload).digest()
    chunk_id = blake3(payload).digest()
    key = bytes([7]) * 32
    peer = blake3(key).digest()
    manifest = content_id + struct.pack(">QII", len(payload), 32768, 1) + chunk_id
    assignment = peer + key + text("relay-a") + text("127.0.0.1:7001") + text("m9-route")
    values = {
        "swarm-request": app_request(content_id),
        "swarm-manifest": header(DOMAIN_APPLICATION, 17) + manifest + struct.pack(">II", 1, 1) + b"\x01",
        "chunk-request": app_chunk_request(0),
        "chunk-response": header(DOMAIN_APPLICATION, 3) + struct.pack(">I", 0) + chunk_id + blob(payload),
        "unavailable": header(DOMAIN_APPLICATION, 19) + struct.pack(">I", 0),
        "completion": app_complete(),
        "structured-error": header(DOMAIN_APPLICATION, 5) + text("test error"),
        "peer-registration": header(DOMAIN_REQUEST, 1) + peer + key + b"\x01" + struct.pack(">H", 2) + struct.pack(">H", 1) + content_id,
        "relay-advertisement": header(DOMAIN_REQUEST, 4) + text("relay-a") + text("127.0.0.1:7001"),
        "provider-discovery": header(DOMAIN_REQUEST, 7) + content_id + struct.pack(">H", 1),
        "bootstrap-health": header(DOMAIN_REQUEST, 9),
        "route-assignment": header(DOMAIN_RESPONSE, 4) + struct.pack(">H", 1) + assignment,
        "relay-registration": relay_registration("m9-route"),
    }
    return values[name]


def assert_rejected(vector: dict) -> None:
    if vector["surface"] == "state":
        if vector["sequence"] and vector["sequence"][0] != "swarm_request":
            return
        raise AssertionError("invalid state vector was not invalid")
    if vector["surface"] == "framing":
        if vector["declared_length"] > MAX_FRAME:
            return
        raise AssertionError("oversized framing vector was not oversized")
    data = bytes.fromhex(vector["hex"])
    try:
        domain = DOMAIN_REQUEST if vector["surface"] == "overlay_request" else DOMAIN_APPLICATION
        reader = Reader(data, domain)
        kind = reader.u8()
        if domain == DOMAIN_REQUEST and kind == 1:
            reader.take(64)
            capabilities = reader.u8()
            for _ in range(capabilities):
                if reader.u16() not in {1, 2}:
                    raise ValueError("unsupported capability")
            raise AssertionError("hostile capability vector did not contain an unknown value")
        if kind not in {1, 2, 3, 4, 5, 16, 17, 18, 19, 20}:
            raise ValueError("unknown message")
        reader.finish()
    except ValueError:
        return
    raise AssertionError(f"negative vector accepted: {vector['name']}")


def main() -> None:
    suite = json.loads((ROOT / "test-vectors/m9-testnet-v1.json").read_text())
    assert suite["protocol_version"] == VERSION
    assert suite["network"].encode() == NETWORK
    payload = (ROOT / suite["canonical_artifact"]["path"]).read_bytes()
    assert len(payload) == suite["canonical_artifact"]["size"]
    assert blake3(payload).hexdigest() == suite["canonical_artifact"]["content_id"]
    for vector in suite["positive"]:
        assert expected_positive(vector["name"], payload).hex() == vector["hex"], vector["name"]
    for vector in suite["negative"]:
        assert_rejected(vector)
    print(f"PASS {len(suite['positive']) + len(suite['negative'])} vectors network={NETWORK.decode()} protocol={VERSION}")


if __name__ == "__main__":
    main()
