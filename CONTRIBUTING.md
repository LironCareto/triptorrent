# Contributing

Thanks for your interest in TripTorrent.

## Before coding

For protocol-visible changes, open or discuss an RFC first. Implementation work should not silently create new protocol behaviour.

## Pull requests

- keep changes focused;
- add tests for behaviour changes;
- document protocol-facing changes;
- avoid unrelated refactors;
- do not introduce custom cryptography without exceptional justification and review;
- keep implementation details consistent with the specification.

## Rust

The reference implementation uses stable Rust unless a documented decision says otherwise.

Run before submitting:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python interop/python/conformance.py
```

## Testing

Install `interop/python/requirements.txt` first. `cargo test --workspace` is the standard full validation. It builds the CLI first, then runs unit, integration and process-level end-to-end tests. The harness launches compiled `triptorrent` binaries plus the independent Python receiver, exercises bootstrap, relay, Noise, sharing, fetching, failover and concurrent routes, and cleans up every child process.

Protocol changes must update `spec/testnet-v1.md` (or introduce a new version), fixed vectors, Rust conformance, Python conformance, and migration notes together. Never regenerate expected vectors during a conformance test.

The multi-terminal commands in the README remain useful for exploration and debugging, but they are optional and are not milestone acceptance requirements.
