# M5 Persistent Node

## Commands

Create a configuration once, then keep the foreground daemon running:

```bash
triptorrent node init --config triptorrent.toml --data-dir ./data --bootstrap 127.0.0.1:7100
triptorrent node start --config triptorrent.toml
```

Persistent CLI operations are local API clients:

```bash
triptorrent node status --config triptorrent.toml
triptorrent content add --config triptorrent.toml ./file.bin
triptorrent content list --config triptorrent.toml
triptorrent node fetch --config triptorrent.toml --content <CONTENT_ID>
triptorrent transfer list --config triptorrent.toml
triptorrent transfer show --config triptorrent.toml <ID>
triptorrent transfer pause --config triptorrent.toml <ID>
triptorrent transfer resume --config triptorrent.toml <ID>
triptorrent content remove --config triptorrent.toml --content <CONTENT_ID>
triptorrent content remove --config triptorrent.toml --content <CONTENT_ID> --delete-bytes
triptorrent node diagnostics --config triptorrent.toml
triptorrent node stop --config triptorrent.toml
```

Import copies bytes into managed storage. Removing metadata stops sharing but preserves managed bytes; `--delete-bytes` deletes only the managed copy, never the source file.

## Configuration

Configuration precedence is CLI override, `TRIPTORRENT_*` environment variable, TOML file, then built-in default. Supported environment variables are `DATA_DIR`, `API_LISTEN`, `BOOTSTRAP`, `DOWNLOAD_LIMIT`, `UPLOAD_LIMIT`, `MAX_CONCURRENT_TRANSFERS` and `LOG` with the `TRIPTORRENT_` prefix. Zero bandwidth means unlimited. Defaults use `127.0.0.1:7331`, four concurrent downloads and `info` logging.

`node init` generates the API bearer token. On Unix it creates the config with mode `0600` where supported. Keep this file private.

## Local API

The HTTP+JSON API is an implementation control interface, separate from TripTorrent's wire protocol:

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/v1/health` | Liveness, store and API readiness |
| GET | `/v1/status` | Uptime and transfer/content counters |
| GET | `/v1/node` | Local node configuration summary |
| GET/POST | `/v1/content` | List or import content |
| DELETE | `/v1/content/{id}?delete_bytes=true` | Remove metadata and optional managed bytes |
| POST | `/v1/fetches` | Start a persistent fetch |
| GET | `/v1/transfers` | List transfers |
| GET | `/v1/transfers/{id}` | Inspect one transfer |
| POST | `/v1/transfers/{id}/pause` | Cooperatively pause a running download |
| POST | `/v1/transfers/{id}/resume` | Resume a paused download from verified state |
| GET | `/v1/diagnostics` | Bootstrap and provider-worker state |
| GET | `/v1/network-privacy` | Structured current path and disclosure status |
| POST | `/v1/shutdown` | Clean shutdown |

Mutating requests require `Authorization: Bearer <api_token>`. Errors use `{"error":{"code":"...","message":"..."}}`. The daemon rejects non-loopback bind addresses. Loopback plus a token is not protection from hostile software running under the local account; do not expose this API through port forwarding or a public proxy.

## Storage and Recovery

The data directory contains:

```text
data/
├── content/<content-id>/
│   ├── data
│   ├── manifest.postcard
│   ├── data.triptorrent-part       # while downloading
│   └── data.triptorrent-state      # verified M4 completion bits
└── state/node.sqlite3              # schema version 2, WAL enabled
```

SQLite stores content and transfer metadata, not file blobs. Transfer rows include verified progress, current verified-byte rate, provider/retry/rejection counters and provider contribution. Startup verifies completed bytes and manifests. Missing or corrupt content is marked accordingly and unshared. Stale running transfers become interrupted and restart automatically when bootstrap is configured; explicitly paused transfers remain paused. M4 rechecks every recorded chunk before either recovery or explicit resume. Valid shared content gets a fresh ephemeral advertisement. Connections, relay routes and protocol peer identities are never restored.

Logs are JSON and cover lifecycle, content and transfer outcomes. They omit tokens and key material but can include content IDs, paths, peer/route metadata and timing, so retain them as privacy-sensitive data. OS service packaging, remote administration, transfer cancellation and production migrations are deferred.
