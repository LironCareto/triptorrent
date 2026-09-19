# Roadmap

TripTorrent is currently in design/pre-alpha.

## M0 — Foundation
- project vision
- architecture v0
- threat model v0
- RFC process
- Rust workspace
- CI
- initial protocol terminology

## M1 — Minimal relayed transfer (implemented prototype)
- [x] two peers
- [x] one local relay
- [x] end-to-end encrypted session with a pre-shared key
- [x] deterministic chunked transfer
- [x] chunk and complete-content integrity verification
- [x] no direct peer-to-peer connection between the two peers

M1 validates the executable vertical slice only. Its content IDs, wire format, route registration, key distribution and session setup are experimental rather than normative. It makes no anonymity claim: the relay and network observers still learn connection metadata, timing and traffic volume.

## M2 — Multi-node overlay (implemented prototype)
- [x] leased peer and content advertisements
- [x] temporary bootstrap/rendezvous service
- [x] leased relay advertisements and deterministic selection
- [x] automatically coordinated relay routes
- [x] disconnect, expiry, reconnect and relay failover handling
- [x] deterministic 10-peer/3-relay simulation harness
- [x] discovery-driven encrypted M1 transfer

M2 proves local overlay mechanics only. The centralized bootstrap, control messages, ephemeral peer identifiers, lease values and selection policy are experimental. Anonymous and decentralized discovery remain M3 research topics.

## M3 — Anonymous discovery research
- DHT/overlay candidate evaluation
- metadata leakage analysis
- eclipse/Sybil analysis
- bootstrap strategy

## M4 — Swarm transfer
- multiple sources
- piece availability
- scheduling
- resume
- bandwidth controls
- malicious-piece handling

## M5 — Persistent node
- daemon
- local API
- storage/index
- configuration
- observability

## M6 — BitTorrent interoperability research
- magnet/infohash mapping
- import/export semantics
- optional compatibility layer
- explicit separation from TripTorrent privacy guarantees

## M7 — Reference desktop client
- add content
- transfer list
- progress
- pause/resume
- network/privacy status

## M8 — Adversarial testing
- fuzzing
- protocol conformance
- relay abuse
- Sybil/eclipse simulation
- traffic-analysis experiments

## M9 — Public testnet
- versioned wire protocol
- independent implementation test
- public conformance suite
