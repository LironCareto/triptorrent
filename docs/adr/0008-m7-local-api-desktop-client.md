# ADR 0008: Keep the desktop client behind the local node API

- Status: Accepted
- Date: 2026-09-20

## Context

M5 established a persistent daemon that owns storage, transfer recovery and network workers. M7 needs a native client that can launch and monitor that daemon without turning presentation code into a second runtime. The client must remain responsive during hashing, daemon startup and network failure, and its status must not overstate the privacy of the current M2/M4 path.

## Decision

Use `egui`/`eframe` for the native Rust desktop executable. Extract local API DTOs and the authenticated HTTP client into `triptorrent-node-api`; both CLI and desktop depend on this crate, while the node implements the server. The GUI performs no direct storage or swarm operation.

Run a background desktop controller that polls the loopback API and sends immutable snapshots to the view. It first probes health, then starts the adjacent `triptorrent` executable only when needed, waits for readiness, and reconnects with bounded backoff. The daemon persists after the window closes and has an explicit authenticated stop action. First run creates a normal M5 configuration in a per-user directory without inventing a bootstrap service.

Extend `/v1` with pause/resume actions, verified progress and provider diagnostics, plus a structured network/privacy status. Pause cooperatively stops scheduling, closes sessions and preserves verified M4 state. Explicitly paused work is distinct from interrupted work and is not resumed at daemon startup.

## Alternatives considered

- Embed node/runtime code in the GUI. Rejected because it duplicates lifecycle ownership and prevents other clients from reusing the daemon.
- Build a browser UI or add a JavaScript toolchain. Rejected because the required interface fits the existing Rust workspace and a native window avoids another build/runtime stack.
- Block the render thread on HTTP calls. Rejected because daemon and network failure would freeze the application.
- Stop the daemon when the window closes. Rejected because the node is deliberately persistent and may continue sharing or downloading.

## Consequences

The API becomes a shared implementation boundary and gains typed DTO compatibility responsibilities. HTTP polling adds bounded update latency but keeps the state machine inspectable. The desktop sidecar must be packaged beside the GUI executable in distributions. A loopback bearer token limits accidental mutation but does not protect against hostile software running as the same user.

The interface truthfully exposes only the implemented TripTorrent identity and M2/M4 network path. M3 discovery, M6 classic/dual networking, installers and remote administration remain separate future work. This ADR changes no TripTorrent wire protocol.
