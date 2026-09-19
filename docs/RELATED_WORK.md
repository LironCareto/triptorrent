# Related Work

TripTorrent's distinguishing design goal is a client-independent protocol evolution with separable privacy and compatibility layers. It is not merely an anonymous torrent client. The projects below solve related problems with different boundaries; the comparison describes architecture rather than ranking them.

## Tribler

**Primary goal.** [Tribler](https://www.tribler.org/) is a decentralized BitTorrent client focused on searching, sharing, and downloading without a central website. Its [anonymity specification](https://github.com/Tribler/tribler/wiki/Anonymous-Downloading-and-Streaming-specifications) describes its Tor-inspired routing, separate network, and known Sybil limitations.

**Relationship and type.** Tribler is an application/client with its own decentralized overlay and anonymity network around BitTorrent workflows. It speaks to the BitTorrent ecosystem while adding Tribler-specific discovery and routed transfer behavior.

**Discovery.** Tribler uses a decentralized content/community overlay rather than depending on a central torrent index. Classic torrent metadata and swarms can still be part of a download.

**Routing and anonymity.** Traffic can traverse layered circuits, including exit nodes for classic BitTorrent access and hidden seeding for peers participating through Tribler. Circuit participants have role-specific visibility, and the project does not present this as complete protection from every adversary.

**Compatibility and network boundary.** Unmodified BitTorrent peers can participate on the classic swarm side through exits, but they do not implement Tribler's overlay or anonymity protocol. Tribler therefore operates a distinct overlay while interoperating with ordinary swarms at its boundary.

**Similarity to TripTorrent.** Both retain torrent distribution concepts, seek decentralized discovery, route transfers to reduce direct endpoint exposure, and explicitly account for metadata leakage.

**Architectural difference.** Tribler delivers these properties through a particular client and its overlay. TripTorrent specifies a protocol intended for independent implementations and keeps native privacy discovery/transport separate from an optional classic compatibility adapter. TripTorrent's current M3 direction also separates discovery relays/gateways from transfer relays rather than defining one deployed onion-circuit system.

## BitTorrent over I2P

**Primary goal.** [I2P](https://i2p.net/en/docs/overview/intro/) is a general-purpose anonymous overlay network based on cryptographic Destinations and unidirectional tunnels. [BitTorrent over I2P](https://i2p.net/en/docs/applications/bittorrent/) adapts torrent applications, including I2PSnark and SAM-based clients, to operate inside that network.

**Relationship and type.** I2P is a network; BitTorrent over I2P is a transport/tracker adaptation of the standard BitTorrent peer protocol. Peers use I2P Destinations instead of public IP address and port endpoints, and I2P trackers return destination hashes.

**Discovery.** I2P-local trackers and, where supported, I2P-specific peer discovery locate peers. The official guidance tells clients to ignore non-I2P announce URLs for anonymous operation, preventing an accidental clearnet path.

**Routing and anonymity.** I2P tunnels carry connections between Destinations. This hides ordinary endpoint addressing from torrent peers but retains I2P's own threat assumptions, routing metadata, and traffic-analysis limits.

**Compatibility and network boundary.** The BitTorrent peer protocol is substantially reused, but an unmodified clearnet client cannot connect to an I2P Destination. A client needs built-in I2P support or a SAM integration. I2P torrents therefore form a separate swarm/network unless an explicit gateway bridges them; uTP is not the normal I2P transport.

**Similarity to TripTorrent.** Both separate application content semantics from an endpoint-hiding network path and require fail-closed handling to avoid clearnet leaks. Both can reuse torrent metadata while changing discovery/addressing and transport.

**Architectural difference.** I2P supplies a general anonymous network beneath adapted BitTorrent clients. TripTorrent defines its own evolvable discovery, relay, identity, and swarm layers, plus explicit BitTorrent aliases and compatibility modes. TripTorrent does not currently depend on a universal anonymity substrate and makes no production anonymity claim.

## Design implication

Tribler shows that a client can bridge private routing and large classic swarms; I2P shows that BitTorrent semantics can run over a separate privacy network with strict network isolation. TripTorrent adopts the shared lesson that compatibility paths must be explicit. Its separate identifier namespaces and adapters allow native protocol implementations to evolve without requiring one client or treating classic BitTorrent networking as privacy-preserving.
