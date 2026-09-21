# M9 Implementer Guide

The authoritative wire document is [`spec/testnet-v1.md`](../spec/testnet-v1.md). ADR 0009 records the encoding decision and RFC 0005 defines the profile boundary. Rust source is a reference, not the specification.

## Conformance

`test-vectors/m9-testnet-v1.json` contains semantic descriptions and fixed bytes for negotiation, content/manifest, capabilities, bootstrap registration/discovery, route assignment, relay registration, swarm messages, boundaries, malformed input, unknown messages, and invalid ordering. Implementations should load this JSON directly and compare their own encoder/decoder results; do not invoke either reference encoder to generate expected bytes during tests.

```bash
cargo test -p triptorrent-protocol --test testnet_v1_conformance
python interop/python/conformance.py
```

The independent Python receiver uses pinned `noiseprotocol` and `blake3` packages. It neither imports Rust crates nor calls the Rust executable for encoding. `cargo test --workspace` launches it as a separate process against compiled Rust bootstrap, relay, and provider processes and verifies the final bytes.

## Compatibility discipline

Testnet v1 has no version-0/Postcard fallback. Unsupported version, wrong network, unknown type, unsupported capability, malformed lengths, trailing data, or invalid state order fail closed. An incompatible change requires a new protocol version, updated normative specification and migration notes, regenerated reviewed vectors, updated Rust and independent implementations, and cross-language evidence. A software release alone does not change the wire version.

Discovery is explicitly replaceable. `m2-testnet-bootstrap-v1` supplies provider key, relay address, and opaque route; core content identity, Noise session, relay transport, and swarm exchange do not depend on its registry implementation. Do not describe this scaffold as the future M3 private discovery design.
