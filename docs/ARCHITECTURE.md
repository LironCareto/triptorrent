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

- What should the canonical TripTorrent multifile/tree identifier be?
- How should verified BitTorrent aliases be authenticated in native metadata?
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

## M4 swarm transfer

M4 adds `triptorrent-swarm` for deterministic scheduling and bandwidth reservations while retaining the M2 bootstrap as replaceable discovery scaffolding. One discovery response creates at most eight independent provider routes, distributed deterministically across live relays. Each provider advertises a compact bitfield only after its end-to-end Noise session is established; piece availability is not published into the M3 research design.

The receiver first obtains and compares manifests from connected providers. The requested content ID, total length, 32 KiB chunk size and ordered BLAKE3 chunk hashes must agree exactly. It then assigns one in-flight chunk per provider, preferring the lowest-index chunk among those with the fewest active sources. Verified responses are written by offset to a transfer-local partial file. A disconnect, unavailable response, wrong index or invalid digest disables that provider for the current transfer and releases its work to another source. Final output appears only after the complete content ID verifies.

Resume uses `<output>.triptorrent-part` plus a Postcard-encoded `<output>.triptorrent-state`. The state contains a format version, the exact manifest and completion bits. On restart, every claimed completed chunk is reread and verified before reuse. Successful completion renames the partial file and removes metadata. This is download state, not the persistent node storage planned for M5.

Download and upload limits use deterministic leaky-bucket reservations measured in bytes per second. Omitted or zero limits are unlimited. Scheduling is bounded by eight provider sessions; there is no tit-for-tat, choking, global reputation or long-lived provider score.

## M5 persistent node runtime

M5 adds `triptorrent-node` as an implementation layer above the existing network crates. It owns persistent configuration, a versioned SQLite metadata index, managed content files, transfer recovery, worker lifecycle, structured observability and a versioned loopback HTTP+JSON API. A small `TransferEngine` interface keeps M4 execution outside the node state machine.

`triptorrent-cli` supplies the current M4 adapter and acts as a client of the local API for persistent operations. Command parsing does not own daemon state, storage or API routing. A future GUI can call the same API without embedding CLI behavior.

At startup the node verifies indexed bytes and manifests, disables sharing for missing or corrupt entries, converts stale `running` downloads to `interrupted`, resumes those downloads from M4's verified partial files, and starts new advertisements for valid shared content. Relay routes, connections and protocol peer keys remain ephemeral and are rebuilt. See [ADR 0006](adr/0006-m5-persistent-node.md) for the boundary and [the M5 guide](M5_PERSISTENT_NODE.md) for operational details.

## M6 BitTorrent compatibility boundary

M6 keeps the current TripTorrent BLAKE3 content identity independent from BitTorrent identities. BEP 3 `btih` and BEP 52 `btmh` values are typed aliases because they hash exact torrent `info` metadata rather than bare payload bytes. Metadata import records aliases as unverified; reading and validating payload bytes binds aliases to a TripTorrent manifest. Multifile torrents require a future canonical tree identity and cannot be represented safely as concatenated blobs.

The persistent content layer is shared by isolated network adapters. A TripTorrent-native adapter uses an M3 discovery capability, native discovery, and relayed transfer. A future classic adapter may use trackers, Mainline DHT, PEX, local discovery, web seeds, and direct peers only under an explicit dual-network or classic policy. TripTorrent-only policy fails closed and never invokes the classic adapter. Network modes, active paths, alias status, and private-torrent restrictions must remain visible through local APIs and clients.

M6 implements only an offline research crate for strict bencoding, identity calculation, magnet parsing, Merkle validation, and path sanitization. It adds no production BitTorrent network component. [RFC 0004](../rfcs/0004-namespaced-bittorrent-compatibility.md) defines the selected architecture; [the research report](M6_BITTORRENT_INTEROP_RESEARCH.md) records evidence and remaining questions.

## M7 desktop and local API boundary

M7 adds `triptorrent-desktop`, a native `egui`/`eframe` reference client, and `triptorrent-node-api`, the shared typed DTO and loopback HTTP client crate. The desktop and CLI depend on the API client. Only `triptorrent-node` owns SQLite, managed files, recovery, providers, downloads and API routing; presentation code cannot bypass authentication or call the swarm engine.

The desktop controller performs API and process work on a background thread. It probes an existing node before starting the adjacent CLI sidecar, waits for readiness, polls at a bounded interval and reconnects with backoff. Closing the window does not stop the persistent node. UI state is a snapshot of daemon state rather than a second transfer state machine.

The local API now reports typed content identities, advertised state, verified bytes/chunks, measured verified-byte rate, provider/retry/rejection contribution, and the active network path. Cooperative pause stops new chunk scheduling, releases sessions and preserves M4 partial state; resume starts a fresh discovery session after revalidating completed chunks. A persisted `paused` state is excluded from automatic interrupted-transfer recovery.

The structured privacy model labels the actual M2 discovery and M4 relay path and explicitly marks M3 private discovery and M6 classic networking inactive. This is an implementation API extension, not a wire-protocol change. See [ADR 0008](adr/0008-m7-local-api-desktop-client.md) and the [M7 guide](M7_DESKTOP_CLIENT.md).

## M8 adversarial validation boundary

M8 leaves the M1-M7 architecture intact and makes implementation resource policies explicit. Application/control messages, manifests, interop metadata, relay routes, bootstrap registries, local API connections and resume state now have bounded sizes or counts. Validation occurs before allocation, path derivation or state promotion where practical. Malformed connections fail independently; verified bytes and authenticated mutation state cannot be created by parser success alone.

The `fuzz/` package is isolated from the stable workspace so long libFuzzer campaigns do not affect Windows development or normal CI. Stable deterministic regressions and property tests remain in their owning crates. `triptorrent-overlay::research` adds fixed-seed Monte Carlo scenarios, while `traffic_analysis` is a synthetic metadata-only model using real encoded message sizes. Neither is a production networking component.

The bounds are reference-implementation policy for the current prototype, except where [the draft specification](../spec/README.md) records current experimental wire validity. M2 remains unauthenticated scaffolding, and M8 does not add identity cost, traffic padding, cover traffic, public-testnet operations or production M3 discovery. See the [M8 report](M8_ADVERSARIAL_TESTING.md).
