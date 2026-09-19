# TripTorrent

> **BitTorrent + onion-style privacy = TripTorrent.**

TripTorrent is an open peer-to-peer protocol evolved from BitTorrent. Its goal is to preserve the parts that made BitTorrent exceptionally effective — swarms, piece-based transfer, content addressing, decentralization and efficient distribution — while redesigning discovery, peer identity, routing and transport for stronger privacy, security and resilience.

TripTorrent is **a protocol, not a client**. The reference implementation in this repository exists to prove interoperability and help other implementations get started. qBittorrent, Transmission, Deluge, NAS software, mobile clients, libraries and independent implementations should all be able to speak TripTorrent without depending on this codebase.

The conceptual relationship is closer to **Kademlia relative to eD2k** than to “a new torrent client”: TripTorrent is intended as an evolution of the network/protocol layer that can revitalize an existing P2P model rather than replace it with one specific application.

## Status

**Pre-alpha / protocol design.**

There is no stable wire protocol yet and there are currently **no security or anonymity guarantees**. Do not rely on TripTorrent for privacy-sensitive use until the threat model, protocol, independent review and interoperability tests are substantially more mature.

## Current prototype

M3 research is complete and recommends that the next prototype test capability-keyed Kademlia records, oblivious relay/gateway lookup paths, diverse replication and separate rendezvous coordination. The [research report](docs/M3_DISCOVERY_RESEARCH.md), [RFC 0002](rfcs/0002-separated-multi-stage-discovery.md) and [ADR 0004](docs/adr/0004-m3-separated-discovery-research.md) state the assumptions and limits. M3 does not implement a production DHT or establish an anonymity guarantee.

The current executable remains M2. It extends the encrypted M1 transfer into a local multi-node overlay. Peers advertise experimental content IDs under short leases, relays advertise availability, and a temporary bootstrap selects a provider and relay and creates an automatic route. The receiver needs only the bootstrap address and content ID; peers still connect exclusively through a relay.

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

The content-ID format, chunking, overlay messages, leases, ephemeral identities, bootstrap, selection policy and Noise pattern remain experimental. M2 does not implement a DHT, multi-hop routing, traffic-analysis resistance, BitTorrent compatibility, NAT traversal or production hardening. M3 adds only deterministic no-I/O discovery research tooling:

```bash
cargo run -p triptorrent-overlay --example m3_discovery_simulation
```

Manual M1 `--relay`, `--route` and `--key` commands remain available for regression testing.

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
