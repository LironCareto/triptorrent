# TripTorrent Testnet Protocol v1

Status: **normative for the controlled experimental testnet**. The key words MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY are normative. This profile is not a production or anonymity specification.

## Identity and layering

- Wire protocol version: unsigned integer `1`.
- Network identifier: UTF-8/ASCII `triptorrent-testnet-1`.
- Discovery profile: `m2-testnet-bootstrap-v1` (centralized testnet scaffolding).
- Transfer profile: `relayed-swarm-v1`.

Software versions are independent of these identifiers. Every bootstrap message, relay registration, and encrypted peer application record carries the protocol version and network identifier. Missing, malformed, unsupported-version, wrong-network, and unsupported-required-capability inputs MUST fail closed. Implementations MUST NOT retry with the M1-M8 version-0/Postcard protocol.

Core identity, encrypted sessions, relay transport, and swarm transfer are independent of discovery. A future M3 discovery implementation may provide the same provider public key, relay endpoint, and opaque route without changing the downstream protocol.

## Integer, string, and framing rules

All integers are unsigned, big-endian, and fixed-width (`u8`, `u16`, `u32`, or `u64`). A `bytes` value is `u32 length || octets`; a `string` is `u16 byte-length || canonical UTF-8`. Arrays have an explicit count followed by elements. Optional route assignments use tag `0` for absent and `1` for present. Booleans and maps are not used. Decoders MUST reject truncation, trailing bytes, invalid UTF-8, invalid optional tags, unknown message types, nonzero availability padding, and values beyond stated limits.

TCP records are `u32 payload-length || payload`. Control records MUST NOT exceed 262,144 bytes. Relay opaque records MUST NOT exceed 1,048,576 bytes. Decrypted application records MUST NOT exceed 65,519 bytes.

Application and control payloads begin:

```text
"TTP1" || protocol:u16 || network_len:u8 || network || domain:u8 || type:u8 || body
```

Domains are application `1`, bootstrap request `2`, and bootstrap response `3`.

## Content and manifest

A content ID is the 32-byte BLAKE3 digest of complete file bytes. A chunk ID is the 32-byte BLAKE3 digest of one chunk. Text form is 64 lowercase hexadecimal characters. The chunk size is exactly 32,768 bytes. A manifest body is:

```text
content_id:32 || content_length:u64 || chunk_size:u32 || chunk_count:u32 || chunk_id:32 * chunk_count
```

`chunk_count` MUST equal `ceil(content_length/chunk_size)` and MUST NOT exceed 2,036 in Testnet v1. This wire limit permits about 63.6 MiB and replaces the larger internal M8 prototype bound. Empty content has zero chunks. Each chunk and the complete reconstructed content MUST verify before publication.

Peer IDs are 32-byte BLAKE3 digests of the advertised 32-byte Noise static public key. They are ephemeral process identifiers, not durable identities.

## Bootstrap discovery profile

The bootstrap is centralized M2-derived scaffolding. It observes source IPs, peer/content advertisements, queries, relay endpoints, route assignments, timing, and churn. Requests are:

| Type | Name | Body |
|---:|---|---|
| 1 | RegisterPeer | `peer_id:32 || public_key:32 || capability_count:u8 || capability:u16* || content_count:u16 || content_id:32*` |
| 2 | HeartbeatPeer | `peer_id:32` |
| 3 | PollPeer | `peer_id:32` |
| 4 | RegisterRelay | `relay_id:string || address:string` |
| 5 | HeartbeatRelay | `relay_id:string` |
| 6 | Discover | `content_id:32` |
| 7 | DiscoverProviders | `content_id:32 || limit:u16` |
| 8 | ReportRelayFailure | `relay_id:string` |
| 9 | Health | empty; MUST NOT mutate registry state |

Capabilities are `1` RelayedTransferV1 and `2` SwarmTransferV1. Unknown required capabilities MUST be rejected. Peer advertisements contain 1-8 capabilities and 1-256 distinct content IDs. Discovery limits are 1-16.

Responses are: `1 Registered(lease_ms:u64)`, `2 Assignment(optional assignment)`, `3 Route(optional assignment)`, `4 Routes(count:u16 || assignment*)`, `5 Acknowledged`, and `255 Error(message:string)`. An assignment is `provider_id:32 || provider_public_key:32 || relay_id:string || relay_address:string || route:string`. IDs/routes use 1-64/128 ASCII letters, digits, `_`, or `-`; addresses are numeric IP socket addresses. Error text is at most 1,024 bytes.

Registrations are leased. Heartbeat and provider polling renew existing leases. The bootstrap MAY choose providers/relays by local policy, but MUST return distinct providers in one result and MUST cap registry, assignment, and handler state as documented by the reference limits. Reports are unauthenticated in this profile.

## Relay setup and forwarding

Each endpoint opens TCP to the assigned relay and sends one framed registration:

```text
"TTR1" || protocol:u16 || network_len:u8 || network || role:u8 || route_len:u16 || route
```

Role `0` is provider/sender and `1` is receiver. The relay pairs one endpoint of each role for a route, rejects duplicates/reuse, expires unmatched routes after 30 seconds, and then forwards framed octets without interpreting Noise or application plaintext. Route length is 1-128 restricted ASCII. Wrong network/version MUST fail before pairing.

## Noise session

Swarm sessions use exactly `Noise_KN_25519_ChaChaPoly_BLAKE2s`, empty prologue, and no PSK. The provider is the Noise initiator and owns the advertised ephemeral X25519 static private key. The receiver is the responder and configures that advertised key as the initiator remote static key. The two handshake messages, then every Noise transport message, use relay framing. Application headers are inside Noise transport encryption.

Implementations MUST use a reviewed Noise library. Noise authentication failure, nonce exhaustion, replay, or out-of-order transport records terminate the connection. Routes are single-use; application records are not replayable into another Noise session.

## Swarm application protocol

Application message types are:

| Type | Name | Body |
|---:|---|---|
| 1 | Request | `content_id:32` (legacy single-source semantic inside v1 only) |
| 2 | Manifest | manifest |
| 3 | Chunk | `index:u32 || chunk_id:32 || data:bytes` |
| 4 | Complete | empty |
| 5 | Error | `message:string` |
| 16 | SwarmRequest | `content_id:32` |
| 17 | SwarmManifest | `manifest || availability_count:u32 || availability:bytes` |
| 18 | ChunkRequest | `index:u32` |
| 19 | ChunkUnavailable | `index:u32` |
| 20 | SwarmComplete | empty |

Availability bits are least-significant-bit first; its count MUST equal manifest chunk count, byte length MUST equal `ceil(count/8)`, and unused high bits MUST be zero.

The receiver sends one SwarmRequest first. The provider replies with SwarmManifest or Error. The receiver MAY then send ChunkRequest messages; the provider replies with the matching Chunk or ChunkUnavailable. A successful receiver sends SwarmComplete. Any request before a manifest, mismatched index, inconsistent manifest, unrequested/replayed chunk, invalid digest, or message invalid in the current state terminates that session. Multi-provider receivers MUST require byte-identical manifests and MUST verify every chunk and final content ID.

Unknown message types are fatal in v1. There are no ignorable fields. Extensions require a later protocol version or a separately negotiated capability whose encoding is specified. Connection teardown is TCP close after completion or any fatal error; implementations SHOULD send Error only when a valid encrypted session remains usable.

## Limits and security position

Reference Internet-facing limits are 4,096 peers, 256 relays, 256 content IDs per peer, 64 queued assignments per provider, 16 discovery results, 256 bootstrap handlers, 1,024 pending relay routes, 256 active relay pairs, and 256 relay-registration handlers. Operators MAY lower them and MUST NOT remove bounds.

The bootstrap claims are unauthenticated and can be spoofed or biased. A relay sees endpoint pairing, route identifiers, timing, and volume. Traffic shape permits correlation. Targeted Sybil/eclipse resistance and global-passive-observer resistance are not provided. No direct provider connection occurs in this profile, and the relay lacks application plaintext, but anonymity is not guaranteed. The testnet MUST NOT be used for privacy-sensitive transfers.
