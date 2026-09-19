# Design Principles

- Prefer simple, inspectable protocol state machines.
- Separate content identity from peer identity.
- Separate peer identity from network location.
- Minimise metadata exposure.
- Reuse reviewed cryptographic primitives and libraries.
- Make downgrade behaviour explicit.
- Avoid silent fallback from private modes to direct networking.
- Define failure modes before optimising happy paths.
- Design for interoperability, fuzzing and simulation.
- Keep the protocol implementable outside Rust.
- Do not let the reference client become the specification by accident.
