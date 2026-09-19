# RFC 0002: Separated multi-stage discovery direction

- Status: draft
- Authors: TripTorrent contributors
- Created: 2026-09-19

## Summary

The next TripTorrent discovery prototype should combine capability-derived Kademlia keys, encrypted signed provider descriptors, oblivious publication and lookup paths, diverse replication, and rendezvous tokens that lead to the existing encrypted relayed transfer. This RFC records a research direction. It does not define a stable wire format or replace M2.

## Motivation

M2's bootstrap observes requester IP, raw content query, provider identity and endpoint, relay choice and timing in one service. Vanilla Kademlia distributes that state but still reveals requester endpoint plus content key during iterative lookup and provider endpoint plus key during announcement. M3 simulation also shows direct closest-node replication failing completely when 250 Sybils occupy a target key.

The proposed separation ensures that, absent collusion, no DHT node, query relay, gateway or bootstrap seed receives the full requester-to-content-to-provider mapping.

## Proposed protocol shape

### Content link

A TripTorrent private-discovery link carries two independent values:

- the content identifier used by M1 integrity verification;
- a random 32-byte discovery capability.

The capability is not a peer identity or network endpoint. A public capability provides no secrecy from other holders of the same link.

### Keys

Domain-separated, versioned derivations produce:

- `lookup_key`, used only as the DHT target;
- `descriptor_key`, used only to encrypt provider descriptors.

The prototype should use HKDF and a specified standard hash. It must not use `hash(content_id)` as the privacy mechanism because public content permits dictionary testing.

### Provider descriptor

The encrypted plaintext should contain at least:

- protocol and descriptor version;
- content ID;
- process-scoped provider signing/session public keys;
- supported transfer capabilities;
- several relay endpoints and opaque rendezvous tokens;
- sequence number and expiration;
- a signature covering all preceding fields.

Use Ed25519 signatures and an established AEAD such as ChaCha20-Poly1305. A self-signature detects modification and binds updates; it does not establish a durable identity or provider trust. Receivers still verify content through M1 hashes.

### Publication and lookup

Publish eight short-lived replicas near `lookup_key`, subject to network-prefix diversity. Publication and lookup use an OHTTP-style client -> relay -> gateway separation based on RFC 9458 and HPKE. The relay learns the client endpoint and gateway but not the request. The gateway learns the operation and DHT key but not the original endpoint.

Receivers query through three independently selected relay/gateway pairs, validate and merge returned descriptors, and reject invalid signatures, expired records and unsupported versions. Relay, gateway and network-prefix choices should be diverse. No result count is an identity-based quorum while identities remain cheap.

### Rendezvous and transfer

The receiver selects a descriptor and sends an introduction using its opaque rendezvous token. The provider and receiver connect to the selected transfer relay and establish the existing M1 end-to-end encrypted session. Discovery messages do not carry file chunks. Failure may select another valid descriptor or relay; it must not silently fall back to direct transfer, a tracker or the public BitTorrent DHT.

### Bootstrap

Cached peers, user-provided contacts, peer exchange and several independent DNS/hard-coded seeds supply routing contacts only. A seed must not receive a content lookup or return authoritative provider data. Clients verify liveness and populate their own diverse routing tables before discovery.

## Privacy and security considerations

The design hides raw content IDs and descriptor contents from DHT infrastructure while the discovery capability remains limited. It separates requester IP from lookup key and provider IP from publication key only when relay and gateway do not collude. Stable lookup keys remain observable and repeated queries remain linkable at DHT nodes. Multiple paths increase the number of nodes seeing that derived key.

Transfer relays still see both endpoint addresses, timing, volume and duration. Local, ISP and large passive observers may correlate all stages. A leaked or public capability permits membership and dictionary observation. This RFC makes no anonymity or global-observer claim.

Signed records limit undetected modification but not censorship, malicious providers or false relay availability. Prefix diversity, conservative routing-table replacement, replication, independent paths and rate limits raise attack cost but do not solve Sybil or eclipse attacks. Endpoint-bound IDs similar to BEP 42 and proof-of-work remain experiments, not requirements.

## Compatibility

Kademlia XOR routing, compact records and BEP 42/44 concepts provide an evolutionary path for BitTorrent-capable clients. The discovery capability and encrypted descriptor are TripTorrent extensions. A client may operate classic BitTorrent and TripTorrent tables side by side, but it must not place TripTorrent private-discovery keys into the public BitTorrent DHT without explicit user selection.

M1 transfer framing remains unchanged. M2 remains available as local prototype scaffolding while this direction is tested.

## Alternatives

- Vanilla Kademlia leaks raw content and both endpoint associations to target nodes.
- Blinded keys alone hide the raw ID but preserve endpoint linkage and targeted-Sybil failure.
- One oblivious rendezvous path reduces metadata but had 79% success with 20% malicious nodes and 75% under concentrated Sybils in the deterministic M3 scenarios.
- A central tracker recreates M2's trust concentration.
- Tor-style multi-hop circuits and cover traffic exceed this milestone's performance and implementation scope.
- PIR requires a separate database, deployment and cost experiment.

## Test plan

Before acceptance, implement a no-Internet prototype that verifies:

- domain-separated key derivation vectors;
- deterministic descriptor encoding, signature and AEAD vectors;
- expiry, replay, malformed-record and unsupported-version rejection;
- publication and lookup through non-colluding relay/gateway roles;
- three independent paths and diverse replica placement;
- failure without direct or public-DHT fallback;
- randomized 100/1,000-node Monte Carlo simulations across malicious-node and attacker-prefix counts;
- preservation of all M1 and M2 tests.

## Migration and versioning

All fields require an experimental version. Capability links need a new, explicitly private-discovery namespace. M2 advertisements and control messages are not automatically translated. Deployment should begin as an opt-in parallel discovery mode, with downgrade and fallback prohibited unless explicitly selected.
