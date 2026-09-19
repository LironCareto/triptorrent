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

Peers reveal their network address to the bootstrap and selected relay. They do not open direct sockets to each other. Network observers can correlate bootstrap activity with subsequent relay connections by timing and volume. M2 makes no anonymity, unlinkability, Sybil-resistance or malicious-bootstrap-resistance claim; these questions were deferred to M3 research.

## M3 discovery research findings

M3's recommended prototype separates discovery roles so that, without collusion:

- a query relay sees the requester endpoint but not the HPKE-protected lookup;
- a query gateway and queried DHT nodes see a capability-derived key but not the requester endpoint;
- DHT storage nodes see an encrypted descriptor rather than a raw content ID, provider endpoint or relay choice;
- bootstrap seeds see join traffic but receive no content lookup;
- a transfer relay still sees both transfer endpoints, route token, timing and volume.

These properties depend on the secrecy of the discovery capability and on query relay/gateway non-collusion. DHT nodes can link repeated use of the same derived key. Public capabilities permit recognition by anyone holding the link. Malicious gateways can censor results, target-key Sybils can occupy routing and storage positions, and prefix diversity is ineffective against an attacker with enough network origins. Signed records prevent undetected modification but do not prevent withholding, malicious providers or false relay advertisements.

Local and transit observers can correlate discovery and transfer timing. A colluding query relay and gateway recover requester endpoint plus lookup key; collusion with storage or transfer roles may add provider or content associations. A large passive observer remains outside the protection claim. No padding, cover traffic, PIR, production identity cost or onion routing is implemented.

The deterministic M3 model found that three diverse lookup paths improved availability under its malicious-node and concentrated-Sybil scenarios, but increased control traffic and the number of nodes seeing the derived key. These comparative results are not an Internet-scale security estimate. See [M3 Discovery Research](M3_DISCOVERY_RESEARCH.md) for assumptions, matrices and measurements.

## M4 swarm security and metadata

M4 verifies each received chunk against one agreed manifest and verifies the completed bytes against the requested content ID. Corrupt data, wrong indices, false availability and mid-transfer disconnects disable that provider only for the current transfer; another source may retry the chunk. Resume metadata is bound to the exact manifest, and every recorded completed chunk is revalidated from disk after restart. These checks protect integrity but do not prove that a provider is available, prevent resource exhaustion or create durable reputation.

Using several sources broadens metadata exposure. The M2 bootstrap sees the full provider set and every route assignment. Each selected relay sees the endpoint pair, route, timing, frame sizes and duration for its session. Providers learn the requested content and requested chunk indices; availability bitfields reveal which pieces each provider holds to the receiver. Noise continues to hide application plaintext from a non-colluding relay, but traffic volume and piece-sized timing remain observable. M4 introduces no new anonymity claim and does not implement the M3 discovery architecture.

## Security rule

No protocol component should make an anonymity claim stronger than what the threat model and implementation can support.
