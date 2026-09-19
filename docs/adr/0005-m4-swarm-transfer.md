# ADR 0005: M4 relayed swarm transfer

Status: accepted for prototype

## Context

M4 must prove that one receiver can combine verified chunks from independent providers while M3 discovery remains research only. The implementation needs partial availability, bounded concurrency, failover, malicious-piece handling, resume and bandwidth controls without introducing direct peer sockets or persistent-node storage.

## Decision

Retain the M2 bootstrap and relay network as replaceable scaffolding. Extend discovery to return distinct providers and create one independent relay route and Noise session per provider. Add experimental manifest, compact availability-bitfield and indexed chunk-request messages. Keep the existing 32 KiB chunks and BLAKE3 integrity model.

Add a separate `triptorrent-swarm` crate containing a deterministic rarest-first scheduler and deterministic byte-rate reservation model. Allow one in-flight request per provider and at most eight provider sessions. Disable a provider for the current transfer after a failed, unavailable, unexpected or corrupt response, then reassign its chunk.

Persist verified chunks in `<output>.triptorrent-part` with a versioned, manifest-bound `<output>.triptorrent-state`. Revalidate recorded chunks on restart and rename only after whole-content verification. Keep all penalties and resume state local to one transfer.

## Alternatives considered

Sequentially downloading complete files from providers would not be a swarm. Publishing piece maps in the future M3 DHT would couple transfer state to an unimplemented discovery design. Global reputation, tit-for-tat and persistent storage belong to later work. Replacing M2 discovery during M4 would combine two architectural experiments and was rejected by scope.

## Consequences

The prototype demonstrates concurrent multi-source transfer, deterministic scheduling and recovery while preserving relayed encrypted sessions. More providers increase relay load and expose swarm membership, timing and volume to more parties. The centralized bootstrap remains trusted scaffolding, manifest authenticity is content-addressed rather than publisher-signed, and resume files are not a general storage subsystem. All messages and local formats remain experimental.
