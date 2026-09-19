# M3 Discovery Research

Status: **research complete; prototype direction selected**
Date: **2026-09-19**

M3 asks how a peer can find content providers while revealing substantially less metadata than the M2 bootstrap. It does not claim anonymity and does not replace M2 networking. The result is a direction for the next discovery prototype: capability-keyed Kademlia records, oblivious query relays, independently operated gateways, diverse replication and a separate rendezvous stage.

## Threat assumptions

The design protects against an observer that occupies only one role and against a bounded fraction of malicious DHT nodes. It assumes at least one selected query relay and gateway do not collude, at least one diverse lookup path reaches an honest replica, and discovery capabilities are not globally published. These are conditional properties, not universal guarantees.

Adversaries are considered separately:

- ordinary and target-key DHT nodes observe the lookup key, request timing and their routing neighbors;
- malicious DHT nodes may poison routing, withhold records, return false contacts or correlate repeated keys;
- Sybil operators may create many identities, concentrate them near a key and occupy routing tables;
- query relays observe client endpoints and gateway choice, but should receive only HPKE ciphertext;
- query gateways observe lookup operations and relay endpoints, but not the original client endpoint;
- transfer relays observe both transfer endpoints, route tokens, timing, duration and volume, but not the discovery capability, raw content ID or M1 plaintext by protocol design;
- bootstrap seeds observe join-time endpoints and timing, but must not perform content lookup;
- local, ISP and transit observers see connections, sizes and timing and may correlate stages;
- colluding roles can combine their views; a query relay plus gateway defeats their separation;
- a large passive observer can correlate bootstrap, discovery and transfer traffic. M3 does not resist this adversary.

The model excludes endpoint compromise, capability leakage, cryptographic breaks and denial of Internet connectivity. Popular public content has a small effective ambiguity set: once its capability is public, an observer can join the same discovery namespace.

## Candidate architectures

### 1. Vanilla Kademlia provider DHT

Store provider endpoints under the raw content ID and perform iterative XOR-distance lookups, broadly following [Kademlia](https://www.scs.stanford.edu/~dm/home/papers/kpos.pdf) and BitTorrent [BEP 5](https://www.bittorrent.org/beps/bep_0005.html).

This is efficient, deployed and compatible with BitTorrent concepts. It removes M2's single complete database, but each queried node receives the requester's source address and raw content key. Nodes near the key receive provider announcements and can observe queries over time. A node near the target can therefore learn requester, content and provider. Replacing a tracker with vanilla Kademlia decentralizes control; it does not provide private discovery.

### 2. Kademlia with a capability-derived key

Use a random discovery capability from the content link and derive a DHT key with a domain-separated hash. DHT nodes see a stable pseudorandom key instead of the raw content ID. A deterministic hash of the public content ID alone would permit dictionary testing and is not considered blinded.

This reduces raw-ID disclosure while the capability remains limited. Direct lookup still exposes requester IP plus derived key, and direct provider records expose provider IP plus that key. Stable keys remain linkable. For public links, anyone with the link can derive the key, so the benefit is access-controlled metadata, not anonymity.

### 3. Distributed encrypted rendezvous records

Store an encrypted provider descriptor under the capability-derived key. Publish and retrieve it through a two-party oblivious transport patterned on [Oblivious HTTP](https://www.rfc-editor.org/rfc/rfc9458.html): a relay sees the client endpoint and ciphertext; a gateway sees the request and relay endpoint. The descriptor contains opaque rendezvous tokens rather than a provider network endpoint.

This separates requester location, lookup key and provider location when relay and gateway do not collude. One lookup path remains fragile: in the simulation it succeeds in only 79% of lookups with 20% randomly malicious nodes and 75% under the concentrated-Sybil scenario.

### 4. Separated multi-stage discovery

Extend candidate 3 with three independently selected relay/gateway paths, prefix-diverse routing and replica selection, signed expiring records, and a distinct rendezvous step:

```text
content link
  -> capability-derived DHT key
  -> three oblivious Kademlia lookups
  -> encrypted provider descriptors
  -> opaque rendezvous token and relay choice
  -> existing M1 encrypted relayed transfer
```

No DHT record contains a provider endpoint or plaintext relay choice. A provider publishes through the same relay/gateway separation. A receiver decrypts candidate descriptors, validates their signatures and expiry, then contacts a listed rendezvous relay. The provider and receiver continue to use M1 session and content verification; discovery does not carry file data.

This is the recommended prototype direction. It improves availability and role separation, but uses more messages, exposes the stable derived key to more DHT nodes, and still depends on non-collusion.

### 5. Onion-service-style introduction and rendezvous

The [Tor rendezvous protocol](https://spec.torproject.org/rend-spec/protocol-overview.html) demonstrates established separation of directory, introduction and rendezvous roles. Adopting Tor circuits could provide stronger endpoint hiding, but arbitrary multi-hop onion routing and cover traffic are outside M3. TripTorrent should retain the role separation and opaque rendezvous token, without claiming Tor's threat model or transplanting its protocol.

Central trackers, private information retrieval and fully replicated gossip were also considered. A tracker recreates M2's trust concentration. PIR could hide the selected record from a server, but deployment, multi-server assumptions and database cost require a separate experiment. Gossip maximizes disclosure and control traffic.

## Metadata exposure matrix

`Yes` means the role receives the item in normal operation. `Conditional` depends on capability knowledge, traffic correlation or collusion.

| Architecture | Raw content ID at DHT | Requester IP + lookup key | Provider IP + lookup key | Stable-query linkability | Relay choice at DHT | Single service maps requester -> content -> provider |
|---|---|---|---|---|---|---|
| M2 bootstrap | Yes | Yes | Yes | Yes | Yes | Yes |
| Vanilla Kademlia | Yes | Yes | Yes | Yes | Provider endpoint is direct | No single global service, but target nodes can |
| Blinded Kademlia | No while capability is secret | Yes | Yes | Yes | Provider endpoint is direct | Target nodes can link both sides under the derived key |
| Distributed rendezvous | No | No under relay/gateway non-collusion | No under publication relay/gateway non-collusion | Key only | No; descriptor is encrypted | Conditional on collusion |
| Separated multi-stage | No | No under relay/gateway non-collusion | No under publication relay/gateway non-collusion | Key only, across more paths | No; descriptor is encrypted | Conditional on collusion or traffic analysis |

For the recommended design:

| Observer | Raw ID | Lookup key | Requester IP | Provider IP | Requester/provider link | Other observations |
|---|---:|---:|---:|---:|---:|---|
| Ordinary queried DHT node | No | Yes | No | No | No | Gateway IP, timing, routing topology |
| Node storing the record | No | Yes | No | No | No | Encrypted descriptor, publication timing |
| Query relay | No | No | Yes | No | No | Gateway choice, ciphertext size and timing |
| Query gateway | No | Yes | No | No | No | Relay IP and DHT path |
| Join/bootstrap seed | No | No | Yes | No | No | Join timing and initial topology |
| Selected transfer relay | No by protocol | No | Yes | Yes | Yes | Route token, timing, volume, duration |
| Local/ISP observer | Conditional | Conditional | Yes | Conditional | Conditional | Destinations, sizes and timing |
| Colluding relay + gateway/storage | Conditional | Yes | Yes | Conditional | Conditional | Role separation can collapse |
| Large passive observer | Conditional | Conditional | Yes | Conditional | Conditional | Cross-stage timing correlation remains |

Provider and requester identity fields should be process-scoped keys, not durable user identities. Network endpoints remain separate from peer IDs. A self-signed descriptor proves continuity and detects modification; it does not prove that a provider is honest or authorized.

## Vanilla Kademlia leakage by phase

- **Advertisement:** the closest nodes receive the raw content key, provider endpoint, announcement timing and a token tied to the announcing address.
- **Iterative lookup:** every directly queried node receives requester source address, node ID, target content key and timing. Intermediate nodes can retain this even when they are not final record holders.
- **Retrieval:** target-near nodes return provider endpoints and can observe requester plus provider set together.
- **Repetition:** the stable content key, source endpoint and node ID make repeated interest linkable. Rotating a DHT node ID does not hide the network endpoint.
- **Topology:** routing RPCs reveal neighbor sets and allow active nodes to influence the next hop.

These properties follow from the direct KRPC model in BEP 5. Proxy guidance in [BEP 37](https://www.bittorrent.org/beps/bep_0037.html) also warns that DHT and peer ports create correlations.

## Sybil and eclipse analysis

Cheap identities can poison buckets, surround a target key, accept and discard provider records, return only attacker contacts, inject bogus provider records and generate new identities after eviction. Signed descriptors prevent undetected record modification; they do not prevent censorship, replay of unexpired records or attacker-authored providers.

The recommended prototype combines mitigations rather than treating one as sufficient:

- cap routing and replica entries per IPv4 `/24`, IPv6 `/48` and, where available, network origin; this is a heuristic and can penalize NATed users;
- prefer responsive, long-lived contacts and use conservative Kademlia replacement caches;
- require independent lookup paths with distinct query relays, gateways and network-prefix groups;
- replicate records across failure domains rather than only the numerically closest identities;
- validate signatures, sequence numbers and short expirations before using descriptors;
- rate-limit stores and expensive responses per source and capability key;
- compare independently obtained results, while recognizing that majority voting fails when identities are cheap;
- investigate an endpoint-bound node-ID rule similar to [BEP 42](https://www.bittorrent.org/beps/bep_0042.html), which raises the cost of targeted IDs but binds identity to network location and is not Sybil-proof;
- defer proof-of-work or other identity cost until measurements quantify required cost and mobile/NAT impact.

Prefix diversity does not defeat an attacker with many prefixes or ASes. Independent paths fail if routing tables are already eclipsed. Gateways can selectively drop or bias results. False relay advertisements still cause denial of service; receivers need multiple signed descriptors and must fail closed rather than switch to direct transfer.

## Bootstrap analysis

Joining the overlay and looking up content are separate operations. A bootstrap source returns candidate DHT contacts only. It must never receive a content key or return provider records as an authoritative service.

Clients should try, in order:

1. cached, recently responsive and prefix-diverse contacts;
2. user-provided peers or contacts embedded in a private content link;
3. peer exchange learned through existing authenticated sessions;
4. several independently operated DNS seed lists and hard-coded emergency seeds.

Clients populate and verify their own routing tables, query several sources, and discard seed authority after joining. DNS operators still learn join-time metadata; DNSSEC authenticates answers but does not hide queries or make contacts honest. Existing BitTorrent `nodes` fields and DHT contacts may help dual-stack migration, but a privacy-selected mode must not silently query the public BitTorrent DHT with a TripTorrent content key.

## Simulation methodology

The no-I/O model in `triptorrent_overlay::research` uses deterministic BLAKE3-derived 64-bit logical IDs, XOR distance, 64 Kademlia-style buckets, `k = 8`, `alpha = 3`, eight record replicas and at most 64 iterative rounds. Logical providers are distributed across live honest nodes and route each direct publication toward its content key. The model also includes nodes that fail after publication, malicious nodes that respond while withholding records and contacts, and Sybil IDs placed immediately around a target key. Prefix diversity permits one queried or storage node per simulated network group. The same logical requester repeats each content lookup.

The oblivious candidates assume relay/gateway non-collusion. A DHT gateway sees the key but not requester IP; a query relay sees requester IP but not the HPKE-protected request. Indirect records expose neither provider endpoints nor relay choices to DHT nodes. These are model inputs derived from role separation, not properties measured from packets.

Metrics are:

- lookup success at an honest live replica;
- maximum iterative rounds across parallel paths;
- distinct lookup-key and raw-ID observers;
- nodes seeing requester IP plus key or provider IP plus key;
- probability that one malicious node, or a colluding malicious set, observes both sides;
- repeated-key and repeated-requester linkability;
- replicated records, Sybil-held replicas, routing state and logical control messages.

Scenarios are deterministic single runs, not statistical confidence intervals:

- `clean-100`: 100 nodes, 20 content IDs, five lookups each;
- `churn-1000`: 1,000 nodes, 15% failed, 50 IDs, two lookups each;
- `malicious-1000`: 1,000 nodes, 20% malicious and 5% failed;
- `target-sybil-1000`: 1,000 ordinary nodes, 250 target-key Sybils across four groups, 5% ordinary malicious, 5% failed and 100 repeated lookups.

## Results

Values other than percentages are means per lookup or per content as indicated.

| Scenario | Candidate | Success | Rounds | Key observers | Raw-ID observers | Requester+key | Provider+key | Same node both | Colluding both | Messages |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| clean-100 | vanilla | 100.0% | 1.5 | 4.6 | 4.6 | 4.6 | 10.3 | 0.0% | 0.0% | 9.3 |
| clean-100 | blinded | 100.0% | 1.4 | 4.3 | 0.0 | 4.3 | 10.5 | 0.0% | 0.0% | 8.7 |
| clean-100 | rendezvous | 100.0% | 1.5 | 5.6 | 0.0 | 0.0 | 0.0 | 0.0% | 0.0% | 15.2 |
| clean-100 | multi-stage | 100.0% | 1.8 | 8.6 | 0.0 | 0.0 | 0.0 | 0.0% | 0.0% | 25.7 |
| churn-1000 | vanilla | 100.0% | 2.8 | 8.5 | 8.5 | 8.5 | 15.2 | 0.0% | 0.0% | 17.0 |
| churn-1000 | blinded | 100.0% | 3.3 | 9.9 | 0.0 | 9.9 | 16.8 | 0.0% | 0.0% | 19.8 |
| churn-1000 | rendezvous | 100.0% | 3.2 | 10.6 | 0.0 | 0.0 | 0.0 | 0.0% | 0.0% | 25.2 |
| churn-1000 | multi-stage | 100.0% | 3.7 | 14.4 | 0.0 | 0.0 | 0.0 | 0.0% | 0.0% | 36.8 |
| malicious-1000 | vanilla | 100.0% | 2.8 | 8.5 | 8.5 | 8.5 | 15.5 | 68.0% | 68.0% | 17.0 |
| malicious-1000 | blinded | 100.0% | 3.4 | 10.2 | 0.0 | 10.2 | 16.6 | 84.0% | 86.0% | 20.5 |
| malicious-1000 | rendezvous | 79.0% | 2.5 | 8.7 | 0.0 | 0.0 | 0.0 | 0.0% | 0.0% | 21.4 |
| malicious-1000 | multi-stage | 100.0% | 3.6 | 14.0 | 0.0 | 0.0 | 0.0 | 0.0% | 0.0% | 36.1 |
| target-sybil-1000 | vanilla | 0.0% | 6.0 | 18.0 | 18.0 | 18.0 | 23.0 | 100.0% | 100.0% | 36.0 |
| target-sybil-1000 | blinded | 0.0% | 6.0 | 18.0 | 0.0 | 18.0 | 26.0 | 100.0% | 100.0% | 36.0 |
| target-sybil-1000 | rendezvous | 75.0% | 2.9 | 9.7 | 0.0 | 0.0 | 0.0 | 0.0% | 0.0% | 23.4 |
| target-sybil-1000 | multi-stage | 99.0% | 3.9 | 14.8 | 0.0 | 0.0 | 0.0 | 0.0% | 0.0% | 37.7 |

| Scenario | Candidate | Replicas/content | Sybil replicas | Routing contacts/node | Repeated key | Repeated requester |
|---|---|---:|---:|---:|---:|---:|---:|
| clean-100 | all | 8.0 | 0.0 | 35.7 | 0.0% | 0.0% |
| churn-1000 | all | 8.0 | 0.0 | 62.5 | 0.0% | 0.0% |
| malicious-1000 | vanilla / blinded | 8.0 | 0.0 | 62.5 | 68.0% / 86.0% | 68.0% / 86.0% |
| malicious-1000 | rendezvous / multi-stage | 8.0 | 0.0 | 62.5 | 46.0% / 80.0% | 0.0% / 0.0% |
| target-sybil-1000 | vanilla / blinded | 8.0 | 8.0 | 70.7 / 71.3 | 100.0% | 100.0% |
| target-sybil-1000 | rendezvous / multi-stage | 8.0 | 4.0 | 71.3 | 100.0% | 0.0% |

The multi-stage candidate trades control traffic and broader key exposure for availability. Three paths expose a derived key to 8.6 nodes instead of 4.6 in the clean topology and use 25.7 rather than 9.3 messages. Under 20% malicious nodes, one rendezvous path falls to 79% while three paths reach 100% in this run. Under concentrated Sybils, direct closest-node replication puts all eight records on Sybils and discovery fails; diversity limits the four-group attacker to four replicas, and three paths reach 99%.

Blinding removes raw-ID exposure in the model but does not improve targeted-Sybil failure and does not prevent requester/provider linkage under the stable derived key. All candidates retain 100% repeated-key linkability in the targeted scenario. Oblivious transport removes repeated-requester linkage only under the non-collusion assumption.

## Recommendation

Prototype **separated multi-stage discovery** next:

1. A content link carries a random 32-byte discovery capability separately from the content ID.
2. Derive a lookup key and descriptor-encryption key with domain-separated HKDF/hash operations.
3. Publish short-lived, sequence-numbered, Ed25519-signed provider descriptors encrypted with a standard AEAD. Descriptors contain the content ID, process-scoped provider session key, capabilities and several opaque rendezvous tokens/relay candidates.
4. Store eight replicas through an oblivious relay/gateway path with network-prefix diversity.
5. Retrieve through three independently selected relay/gateway paths and merge valid descriptors.
6. Coordinate an M1 relay session through a selected opaque rendezvous token. Never fall back automatically to direct transfer or the public BitTorrent DHT.

Use existing constructions: [HKDF](https://www.rfc-editor.org/rfc/rfc5869), [Ed25519](https://www.rfc-editor.org/rfc/rfc8032), [ChaCha20-Poly1305](https://www.rfc-editor.org/rfc/rfc8439), [HPKE](https://www.rfc-editor.org/rfc/rfc9180) and the OHTTP role-separation model. Exact suites and encodings remain RFC work; no new cryptographic primitive is proposed. BEP 44 shows a deployed model for signed mutable DHT values, although TripTorrent descriptors need different privacy semantics.

This direction fits TripTorrent because discovery remains independent of M1 transfer, content capability remains distinct from peer identity, gateways and relays can be implemented independently, and Kademlia concepts preserve a plausible BitTorrent migration path. It does not require a central node to know the full mapping.

### Swarm and BitTorrent implications

Later swarm discovery should return a bounded set of provider descriptors; it should not publish per-piece availability in the DHT. M4 can select several validated providers and establish separate relayed sessions while keeping scheduling outside discovery. More sources increase availability and throughput, but also increase descriptor volume, relay bandwidth and the number of parties able to infer swarm membership. M3 does not choose that trade-off or implement it.

BitTorrent clients can reuse XOR routing, bucket maintenance, bootstrap contacts and concepts from BEP 42/44 in a parallel TripTorrent table. A future magnet extension can carry the discovery capability separately from v1/v2 content hashes. Classic DHT lookup remains useful only as an explicit compatibility mode because publishing a TripTorrent private lookup there would disclose the key and endpoints. No existing BitTorrent client or DHT packet is wire-compatible with the proposed descriptor yet.

## Honest claims

M3 supports these conditional statements:

- DHT nodes need not receive raw content IDs or provider endpoints.
- A query relay need not receive the lookup key, and a gateway need not receive the requester endpoint.
- No mandatory bootstrap service performs content lookup.
- Multiple diverse paths improve availability in the simulated attacks.

M3 does **not** establish:

- anonymity, untraceability or resistance to a global passive observer;
- protection when query relay and gateway collude;
- protection after a discovery capability becomes public;
- Sybil or eclipse resistance against an attacker with enough network diversity;
- traffic-analysis resistance, because there is no padding, batching or cover traffic;
- provider honesty, relay honesty, NAT reachability or production-scale performance;
- compatibility with today's BitTorrent DHT wire protocol.

## Limitations and unresolved questions

The simulator uses 64-bit logical IDs, idealized routing tables, one deterministic topology per scenario and no latency, NAT, bandwidth, packet size, adaptive corruption or real bootstrap process. Network groups are known perfectly. It does not implement HPKE, AEAD, signatures or rendezvous packets; it assigns their documented visibility properties. Results are comparative evidence, not deployment estimates.

The next prototype must answer:

- whether real OHTTP relay/gateway selection can be decentralized and rotated without creating a new authority;
- descriptor size, provider multiplicity, expiry and replay behavior;
- how prefix/ASN diversity behaves behind NAT, IPv6 privacy addresses and hostile multi-prefix operators;
- whether three paths and eight replicas remain the right values under randomized Monte Carlo topologies;
- whether endpoint-bound IDs, modest proof-of-work or neither provides an acceptable identity cost;
- how public magnet links represent the discovery capability without confusing it with the content ID;
- how provider polling and rendezvous avoid a central assignment service;
- what subset can interoperate with BEP 5/42/44 without leaking TripTorrent private-mode keys.

The smallest additional experiment is a packet-level local prototype of one publication and lookup through two independently operated OHTTP relays/gateways, combined with randomized Monte Carlo runs across attacker prefix counts. That should precede any production DHT replacement.

## Reproduction

```bash
cargo run -p triptorrent-overlay --example m3_discovery_simulation
cargo test -p triptorrent-overlay research
```
