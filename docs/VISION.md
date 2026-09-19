# Vision

TripTorrent is a new open P2P protocol evolved from BitTorrent.

Its purpose is to preserve BitTorrent's strongest ideas — swarming, decentralised distribution, verifiable pieces and content addressing — while redesigning the network layer around stronger privacy, security and resilience.

## Core idea

TripTorrent should be to BitTorrent what Kademlia was to eD2k: a protocol-level evolution that can revitalise an existing P2P ecosystem without binding it to one client.

## Principles

1. Protocol first, client agnostic.
2. Open specification and public development.
3. Privacy and security are architectural concerns, not optional plug-ins.
4. No custom cryptography where established constructions exist.
5. Interoperability must be testable.
6. Performance matters: privacy mechanisms must be designed for high-volume P2P traffic.
7. Compatibility with BitTorrent is useful only when it does not force inheritance of unwanted security properties.
8. Claims about anonymity or security must be evidence-based and threat-model specific.

## Success

TripTorrent succeeds when multiple independent implementations can interoperate using the public specification, and users can participate in efficient swarms with materially better protection of peer identity and metadata than classic BitTorrent.
