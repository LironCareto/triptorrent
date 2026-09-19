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

## M2 local multi-node overlay

M2 adds one architectural crate, `triptorrent-overlay`, rather than embedding discovery into the relay or transfer protocol. It contains a deterministic in-memory registry, a temporary TCP bootstrap service and client, leases, content lookup, relay selection, queued route assignments and a no-I/O simulation harness.

The local flow is:

1. Relays register an identifier and reachable TCP address and renew a server-controlled lease.
2. A provider generates an ephemeral Noise static keypair, derives an experimental peer ID from its public key, advertises that key and its content IDs, and polls while renewing its lease.
3. A receiver queries the bootstrap for a content ID.
4. The bootstrap chooses the lexicographically first live provider and relay, creates a unique local route ID, returns it to the receiver, and queues the same assignment for the provider.
5. Both peers connect independently to the selected relay. The existing M1 framing and file messages run inside `Noise_KN_25519_ChaChaPoly_BLAKE2s`; the receiver uses the discovered provider public key.
6. If connection fails before transfer, the client reports that relay unavailable and repeats discovery against the next live relay.

Peer and relay entries disappear when their leases expire. Re-registering replaces stale metadata and makes content discoverable again. Route assignments also expire and are removed when their peer or relay disappears. State is memory-only and bootstrap restart loses all registrations.

The provider's ephemeral key removes the manual M1 PSK from the M2 CLI without creating a key-distribution construction. It authenticates possession of the advertised provider key, but the bootstrap advertisement itself is unauthenticated. A malicious bootstrap can substitute a public key and mediate sessions. There is no durable or production peer identity.

The bootstrap is deliberately temporary and centralized. Its control connection and Postcard messages are experimental implementation scaffolding expected to be replaced or substantially reworked by a later discovery prototype. It is not a normative dependency or a claim of decentralized discovery.

## M3 discovery research direction

M3 keeps discovery, rendezvous and transfer as separate layers. The selected prototype direction is:

1. A content link carries a random discovery capability separately from the content ID.
2. A domain-separated capability-derived key addresses encrypted, signed and expiring provider descriptors in a Kademlia-style DHT.
3. Publication and lookup traverse independently selected oblivious relays and gateways. A relay sees the client endpoint and encrypted request; a gateway sees the DHT operation and relay endpoint.
4. Records and queries use network-prefix diversity and multiple independent paths to reduce one target-key cluster's influence.
5. Decrypted descriptors yield opaque rendezvous tokens and relay candidates. Rendezvous coordinates the existing M1 encrypted transfer without placing file data in discovery.
6. Cached peers, user contacts, peer exchange and independent seeds populate routing tables only. Seeds do not perform content lookups.

This direction removes the mandatory M2 service that sees the complete requester/content/provider mapping under a non-collusion assumption. It does not hide stable derived keys from DHT nodes, both transfer endpoints from their selected relay, or timing from broad observers. A public discovery capability permits observers holding it to recognize the lookup namespace.

The no-I/O M3 model lives in `triptorrent-overlay::research`; no production DHT, OHTTP transport or rendezvous protocol has been implemented. [RFC 0002](../rfcs/0002-separated-multi-stage-discovery.md) defines the next experiment, and [the M3 report](M3_DISCOVERY_RESEARCH.md) contains the evidence and unresolved questions.
