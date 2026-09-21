# Changelog

All notable project changes will be documented here.

## Unreleased

### Added
- Initial project structure and design documentation.
- M1 minimal relayed transfer prototype.
- Manual validation record for the first successful TripTorrent transfer on 2026-09-19.
- M2 temporary bootstrap overlay with leased discovery, automatic relay selection, failover and multi-node simulation.
- M3 decentralized-discovery research, adversarial simulation, metadata analysis, RFC and ADR selecting a separated multi-stage prototype direction.
- M4 concurrent swarm transfer with multiple providers, compact piece availability, rarest-first scheduling, transfer-local failover penalties, verified resume state, bandwidth limits and process-level adversarial acceptance tests.
- M5 persistent node crate with a loopback HTTP+JSON API, bearer-token mutations, managed content storage, SQLite metadata, TOML configuration, structured logs, restart recovery and process-level daemon acceptance tests.
- M6 BitTorrent interoperability research with namespaced identifier and explicit network-mode architecture, strict offline v1/v2/hybrid and magnet prototypes, deterministic vectors, threat analysis, RFC and ADR.
- M7 native `egui`/`eframe` reference desktop client, shared typed node API client, connect-or-start daemon lifecycle, content management, verified transfer telemetry, persistent download pause/resume, truthful privacy status and process-level acceptance tests.
- M8 adversarial validation with bounded protocol/parser inputs, hardened relay/bootstrap/API/storage state, property and optional fuzz coverage, experimental conformance vectors, reproducible Sybil/eclipse Monte Carlo runs, traffic-metadata analysis and a classified findings/M9 gate report.
- M9 implementation-ready Testnet v1 profile with explicit binary encoding and network negotiation, normative specification, fixed conformance vectors, independent Python receiver and cross-language transfer, canonical test artifact, probe, operator packaging and release workflows. Public deployment validation remains pending.
