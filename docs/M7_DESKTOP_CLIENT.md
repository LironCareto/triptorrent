# M7 Reference Desktop Client

M7 adds a native `egui`/`eframe` reference client. The window is a front-end to the persistent node: all library, transfer, lifecycle and diagnostic operations use the authenticated loopback `/v1` API. The desktop never opens SQLite, edits managed storage or calls the swarm engine.

## Build and run

Build both executables together so the desktop can find the `triptorrent` sidecar beside itself:

```bash
cargo build --workspace
target/debug/triptorrent-desktop
```

For development, explicit paths are supported:

```bash
target/debug/triptorrent-desktop --config ./triptorrent.toml --node-binary ./target/debug/triptorrent --bootstrap 127.0.0.1:7100
```

On first run the client creates a token-protected node configuration. Data lives under `%APPDATA%\TripTorrent` on Windows, `~/Library/Application Support/TripTorrent` on macOS, and `$XDG_CONFIG_HOME/triptorrent` (or `~/.config/triptorrent`) on Linux. `TRIPTORRENT_HOME` overrides that root. No public bootstrap is assumed. Without an explicit bootstrap, local library management works and network fetch is disabled.

## Node lifecycle

Startup checks health before launching anything. It connects to a healthy node or starts the adjacent CLI executable with `node start`, waits for API readiness, and retries failed connections with bounded backoff. This prevents a second launch on the normal API endpoint. The node remains running when the window closes; **Stop node** performs an authenticated shutdown. Startup failures and later crashes appear as disconnected or unhealthy state while the UI remains responsive.

## Transfers and library

Transfers show persisted state, verified bytes/chunks, measured verified-byte rate, provider counts, retries, rejected work and verified provider contribution. Pause stops new scheduling, releases current sessions and persists `paused`; bounded in-flight work may finish first. Resume rediscovers providers, validates the M4 partial state and reuses verified chunks. A paused transfer stays paused across node restart. Pause/resume currently applies only to downloads.

Adding a file copies and verifies it in managed storage. **Remove from TripTorrent** removes metadata and stops sharing while retaining managed bytes. **Delete managed copy** requires confirmation and never deletes the original source file. Identity rows remain typed; the current runtime supplies only a verified TripTorrent identity.

## Network and privacy status

The status view reports the current path: centralized temporary M2 bootstrap discovery and end-to-end encrypted, relayed M4 swarm transfer. Peers do not connect directly, but the bootstrap and relays observe the metadata documented in the threat model. There is no anonymity guarantee. M3 private discovery and M6 classic/dual BitTorrent networking remain unimplemented and cannot be selected.

Diagnostics omit bearer tokens and session secrets. Content IDs, configured addresses, local paths and timing may still be privacy-sensitive.

## Validation and limitations

`cargo test --workspace` exercises controller rendering, real daemon startup/reconnect/authentication, library removal, verified progress, pause/restart/resume and exact transferred bytes. The client has no installer, OS service, remote administration, auto-update, final URI handler, classic BitTorrent adapter or production M3 discovery path.
