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

## Security rule

No protocol component should make an anonymity claim stronger than what the threat model and implementation can support.
