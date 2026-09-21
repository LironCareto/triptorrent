# ADR 0009: Freeze Testnet v1 with an explicit binary wire profile

- Status: Accepted for Testnet v1
- Date: 2026-09-20

## Context

The M1-M8 wire used Serde/Postcard enum layouts. Although deterministic in one Rust build, implicit discriminants and library behavior were a fragile basis for independent implementations. M9 also requires version and network negotiation at bootstrap, relay, and peer boundaries.

## Decision

Use the small binary format specified in `spec/testnet-v1.md`: fixed big-endian integers, explicit lengths, numeric domains/types, canonical field order, strict trailing/unknown rejection, protocol version `1`, and network `triptorrent-testnet-1`. Keep Noise KN and BLAKE3, but cap Testnet v1 manifests at 2,036 chunks so every application record fits the Noise plaintext bound.

The M2-derived bootstrap is named the replaceable `m2-testnet-bootstrap-v1` discovery profile. No version-0 fallback exists. Checked-in vectors and a Python implementation constrain wire drift.

## Alternatives

Postcard was rejected as a normative cross-language contract. Canonical CBOR was viable but adds a general data model and canonicalization dependency for a small fixed message set. JSON was rejected for binary overhead and multiple canonical representations.

## Consequences

M1-M8 local version-0 peers cannot join Testnet v1. Incompatible changes require a new protocol version, specification, vectors, both implementations, and migration notes. The compact codec is intentionally strict and less extensible within one version.
