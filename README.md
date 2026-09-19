# TripTorrent

> **BitTorrent + onion-style privacy = TripTorrent.**

TripTorrent is an open peer-to-peer protocol evolved from BitTorrent. Its goal is to preserve the parts that made BitTorrent exceptionally effective — swarms, piece-based transfer, content addressing, decentralization and efficient distribution — while redesigning discovery, peer identity, routing and transport for stronger privacy, security and resilience.

TripTorrent is **a protocol, not a client**. The reference implementation in this repository exists to prove interoperability and help other implementations get started. qBittorrent, Transmission, Deluge, NAS software, mobile clients, libraries and independent implementations should all be able to speak TripTorrent without depending on this codebase.

The conceptual relationship is closer to **Kademlia relative to eD2k** than to “a new torrent client”: TripTorrent is intended as an evolution of the network/protocol layer that can revitalize an existing P2P model rather than replace it with one specific application.

## Status

**Pre-alpha / protocol design.**

There is no stable wire protocol yet and there are currently **no security or anonymity guarantees**. Do not rely on TripTorrent for privacy-sensitive use until the threat model, protocol, independent review and interoperability tests are substantially more mature.

## Goals

- Preserve efficient swarm-based distribution.
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
- Hiding protocol decisions inside the reference implementation.
- Promising “perfect anonymity”.
- Sacrificing all throughput merely to resemble Tor.
- Preserving BitTorrent compatibility when doing so would force TripTorrent to inherit a security weakness.

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
