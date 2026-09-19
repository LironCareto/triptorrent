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

## M1 — Minimal private transfer
- two peers
- one relay
- encrypted session
- chunked transfer
- integrity verification
- no direct peer-to-peer endpoint exposure between the two peers

## M2 — Multi-node overlay
- peer discovery
- rendezvous
- relay selection
- reconnects
- churn handling
- simulation harness

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
