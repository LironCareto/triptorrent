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
