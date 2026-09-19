# RFC 0003: Experimental M4 swarm transfer

- Status: draft
- Authors: TripTorrent contributors
- Created: 2026-09-19

## Summary

Define the pre-alpha messages and state transitions used to request verified chunks concurrently from several providers through independent relayed Noise sessions.

## Motivation

M1 transfers one complete file from one sender. M2 automates one provider route. M4 must demonstrate actual swarming, heterogeneous piece availability, retry after failure or corruption, local resume and basic rate control while leaving M3 discovery unimplemented.

## Protocol

Peers advertise `SwarmTransferV0`. Temporary discovery returns a bounded list of distinct provider identities, public keys and relay routes. For each session the receiver sends `SwarmRequest(content_id)`. The provider returns one authoritative `Manifest` plus a compact least-significant-bit-first availability bitfield whose declared length equals the manifest chunk count and whose padding bits are zero.

The receiver requests an index with `ChunkRequest`. The provider returns `Chunk` or the matching `ChunkUnavailable`. A receiver accepts a chunk only when its index is outstanding for that provider and both claimed and computed BLAKE3 digests equal the manifest entry. `SwarmComplete` closes a successful provider session. Existing versioned Postcard framing, relay registration and `Noise_KN_25519_ChaChaPoly_BLAKE2s` encryption remain unchanged.

Connected providers must return identical manifests for the requested content ID, length, chunk size and ordered digests. Disagreement fails closed. The reference scheduler prefers chunks with fewer active sources, breaks ties by index, allows one in-flight request per provider, caps providers at eight and reassigns failed work. These policy values are experimental rather than required interoperability constants.

## Resume and rate control

Resume is receiver-local and outside the wire protocol. The reference implementation stores the exact manifest and completion bits beside a fixed-length partial file, flushes chunk data before recording completion, and revalidates completed chunks after restart. It publishes the output only after complete-content verification.

Download and upload limits are optional bytes-per-second controls. Zero or omission means unlimited. Implementations may use another established limiter provided it does not alter protocol semantics.

## Privacy and security

All peer traffic remains relayed and encrypted end to end. Relays still observe endpoints, route IDs, timing and sizes. Providers learn requested chunk indices; receivers learn provider availability. The M2 bootstrap learns the content/provider/receiver route graph. More parallel sources expand metadata exposure. M4 adds no anonymity property and no global reputation. A provider penalized for corruption or failure is excluded only from the current transfer.

## Compatibility and migration

Manual M1 messages and routes remain supported. M4 does not implement or modify the M3 DHT/OHTTP proposal and does not publish availability into it. Protocol version 0 has no stability promise; independent implementations must treat these additions as experimental.

## Test requirements

Conformance work must cover deterministic encoding, malformed availability, manifest disagreement, wrong indices, corruption, unavailable chunks and completion. The reference acceptance suite additionally launches real CLI processes for multi-source transfer, partial availability, provider loss, malicious chunks, restart/resume and simultaneous swarms.
