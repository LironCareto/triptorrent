"""Independent TripTorrent Testnet v1 receiver."""

from __future__ import annotations

import argparse
import socket
import struct
from pathlib import Path

MAGIC = b"TTP1"
RELAY_MAGIC = b"TTR1"
VERSION = 1
NETWORK = b"triptorrent-testnet-1"
MAX_FRAME = 1_048_576
DOMAIN_APPLICATION = 1
DOMAIN_REQUEST = 2
DOMAIN_RESPONSE = 3


def header(domain: int, kind: int) -> bytes:
    return MAGIC + struct.pack(">HB", VERSION, len(NETWORK)) + NETWORK + bytes((domain, kind))


def frame(payload: bytes) -> bytes:
    if len(payload) > MAX_FRAME:
        raise ValueError("frame too large")
    return struct.pack(">I", len(payload)) + payload


def recv_exact(sock: socket.socket, length: int) -> bytes:
    value = bytearray()
    while len(value) < length:
        part = sock.recv(length - len(value))
        if not part:
            raise EOFError("connection closed")
        value.extend(part)
    return bytes(value)


def recv_frame(sock: socket.socket) -> bytes:
    length = struct.unpack(">I", recv_exact(sock, 4))[0]
    if length > MAX_FRAME:
        raise ValueError("frame too large")
    return recv_exact(sock, length)


class Reader:
    def __init__(self, data: bytes, domain: int):
        self.data = data
        self.offset = 0
        if self.take(4) != MAGIC:
            raise ValueError("invalid magic")
        version = self.u16()
        if version != VERSION:
            raise ValueError(f"unsupported protocol version {version}")
        network = self.take(self.u8())
        if network != NETWORK:
            raise ValueError(f"wrong network {network!r}")
        if self.u8() != domain:
            raise ValueError("wrong message domain")

    def take(self, length: int) -> bytes:
        end = self.offset + length
        if end > len(self.data):
            raise ValueError("truncated input")
        value = self.data[self.offset:end]
        self.offset = end
        return value

    def u8(self) -> int: return self.take(1)[0]
    def u16(self) -> int: return struct.unpack(">H", self.take(2))[0]
    def u32(self) -> int: return struct.unpack(">I", self.take(4))[0]
    def u64(self) -> int: return struct.unpack(">Q", self.take(8))[0]
    def text(self) -> str: return self.take(self.u16()).decode("utf-8")

    def finish(self) -> None:
        if self.offset != len(self.data):
            raise ValueError("trailing bytes")


def discover(bootstrap: str, content_id: bytes) -> dict:
    host, port = split_address(bootstrap)
    request = header(DOMAIN_REQUEST, 7) + content_id + struct.pack(">H", 1)
    with socket.create_connection((host, port), timeout=5) as sock:
        sock.sendall(frame(request))
        response = Reader(recv_frame(sock), DOMAIN_RESPONSE)
    if response.u8() != 4 or response.u16() != 1:
        raise ValueError("bootstrap returned no unique route")
    provider_id = response.take(32)
    provider_key = response.take(32)
    relay_id = response.text()
    relay_address = response.text()
    route = response.text()
    response.finish()
    return {"provider_id": provider_id, "provider_key": provider_key, "relay_id": relay_id, "relay_address": relay_address, "route": route}


def split_address(value: str) -> tuple[str, int]:
    host, port = value.rsplit(":", 1)
    return host, int(port)


def relay_registration(route: str) -> bytes:
    encoded = route.encode("ascii")
    return RELAY_MAGIC + struct.pack(">HB", VERSION, len(NETWORK)) + NETWORK + b"\x01" + struct.pack(">H", len(encoded)) + encoded


def app_request(content_id: bytes) -> bytes:
    return header(DOMAIN_APPLICATION, 16) + content_id


def app_chunk_request(index: int) -> bytes:
    return header(DOMAIN_APPLICATION, 18) + struct.pack(">I", index)


def app_complete() -> bytes:
    return header(DOMAIN_APPLICATION, 20)


def fetch(bootstrap: str, content_hex: str, output: Path) -> None:
    from blake3 import blake3
    from noise.connection import Keypair, NoiseConnection

    content_id = bytes.fromhex(content_hex)
    if len(content_id) != 32:
        raise ValueError("content ID must be 32 bytes")
    route = discover(bootstrap, content_id)
    host, port = split_address(route["relay_address"])
    with socket.create_connection((host, port), timeout=10) as sock:
        sock.settimeout(10)
        sock.sendall(frame(relay_registration(route["route"])))
        noise = NoiseConnection.from_name(b"Noise_KN_25519_ChaChaPoly_BLAKE2s")
        noise.set_as_responder()
        noise.set_keypair_from_public_bytes(Keypair.REMOTE_STATIC, route["provider_key"])
        noise.start_handshake()
        noise.read_message(recv_frame(sock))
        sock.sendall(frame(noise.write_message()))
        sock.sendall(frame(noise.encrypt(app_request(content_id))))
        manifest = Reader(noise.decrypt(recv_frame(sock)), DOMAIN_APPLICATION)
        if manifest.u8() != 17:
            raise ValueError("provider did not return a swarm manifest")
        actual_id = manifest.take(32)
        length = manifest.u64()
        chunk_size = manifest.u32()
        count = manifest.u32()
        chunk_ids = [manifest.take(32) for _ in range(count)]
        availability_count = manifest.u32()
        availability = manifest.take(manifest.u32())
        manifest.finish()
        if actual_id != content_id or chunk_size != 32768 or availability_count != count:
            raise ValueError("invalid manifest")
        chunks = []
        for index, expected in enumerate(chunk_ids):
            if not (availability[index // 8] & (1 << (index % 8))):
                raise ValueError("provider lacks required chunk")
            sock.sendall(frame(noise.encrypt(app_chunk_request(index))))
            response = Reader(noise.decrypt(recv_frame(sock)), DOMAIN_APPLICATION)
            if response.u8() != 3 or response.u32() != index:
                raise ValueError("unexpected chunk response")
            claimed = response.take(32)
            data = response.take(response.u32())
            response.finish()
            if claimed != expected or blake3(data).digest() != expected:
                raise ValueError(f"chunk {index} failed verification")
            chunks.append(data)
        sock.sendall(frame(noise.encrypt(app_complete())))
    payload = b"".join(chunks)
    if len(payload) != length or blake3(payload).digest() != content_id:
        raise ValueError("completed payload failed verification")
    output.write_bytes(payload)
    print(f"PASS content={content_hex} bytes={len(payload)} network={NETWORK.decode()} protocol={VERSION}")


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    command = sub.add_parser("fetch")
    command.add_argument("--bootstrap", required=True)
    command.add_argument("--content", required=True)
    command.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    fetch(args.bootstrap, args.content, args.output)


if __name__ == "__main__":
    main()
