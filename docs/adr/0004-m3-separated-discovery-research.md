# ADR 0004: Prototype separated multi-stage discovery next

Status: accepted as research direction

## Context

The M2 bootstrap learns requester, content, provider, relay and timing metadata in one place. M3 compared vanilla Kademlia, capability-derived keys, distributed encrypted rendezvous records and multi-stage discovery. A deterministic 100/1,000-node simulator measured routing, exposure, churn, malicious nodes and target-key Sybils.

Blinded keys removed raw-ID exposure but retained requester/provider linkage and failed under target occupation. A single oblivious rendezvous path reduced linkage but lost availability under malicious nodes. Three diverse paths and eight prefix-diverse replicas reached 100% success with 20% randomly malicious nodes and 99% with 250 concentrated Sybils in the modeled topology, at higher control-message and lookup-key exposure cost.

## Decision

The next discovery prototype will evaluate separated multi-stage discovery as described by RFC 0002:

- random discovery capabilities distinct from content IDs;
- capability-derived DHT keys and encrypted signed descriptors;
- OHTTP/HPKE-style relay and gateway separation for publication and lookup;
- diverse replicas and three independent lookup paths;
- opaque rendezvous tokens leading to the existing M1 relayed transfer;
- bootstrap sources used only to populate routing tables.

M2 is not replaced. The decision selects an experiment, not a production protocol or anonymity claim.

## Alternatives considered

Vanilla Kademlia, blinded direct Kademlia, one-path distributed rendezvous, Tor-style introduction/rendezvous, trackers, gossip and PIR were considered. The research report records their leakage, cost and scope trade-offs.

## Consequences

No mandatory service needs the complete requester-to-content-to-provider mapping when roles do not collude. DHT nodes still observe stable derived keys, transfer relays still observe both endpoints and broad traffic observers may correlate stages. Multi-path lookup increases control traffic and key exposure. Prefix diversity only raises Sybil cost and may disadvantage NATed networks.

The next implementation must first validate real relay/gateway separation, record formats and randomized adversarial behavior. It must not introduce multi-source swarm transfer, a production DHT, automatic public-DHT fallback or unsupported anonymity language.
