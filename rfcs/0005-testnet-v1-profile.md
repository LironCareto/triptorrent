# RFC 0005: TripTorrent Testnet v1 profile

- Status: accepted for controlled testnet
- Authors: TripTorrent contributors
- Created: 2026-09-20

## Summary

Freeze an implementation-independent Testnet v1 core/session/swarm protocol and a separately named centralized discovery profile. Require explicit protocol/network negotiation, no downgrade, public vectors, and cross-language transfer evidence.

## Motivation and specification

M9 needs external implementations and deployments without implying production maturity. The normative byte format, Noise roles, messages, state ordering, bounds, and failure behavior are defined in `spec/testnet-v1.md`. Version `1` and network `triptorrent-testnet-1` appear at all external boundaries. Discovery is `m2-testnet-bootstrap-v1`; it yields provider key, relay endpoint, and opaque route to the reusable transfer layer.

## Privacy and security

This profile preserves encrypted relayed transfer and no direct peer connection. It does not solve bootstrap observation/spoofing, relay metadata, traffic correlation, Sybil/eclipse attacks, or global observation. It is unsuitable for privacy-sensitive content.

## Compatibility and migration

Version-0 Postcard peers are intentionally incompatible and receive no fallback. BitTorrent identifiers and networking remain behind the M6 namespaces/policies and are not part of this profile. Future M3 discovery may replace only the discovery profile.

## Alternatives

Keeping Postcard, using canonical CBOR, and implementing M3 for the testnet were considered. ADR 0009 records why the explicit binary profile was selected and why M3 remains research.

## Test plan and change discipline

Rust and Python consume the same checked-in positive/negative vectors. A process test transfers deterministic content from a Rust provider to the Python receiver through the real bootstrap, relay, and Noise session. Any incompatible change requires a new protocol version, updated spec/vectors, both implementations, and migration notes.
