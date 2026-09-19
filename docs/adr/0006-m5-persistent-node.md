# ADR 0006: Separate Persistent Node Runtime

- Status: Accepted
- Date: 2026-09-20

## Context

M1–M4 commands owned one short-lived transfer. M5 needs one process to manage several contents and transfers, preserve useful state, expose local control to different front-ends and recover after restart. Putting this state in CLI parsing would couple later GUI work to a command-line implementation.

## Decision

Add a dedicated `triptorrent-node` crate. It owns the daemon runtime, state machine, TOML configuration, SQLite index, managed filesystem store, restart recovery, structured logs and versioned loopback HTTP+JSON API.

The crate defines a small `TransferEngine` interface. `triptorrent-cli` implements that interface with the existing M4 share/fetch functions and acts as an API client for persistent commands. The node crate does not depend on CLI parsing or on concrete M4 orchestration.

Use SQLite in WAL mode for transactional metadata and schema versioning. Store large bytes and Postcard manifests under `content/<content-id>/`; retain M4 partial and state files beside the final data. Generate a bearer token during `node init`, require it for mutating API requests and reject non-loopback listeners.

## Consequences

Daemon state can be reused by a future GUI, while network mechanics remain replaceable. Restart restores valid sharing and interrupted downloads but rebuilds ephemeral advertisements, peer keys, relay routes and connections. The local API and storage schema are implementation interfaces rather than normative protocol surfaces.

SQLite and filesystem updates cannot form one atomic transaction, so startup integrity reconciliation is required. OS service installation, remote API exposure, richer authorization, cancellation and schema upgrades beyond version 1 remain future work.
