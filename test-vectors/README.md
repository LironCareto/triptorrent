# Test Vectors

This directory will contain implementation-independent test vectors for TripTorrent.

Vectors should be usable by any implementation language and should cover:

- encoding/decoding;
- content identifiers;
- handshake/session setup;
- authenticated encryption framing;
- routing messages;
- swarm messages;
- invalid and adversarial inputs.

No vectors are normative yet.

`m6-bittorrent-interop.json` contains deterministic synthetic research vectors for exact v1 and v2 infohash calculation, hybrid identification, repeated magnet topics, malformed canonical bencoding, cross-platform path sanitization, the current TripTorrent payload ID, and identical bytes with different BitTorrent identities. The `triptorrent-interop` crate consumes this file without network access. These vectors validate M6 conclusions; they do not define a stable TripTorrent wire format.
