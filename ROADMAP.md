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

M2 proves local overlay mechanics only. The centralized bootstrap, control messages, ephemeral peer identifiers, lease values and selection policy are experimental. Anonymous and decentralized discovery were deferred to M3 research.

## M3 — Anonymous discovery research (completed)
- [x] Kademlia and alternative overlay candidate evaluation
- [x] per-role metadata leakage model and explicit privacy metrics
- [x] deterministic 100/1,000-node churn, malicious-node and target-Sybil simulation
- [x] eclipse/Sybil mitigations and limitations
- [x] non-authoritative bootstrap strategy
- [x] RFC and ADR selecting separated multi-stage discovery for the next prototype

M3 recommends capability-derived DHT keys, encrypted signed rendezvous records, oblivious relay/gateway paths, diverse replication and separate M1 relay coordination. This is a research decision, not a production DHT or anonymity guarantee. OHTTP role separation, descriptor formats and randomized attacker-prefix experiments must be validated before replacing M2.

## M4 — Swarm transfer (implemented prototype)
- [x] multi-provider discovery over the temporary M2 bootstrap
- [x] compact per-session piece availability
- [x] bounded concurrent rarest-first scheduling and provider failover
- [x] transfer-local verified resume state
- [x] receiver download and provider upload limits
- [x] corrupt, wrong, unavailable and failed-piece rejection
- [x] Windows-safe process-level CLI acceptance tests

M4 uses the current M2 discovery and relay services only as replaceable scaffolding. It does not implement the M3 DHT/OHTTP research direction, persistent-node storage or a global reputation system.

## M5 — Persistent node (implemented prototype)
- [x] long-running daemon with concurrent provider and download workers
- [x] versioned loopback HTTP+JSON control API with bearer-token mutations
- [x] managed content-addressed files and versioned transactional SQLite index
- [x] TOML configuration with CLI, environment, file and default precedence
- [x] structured logs, health/status counters and diagnostics
- [x] restart recovery for indexed sharing and verified partial downloads
- [x] Windows-safe process-level daemon acceptance tests

M5 is a reference-implementation runtime boundary, not a wire-protocol change. It continues to use M2 bootstrap/relay scaffolding and M4 swarm mechanics. OS service installers, the M3 discovery design, BitTorrent interoperability and a GUI remain later work.

## M6 — BitTorrent interoperability research (completed)
- [x] namespaced v1, v2, hybrid and TripTorrent identity mapping
- [x] strict `.torrent` and magnet import/export semantics
- [x] discovery, transfer, downgrade and threat matrices
- [x] optional dual-network adapter and bridge boundaries
- [x] deterministic offline interoperability vectors
- [x] explicit separation from TripTorrent privacy guarantees

M6 selects an import-first design with verified BitTorrent aliases and an optional, isolated classic adapter. TripTorrent-only transfers fail closed; importing classic metadata never enables classic networking. This is a research architecture and parser/vector prototype, not production tracker, DHT, peer-protocol, bridge or dual-network support.

## M7 — Reference desktop client (implemented prototype)
- [x] native Rust desktop client using the authenticated M5 local API
- [x] connect-or-start daemon lifecycle with readiness checks and reconnect backoff
- [x] persistent content library import, removal and managed-copy deletion
- [x] fetch workflow and transfer details with verified progress
- [x] real download pause/resume with paused state preserved across restart
- [x] truthful structured network/privacy and diagnostic status
- [x] Windows-safe process-level desktop/controller acceptance tests

M7 keeps the GUI, typed API client and daemon/runtime as separate crates. It reports the currently implemented M2 bootstrap and encrypted M4 relay path without claiming anonymity. Installers, OS services, production M3 discovery and M6 classic/dual networking remain future work.

## M8 — Adversarial testing (completed)
- [x] deterministic parser regressions, property tests and optional fuzz targets
- [x] experimental implementation-conformance vectors and state-machine adversaries
- [x] bounded relay, bootstrap, local API, storage and interoperability abuse tests
- [x] fixed-seed Monte Carlo Sybil/eclipse simulation across four candidates
- [x] metadata-only traffic-analysis experiment and collusion matrix
- [x] classified finding log and explicit M9 gate

M8 fixed concrete fail-closed and resource-bound defects without redesigning the selected protocol. It recommends proceeding only to a controlled, clearly labelled M9 testnet. M2 spoofing, Sybil/eclipse exposure, metadata correlation and the limits of synthetic research remain explicit risks; no security or anonymity guarantee is made.

## M9 — Public testnet
- versioned wire protocol
- independent implementation test
- public conformance suite
