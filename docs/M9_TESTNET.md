# M9 Public Testnet

## Status

**M9 IMPLEMENTATION READY — PUBLIC DEPLOYMENT VALIDATION PENDING.** No public seed is committed because no real endpoint has been deployed and externally validated from this environment. This is a controlled experimental testnet, not production, an anonymity service, or suitable for privacy-sensitive transfers.

## Install and inspect

Build Rust and install the independent-test dependencies:

```bash
cargo build --workspace
python -m pip install -r interop/python/requirements.txt
triptorrent testnet status
```

The output distinguishes the software version, wire protocol `1`, network `triptorrent-testnet-1`, discovery profile `m2-testnet-bootstrap-v1`, and transfer profile `relayed-swarm-v1`.

There is no default public bootstrap until deployment succeeds. For a community/development testnet, pass its real address explicitly with `--bootstrap`. The persistent local API remains loopback-only.

## Canonical test transfer

The project-owned artifact is `test-data/m9-canonical.txt`:

- size: 185 bytes;
- content ID (BLAKE3): `fdd7d693c7ca9bb83ff74f5b900f03eb7f3fbda3ab46043f7938c4473d72e199`;
- SHA-256: `8fee8c591bfdc993cf9a2f53e567a5cfe39738e6d70e60caef9b0955178d81c5`;
- one 32 KiB manifest chunk.

After an operator advertises that exact file, validate the whole path:

```bash
triptorrent testnet probe --bootstrap <REAL_IP:7100> --output canonical.txt
```

The machine-readable JSON reports the negotiated network/version and verified byte count. Ordinary sharing still uses `triptorrent share --bootstrap ... --file ...`. Report non-sensitive failures in GitHub issues with command output, platform, software version, and timestamps. Follow `SECURITY.md` for vulnerabilities.

## Security and privacy

The centralized bootstrap sees peer IPs, content advertisements/queries, public keys, routes, and timing. Relays see endpoint pairing, route IDs, timing, and traffic volume. M8 demonstrated traffic-shape correlation, and targeted Sybil/eclipse resistance remains unsolved. A global passive observer is outside the protection claim. M3 private discovery is not implemented. Testnet v1 prevents direct provider connections and encrypts application data end to end through the relay; these facts do not guarantee anonymity.

GitHub Private Vulnerability Reporting is enabled and API-verified; maintainers must still check the reporter-facing submission flow. Final M9 completion additionally requires real public bootstrap/relay deployment, two-location external probing, incompatible-network rejection, relay restart testing, checksum verification, and secret-free log review.
