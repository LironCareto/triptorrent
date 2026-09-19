# ADR 0002: M1 relayed transfer prototype

Status: accepted for prototype

## Context

M1 needs an executable two-peer transfer through one relay without establishing a direct peer connection or exposing file plaintext to the relay. The repository does not yet define normative content IDs, session establishment or wire encoding.

## Decision

Use fixed 32 KiB chunks and BLAKE3 digests for chunk integrity and an experimental whole-file content ID. Serialize versioned application messages with Postcard.

Each peer opens a framed TCP connection to a relay and registers a visible, caller-selected route ID and role. The relay pairs the endpoints and forwards frames without parsing TripTorrent application messages. It deliberately has no dependency on the content or protocol crates.

Protect peer-to-peer application frames with the reviewed Noise Framework implementation from the `snow` crate, using `Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s`. A 32-byte pre-shared key is distributed out of band for this local prototype and authenticates participants that possess it.

## Consequences

The prototype proves content requests, encrypted relaying, chunk verification and exact reconstruction while keeping transport, session and file protocol concerns separate. Peers never connect directly.

The relay still observes both endpoint addresses, route IDs, roles, frame sizes, timing and duration. The prototype has no key-distribution protocol, durable peer identity, replay policy, anonymity property, discovery, multi-hop routing or BitTorrent interoperability. The selected identifiers, chunk size, serialization, framing and Noise pattern are experimental and may be replaced through future RFCs.
