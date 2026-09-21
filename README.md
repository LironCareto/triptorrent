# TripTorrent

> **BitTorrent + onion-style privacy = TripTorrent.**

TripTorrent is an open peer-to-peer protocol evolved from BitTorrent. Its goal is to preserve the parts that made BitTorrent exceptionally effective — swarms, piece-based transfer, content addressing, decentralization and efficient distribution — while redesigning discovery, peer identity, routing and transport for stronger privacy, security and resilience.

TripTorrent is **a protocol, not a client**. The reference implementation in this repository exists to prove interoperability and help other implementations get started. qBittorrent, Transmission, Deluge, NAS software, mobile clients, libraries and independent implementations should all be able to speak TripTorrent without depending on this codebase.

The conceptual relationship is closer to **Kademlia relative to eD2k** than to “a new torrent client”: TripTorrent is intended as an evolution of the network/protocol layer that can revitalize an existing P2P model rather than replace it with one specific application.

## Status

**Pre-alpha / controlled Testnet v1 implementation ready; public deployment pending.**

The implementation-independent [Testnet v1 specification](spec/testnet-v1.md) is frozen for the experimental network `triptorrent-testnet-1`. No public endpoints have yet been deployed or validated, and there are **no security or anonymity guarantees**. Do not use TripTorrent for privacy-sensitive transfers.

## Current prototype

M9 preparation adds an explicit language-neutral wire profile, fail-closed version/network negotiation, checked-in [conformance vectors](test-vectors/m9-testnet-v1.json), a genuinely independent [Python receiver](interop/python/README.md), cross-language relay transfer testing, deployment packages, release workflows, and a testnet probe. The public deployment gate remains open; see the [tester](docs/M9_TESTNET.md), [operator](docs/M9_OPERATOR_GUIDE.md), and [implementer](docs/M9_IMPLEMENTER_GUIDE.md) guides.

M8 completes adversarial validation across protocol, relay, bootstrap, local API, storage and offline interoperability boundaries. It adds explicit resource bounds, deterministic/property regressions, optional fuzz targets, seeded Sybil/eclipse simulations and a metadata-only traffic model. The result is **READY FOR M9** only as a controlled, clearly experimental testnet; it is not an anonymity or security claim. See the [M8 testing report](docs/M8_ADVERSARIAL_TESTING.md) and [finding log](docs/M8_FINDINGS.md).

M7 adds a native reference desktop client built with `egui`/`eframe`. It connects to the persistent node through the same typed, authenticated local API as the CLI, can start the node sidecar, manages the library, starts fetches, shows verified progress, and genuinely pauses/resumes downloads. Build the workspace and launch the two adjacent executables:

```bash
cargo build --workspace
target/debug/triptorrent-desktop
```

The first launch creates a safe per-user configuration without a bootstrap. Local content management remains available; configure a real bootstrap explicitly to enable fetches. Closing the window leaves the persistent node running, and the UI provides an explicit **Stop node** action. See the [M7 desktop guide](docs/M7_DESKTOP_CLIENT.md) and [ADR 0008](docs/adr/0008-m7-local-api-desktop-client.md).

M6 completes BitTorrent interoperability research without adding a production BitTorrent network stack. The selected design keeps TripTorrent content IDs independent, records verified v1/v2 infohashes as typed aliases, and places classic networking behind explicit `triptorrent-only`, dual-network, or classic policies. See the [M6 research report](docs/M6_BITTORRENT_INTEROP_RESEARCH.md), [RFC 0004](rfcs/0004-namespaced-bittorrent-compatibility.md), and [ADR 0007](docs/adr/0007-m6-namespaced-identifiers-and-explicit-compatibility.md).

M5 adds a persistent reference node around the M4 transfer engine. The `triptorrent-node` crate owns the daemon lifecycle, SQLite index, managed content store, recovery state and versioned loopback HTTP API. The CLI remains a front-end to that API, which also gives a future GUI a stable local boundary. [ADR 0006](docs/adr/0006-m5-persistent-node.md) and the [M5 node guide](docs/M5_PERSISTENT_NODE.md) describe the implementation.

Initialize and start a node after starting the temporary bootstrap and relays shown below:

```bash
triptorrent node init --config triptorrent.toml --data-dir ./data --bootstrap 127.0.0.1:7100
triptorrent node start --config triptorrent.toml
```

From another shell, persistent operations go through the local API:

```bash
triptorrent node status --config triptorrent.toml
triptorrent content add --config triptorrent.toml ./source.bin
triptorrent content list --config triptorrent.toml
triptorrent node fetch --config triptorrent.toml --content <CONTENT_ID>
triptorrent transfer list --config triptorrent.toml
triptorrent node stop --config triptorrent.toml
```

Imports are copied into content-addressed managed storage. Indexed content, completed downloads and verified M4 partial state survive restart; shared content is advertised again with fresh ephemeral protocol identity and routes. The API binds to loopback and requires its generated bearer token for changes. See the node guide for config precedence, storage layout, API routes and limitations.

M4 swarm transfer remains the network data path. A receiver discovers several providers through the temporary M2 bootstrap, opens an independent encrypted relay session to each, schedules verified 32 KiB chunks concurrently, retries failed or corrupt work, and resumes from validated local partial state. [RFC 0003](rfcs/0003-m4-swarm-transfer.md) and [ADR 0005](docs/adr/0005-m4-swarm-transfer.md) describe the experimental design.

The current executable retains M2 discovery as scaffolding rather than implementing the M3 DHT/OHTTP research architecture. Peers advertise experimental content IDs under short leases, and the bootstrap now returns routes to several providers. The receiver needs only the bootstrap address and content ID; every provider/receiver session still travels exclusively through a relay.

The bootstrap is a prototype rendezvous service, not the final decentralized discovery design. It observes peer IP addresses, ephemeral peer IDs and public keys, content advertisements and queries, relay endpoints, selected routes, timing and churn. It can substitute advertised keys because M2 has no durable identity or authenticated bootstrap protocol. M2 therefore provides no anonymity guarantee and must not be used for privacy-sensitive transfers.

Build with `cargo build --workspace`, then run the demo in separate terminals. Start the bootstrap and at least two relays:

```bash
cargo run -p triptorrent-cli -- bootstrap --listen 127.0.0.1:7100
cargo run -p triptorrent-cli -- relay --listen 127.0.0.1:7001 --bootstrap 127.0.0.1:7100 --id relay-a
cargo run -p triptorrent-cli -- relay --listen 127.0.0.1:7002 --bootstrap 127.0.0.1:7100 --id relay-b
```

Identify and share a file, then copy the printed ID into the fetch command:

```bash
cargo run -p triptorrent-cli -- id ./source.bin
cargo run -p triptorrent-cli -- share --bootstrap 127.0.0.1:7100 --file ./source.bin
cargo run -p triptorrent-cli -- fetch --bootstrap 127.0.0.1:7100 --content <CONTENT_ID> --output ./received.bin
```

Run the same `share` command on several peers to form a swarm. Providers can expose an inclusive subset such as `--available 0-7,12-15`; `--upload-limit` and receiver `--download-limit` accept bytes per second, with zero or omission meaning unlimited. Interrupted downloads retain `<output>.triptorrent-part` and `<output>.triptorrent-state`; a later identical fetch validates and reuses completed chunks, then removes the state after final content-ID verification.

The content-ID format, chunking, swarm messages, leases, ephemeral identities, bootstrap, selection policy and Noise pattern remain experimental. M4 does not add anonymity, a DHT, multi-hop routing, traffic-analysis resistance, BitTorrent compatibility, NAT traversal or production hardening. M3 remains deterministic no-I/O discovery research tooling:

```bash
cargo run -p triptorrent-overlay --example m3_discovery_simulation
```

Manual M1 `--relay`, `--route` and `--key` commands remain available for regression testing.

## Testing

`cargo test --workspace` is the complete milestone validation after installing `interop/python/requirements.txt`. It includes real child-process tests for the CLI, independent Python receiver, desktop controller, daemon/API lifecycle, authentication, persistent storage, restart recovery, pause/resume, verified progress, multi-source transfer, serving after restart and simultaneous upload/download. Manual UI or multi-terminal demos are optional debugging tools rather than acceptance requirements.

Run the public conformance entry points directly with `cargo test -p triptorrent-protocol --test testnet_v1_conformance` and `python interop/python/conformance.py`.

M8's seeded discovery and traffic experiments run with `cargo run -p triptorrent-overlay --example m8_monte_carlo` and `cargo run -p triptorrent-overlay --example m8_traffic_analysis`. Optional long fuzz campaigns are documented in the [M8 report](docs/M8_ADVERSARIAL_TESTING.md).

## Compatibility with the BitTorrent ecosystem

TripTorrent is intended to **revitalize BitTorrent, not abandon its ecosystem**.

A core design goal is to keep as much of today’s BitTorrent infrastructure useful as possible, end to end, including existing content identifiers, magnets, metadata, torrent files, swarms, trackers, DHT concepts, clients and distribution workflows wherever compatibility can be preserved without defeating TripTorrent’s security and privacy goals.

The preferred migration path is evolutionary:

- existing BitTorrent content should remain discoverable and reusable where technically possible;
- existing clients should be able to add TripTorrent support without having to become entirely different applications;
- existing publishing and distribution workflows should require as little disruption as possible;
- TripTorrent-native peers and infrastructure should coexist with classic BitTorrent infrastructure during adoption;
- protocol extensions should prefer backward-compatible or dual-stack operation when this does not create a privacy or security downgrade.

Compatibility is therefore a **first-class requirement**, not an afterthought. When compatibility and a core security property are in direct conflict, that trade-off must be explicit, documented and justified rather than silently breaking either side.

## Related projects

[Related work](docs/RELATED_WORK.md) compares TripTorrent with Tribler and BitTorrent over I2P. TripTorrent's specific goal is a client-independent protocol evolution with separable privacy and compatibility layers, rather than one anonymous torrent client.

## Goals

- Preserve efficient swarm-based distribution.
- Preserve as much of the existing BitTorrent ecosystem and infrastructure as practical.
- Keep content verifiable and content-addressed.
- Make privacy and security protocol properties rather than optional client add-ons.
- Reduce direct exposure of peer network identity where practical.
- Decentralize discovery and routing without creating a new central dependency.
- Design for hostile networks, churn, NATs and partial failure.
- Remain implementable by independent clients.
- Publish a clear specification, test vectors and conformance tests.
- Reuse proven cryptographic constructions and libraries; **do not invent cryptography**.

## Non-goals

- Becoming a proprietary network tied to one client.
- Replacing the BitTorrent ecosystem merely for the sake of creating something new.
- Hiding protocol decisions inside the reference implementation.
- Promising “perfect anonymity”.
- Sacrificing all throughput merely to resemble Tor.
- Preserving a specific BitTorrent behaviour when it would necessarily defeat a core TripTorrent security property.

## Repository layout

```text
triptorrent/
├── crates/              # Rust reference implementation
├── spec/                # protocol specification
├── rfcs/                # protocol change proposals
├── docs/                # architecture, threat model and design notes
├── test-vectors/        # implementation-independent vectors
└── .github/             # CI and project templates
```

## Design rule

The specification is authoritative. The reference implementation is **one implementation of TripTorrent**, not the definition of TripTorrent.

Protocol changes that affect interoperability should be documented before they become de facto behaviour.

## Project documents

- [Vision](docs/VISION.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Threat model](docs/THREAT_MODEL.md)
- [M3 discovery research](docs/M3_DISCOVERY_RESEARCH.md)
- [M5 persistent node](docs/M5_PERSISTENT_NODE.md)
- [M6 BitTorrent interoperability research](docs/M6_BITTORRENT_INTEROP_RESEARCH.md)
- [M7 reference desktop client](docs/M7_DESKTOP_CLIENT.md)
- [M8 adversarial testing](docs/M8_ADVERSARIAL_TESTING.md)
- [M8 findings](docs/M8_FINDINGS.md)
- [Related work](docs/RELATED_WORK.md)
- [Design principles](docs/DESIGN_PRINCIPLES.md)
- [Roadmap](ROADMAP.md)
- [Protocol specification](spec/README.md)
- [RFC process](rfcs/README.md)
- [Governance](GOVERNANCE.md)
- [Security policy](SECURITY.md)
- [Contributing](CONTRIBUTING.md)

## Implementation language

The reference implementation is planned in **Rust**. This is an implementation choice, not a protocol requirement.

## License

Licensed under the [Apache License 2.0](LICENSE).

---

TripTorrent is an experimental, general-purpose P2P networking project. Its design will evolve in public.
