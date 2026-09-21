# M9 Testnet Operator Guide

## Services and ports

Bootstrap TCP `7100` carries bounded Testnet v1 control records. Relay TCP `7000` carries versioned registrations and opaque framed Noise traffic. Open only the chosen public service ports. Never expose the persistent node HTTP API; it is loopback-only and is not testnet infrastructure.

Build once with `cargo build --locked --release -p triptorrent-cli`, copy `triptorrent` to the host, and run as an unprivileged account. Alternatively use `deploy/Dockerfile` and `deploy/docker-compose.yml`, or install the units from `deploy/systemd/`. Compose uses Linux host networking and refuses to start until `TRIPTORRENT_PUBLIC_RELAY_ADDRESS` is a real numeric public `IP:port`.

```bash
TRIPTORRENT_PUBLIC_RELAY_ADDRESS=<REAL_IP:7000> docker compose -f deploy/docker-compose.yml up -d --build
docker compose -f deploy/docker-compose.yml ps
```

For separate hosts, start bootstrap first, then configure each relay with its real bootstrap and externally reachable advertised address. Run at least two relays for public validation.

## Health, limits, and logs

```bash
triptorrent testnet health --bootstrap <IP:7100>
triptorrent testnet health --bootstrap <IP:7100> --relay <IP:7000>
```

Defaults retain M8 bounds: 256 bootstrap handlers, 4,096 peers, 256 relays, 256 content IDs per peer, 64 queued assignments per provider, 1,024 pending relay routes, 256 registration handlers/active relay pairs, 30-second pending routes, 256 KiB control frames, and 1 MiB opaque frames. Service stderr emits `TRIPTORRENT_BOOTSTRAP_METRIC` and `TRIPTORRENT_RELAY_METRIC` events for active state, registrations, discovery, rejection, expiry, pairing, and closure. There is no centralized telemetry.

Logs are operator-local. IP addresses remain visible to the OS/network stack; route IDs, timing, counts, and content-related activity are privacy-sensitive. Do not add plaintext, content IDs, private keys, API tokens, or Noise material to logs. Compose rotates three 10 MiB files.

## Restart, upgrade, and abuse

Bootstrap registration state is memory-only; peers and relays re-register after restart. An unmatched relay route expires. Stop with `docker compose ... down` or `systemctl stop ...`; active transfers fail and can resume verified chunks through the client. Upgrade by verifying release SHA-256 files, replacing the binary/image, and restarting one relay at a time before bootstrap. Never mix wire versions under one network identifier.

Unauthenticated registration, failure-report spoofing, capacity consumption, scanning, and traffic correlation remain expected testnet abuse risks. Use host firewall/rate controls and monitor bounded rejection counters. Do not advertise the service for privacy-sensitive traffic.

## Public deployment checklist

From two independent networks, run the probe against canonical content, test wrong version/network rejection, confirm traffic used the relay, restart one relay during a resumable transfer, compare SHA-256, and inspect logs for secrets. Commit endpoints only after these checks. Private vulnerability reporting must also be enabled and tested before public exposure.
