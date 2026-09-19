# RFC 0001: Experimental M2 bootstrap overlay

- Status: draft
- Authors: TripTorrent contributors
- Created: 2026-09-19

## Summary

Define the temporary, local-only control messages used to prove M2 membership, content discovery, relay selection, leases and churn. This RFC records experimental behavior; it does not propose the final decentralized discovery architecture.

## Motivation

M1 requires users to coordinate a relay, route ID and PSK manually. M2 needs an executable multi-node topology before M3 evaluates DHT and anonymous-discovery candidates.

## Specification

A bootstrap holds leased peer and relay advertisements in memory. Peers register an ephemeral ID derived from their advertised Noise public key, experimental transfer capabilities and content IDs. Relays register an ID and reachable socket address. Discovery selects the lexicographically first live provider and relay, creates a route ID, returns the assignment to the receiver and queues it for the provider. Heartbeats renew leases; expired state and dependent routes are removed. A reported relay failure removes that relay and its queued routes.

All requests and responses carry experimental protocol version 0 and use Postcard inside a 32-bit length-prefixed TCP frame. M2 transfer sessions use `Noise_KN_25519_ChaChaPoly_BLAKE2s` and otherwise reuse M1 file-transfer messages and relay framing.

## Privacy and security considerations

The bootstrap sees peer IPs, identifiers, public keys, content advertisements and queries, relay endpoints, assignments and timing. Control messages are unauthenticated. A malicious bootstrap can replace public keys, poison discovery and remove relays. Deterministic selection leaks policy and is not privacy optimized. M2 makes no anonymity or Sybil-resistance claim.

## Compatibility

Manual M1 routes and PSK sessions remain supported. No BitTorrent identifier, tracker, DHT or client interoperability behavior is introduced.

## Alternatives

A DHT is deferred to M3. Bootstrap-distributed symmetric keys were rejected because they would reveal session secrets to the bootstrap. Manual PSKs were retained only for the M1 compatibility path.

## Test plan

Test registration, advertisement, discovery, route coordination, expiry, reconnect, relay failover, a 10-peer/3-relay simulation, end-to-end discovered transfer and simultaneous independent routes.

## Migration and versioning

There is no compatibility promise for version 0. M3 may replace the bootstrap, encoding, identity binding, lease model and selection policy entirely.
