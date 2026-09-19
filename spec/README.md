# TripTorrent Protocol Specification

Status: **draft / pre-alpha**

This directory will contain the normative TripTorrent protocol specification.

## Planned sections

1. Terminology
2. Version negotiation
3. Content identifiers and manifests
4. Peer identities
5. Session establishment
6. Discovery
7. Routing and relays
8. Swarm messages
9. Piece exchange
10. Error handling
11. Privacy properties
12. Security considerations
13. Extension mechanism
14. Compatibility considerations

Until a version is explicitly marked stable, all wire formats are subject to change.

## Experimental M2 bootstrap messages

M2 currently uses a non-normative, versioned Postcard request/response protocol over length-prefixed TCP connections. Requests cover:

- peer registration with an ephemeral peer ID, Noise public key, transfer capabilities and content IDs;
- peer heartbeat and route polling;
- relay registration and heartbeat with an identifier and TCP address;
- discovery by content ID;
- relay-failure reporting.

Responses acknowledge a server-controlled lease or return an optional route assignment. An assignment contains the provider ID and public key, selected relay ID and address, and an automatically generated route ID. Registrations and queued routes expire according to the bootstrap's monotonic lease clock.

M4 adds an experimental `DiscoverProviders` request with a bounded provider count and a `Routes` response. The bootstrap returns distinct live providers advertising `SwarmTransferV0`, creates one route per provider and distributes those routes across live relays in deterministic order. This remains temporary M2 scaffolding and is not the M3 discovery design.

These messages are prototype scaffolding, are not authenticated, and are not the normative TripTorrent discovery protocol. Their current encoding and semantics may be removed or replaced by a later discovery prototype.

## Experimental M3 discovery direction

M3 research recommends capability-derived DHT keys, encrypted signed provider descriptors, oblivious relay/gateway publication and lookup, diverse replicas, independent paths and opaque rendezvous tokens. [RFC 0002](../rfcs/0002-separated-multi-stage-discovery.md) defines the next prototype direction.

No M3 wire encoding is normative or implemented. The content ID and discovery capability must remain separate, and private discovery must not silently fall back to direct transfer or the public BitTorrent DHT. Exact key derivation, descriptor encoding, cryptographic suites, routing RPCs, expiry and rendezvous state machines require test vectors and specification before interoperability claims.

## Experimental M4 swarm messages

M4 application messages reuse the version-0 Postcard envelope and Noise sessions. After the relay and Noise handshake, a receiver sends `SwarmRequest { content_id }`. The provider replies with `SwarmManifest { manifest, availability }`, where the manifest contains the content ID, byte length, fixed 32 KiB chunk size and ordered chunk digests. `availability` is a least-significant-bit-first bitfield with an explicit chunk count; padding bits must be zero.

The receiver sends `ChunkRequest { index }` and the provider returns either the existing `Chunk` message or `ChunkUnavailable { index }`. Every returned index, claimed digest, actual bytes and final content ID must verify. The receiver sends `SwarmComplete` when it no longer needs the session. Providers do not send the complete file eagerly in this mode.

A receiver compares all connected provider manifests byte-for-byte and fails closed on disagreement. It may issue requests concurrently on independent sessions, but must bound concurrency, avoid unnecessary duplicates and retry failed work only from providers claiming that index. Availability is session data and must not be published as part of the future M3 DHT record.

The current resume files and bandwidth-limit algorithm are local implementation details, not wire protocol. No M4 message is stable or normative; see [RFC 0003](../rfcs/0003-m4-swarm-transfer.md).
