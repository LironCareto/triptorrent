# Architecture (v0)

This document describes the current design space, not a frozen protocol.

## Layers

### 1. Content
Defines immutable content identifiers, manifests, chunks/pieces, integrity verification and metadata.

### 2. Peer identity
Defines cryptographic peer identifiers and their lifetime. Network endpoints must not be treated as durable peer identities.

### 3. Discovery
Finds peers and content without requiring a central tracker.

### 4. Secure transport
Provides authenticated encryption, capability negotiation and replay-resistant session establishment.

### 5. Privacy routing
Provides a mechanism for peers to communicate without requiring direct endpoint disclosure to each other.

### 6. Swarm
Coordinates piece availability, request scheduling, fairness, retries and multi-source transfer.

### 7. Compatibility
Optional mechanisms for importing or mapping BitTorrent identifiers and metadata.

## Current open questions

- Which content identifier format should be normative?
- Should BitTorrent v2 infohashes be reusable directly?
- What routing model gives acceptable privacy without destroying throughput?
- How many relay hops should be typical?
- How should bootstrap work without creating a practical central point?
- What is the Sybil-resistance model?
- How much cover traffic, if any, is practical?
- How should NAT traversal interact with privacy guarantees?
- What metadata is visible to intermediate nodes?
- What threat model is realistic against large passive observers?

These are intentionally unresolved.

## M1 executable vertical slice

M1 implements the path `Peer A -> Relay -> Peer B` as five Rust crates:

- `triptorrent-core` creates experimental BLAKE3 content and chunk IDs, fixed 32 KiB chunks, and manifests.
- `triptorrent-protocol` serializes versioned request, manifest, chunk, completion and error messages with Postcard.
- `triptorrent-net` separates framed TCP relay connections from end-to-end Noise sessions and application messages.
- `triptorrent-relay` pairs one sender and receiver by a visible route ID and forwards opaque length-delimited frames. It has no dependency on the content or protocol crates.
- `triptorrent-cli` provides the local relay, identify, share and fetch commands.

Each peer opens one TCP connection to the relay; neither peer learns or connects to the other's endpoint. Application messages and file chunks are encrypted between peers with `Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s`. Possession of the out-of-band 32-byte pre-shared key authenticates the session. The relay observes endpoint addresses, route ID, role, timing, frame sizes and transfer duration, but does not receive the key or plaintext application frames.

The receiver requests a content ID, verifies every chunk against the manifest, reconstructs the bytes, and verifies the complete content ID before renaming the output file. There are no retries, resume support, concurrency controls or persistent state.

All M1 formats and parameters are experimental. They demonstrate layer boundaries and a working transfer; they do not settle normative content addressing, peer identity, discovery, routing, compatibility or anonymity design.
