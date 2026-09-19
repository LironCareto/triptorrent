# Threat Model (v0)

TripTorrent is pre-alpha. This threat model is a starting point and does not imply current protection.

## Assets

TripTorrent may aim to protect:

- peer network address from other peers;
- association between a peer and requested content;
- association between a peer and published content;
- content integrity;
- session confidentiality;
- resistance to malicious peers;
- availability of discovery and routing.

## Potential adversaries

- ordinary malicious peers;
- malicious relay nodes;
- Sybil attackers controlling many identities;
- local network observers;
- ISPs or transit observers;
- tracker/DHT observers;
- compromised bootstrap infrastructure;
- large passive observers with broad visibility;
- active adversaries able to delay, inject, replay or correlate traffic.

## Explicitly unresolved

TripTorrent does **not** currently claim resistance to a global passive adversary.

Traffic correlation, timing analysis, intersection attacks, malicious relays, DHT poisoning, eclipse attacks, Sybil attacks and resource exhaustion must be treated as first-class design problems.

## M2 bootstrap visibility and trust

The temporary M2 bootstrap directly observes:

- source IP addresses and timing of control connections;
- ephemeral peer identifiers and Noise public keys;
- advertised content IDs and which peer advertises them;
- queried content IDs and query timing;
- relay identifiers and reachable addresses;
- selected provider/relay relationships and route identifiers;
- registration, heartbeat, failure-report and expiry timing.

The bootstrap protocol is neither encrypted nor authenticated beyond local TCP assumptions. Registrations and relay-failure reports can be spoofed, overwritten or used for denial of service. The bootstrap distributes provider public keys but there is no durable identity or independent binding for them, so a malicious bootstrap can substitute keys and mediate a transfer. An honest relay alone still receives opaque Noise frames rather than file plaintext, but collusion and traffic correlation remain unresolved.

Peers reveal their network address to the bootstrap and selected relay. They do not open direct sockets to each other. Network observers can correlate bootstrap activity with subsequent relay connections by timing and volume. M2 makes no anonymity, unlinkability, Sybil-resistance or malicious-bootstrap-resistance claim; these questions are explicitly deferred to M3.

## Security rule

No protocol component should make an anonymity claim stronger than what the threat model and implementation can support.
