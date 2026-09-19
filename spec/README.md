# TripTorrent Protocol Specification

Status: **draft / pre-alpha**

This directory will contain the normative TripTorrent protocol specification.

## Planned sections

1. Terminology
2. Version negotiation
3. Content identifiers and manifests
4. Peer identities
5. Session establishment
6. Discovery
7. Routing and relays
8. Swarm messages
9. Piece exchange
10. Error handling
11. Privacy properties
12. Security considerations
13. Extension mechanism
14. Compatibility considerations

Until a version is explicitly marked stable, all wire formats are subject to change.

## Experimental M2 bootstrap messages

M2 currently uses a non-normative, versioned Postcard request/response protocol over length-prefixed TCP connections. Requests cover:

- peer registration with an ephemeral peer ID, Noise public key, transfer capabilities and content IDs;
- peer heartbeat and route polling;
- relay registration and heartbeat with an identifier and TCP address;
- discovery by content ID;
- relay-failure reporting.

Responses acknowledge a server-controlled lease or return an optional route assignment. An assignment contains the provider ID and public key, selected relay ID and address, and an automatically generated route ID. Registrations and queued routes expire according to the bootstrap's monotonic lease clock.

These messages are prototype scaffolding, are not authenticated, and are not the normative TripTorrent discovery protocol. Their current encoding and semantics may be removed or replaced during M3.
