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
```
