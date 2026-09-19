# M1 Manual Validation

Date: **2026-09-19**

Status: **passed**

TripTorrent M1 was manually exercised on Windows using the reference CLI with three local processes:

```text
Peer A  ->  Relay  ->  Peer B
```

Both peers connected only to the relay. The receiver requested content by TripTorrent content ID, received the encrypted relayed transfer, reconstructed the file, and produced a byte-for-byte identical result.

## Environment

- Windows
- Cargo 1.98.1
- Rust workspace build: successful
- Relay address: `127.0.0.1:7000`
- Route ID: `demo`

## Test file

The test source was created with:

```powershell
"Contenido de prueba de TripTorrent" | Set-Content .\source.txt
```

TripTorrent content ID:

```text
005794bf7bd610c5d1f171f6a35bd94d9eaa2ad8bd0f986f02ddfdce098392d4
```

## Commands

Relay:

```powershell
cargo run -p triptorrent-cli -- relay --listen 127.0.0.1:7000
```

Identify content:

```powershell
cargo run -p triptorrent-cli -- id .\source.txt
```

Share from Peer A:

```powershell
cargo run -p triptorrent-cli -- share `
  --relay 127.0.0.1:7000 `
  --route demo `
  --key 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f `
  --file .\source.txt
```

Fetch from Peer B:

```powershell
cargo run -p triptorrent-cli -- fetch `
  --relay 127.0.0.1:7000 `
  --route demo `
  --key 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f `
  --content 005794bf7bd610c5d1f171f6a35bd94d9eaa2ad8bd0f986f02ddfdce098392d4 `
  --output .\received.txt
```

Peer A reported:

```text
sharing 005794bf7bd610c5d1f171f6a35bd94d9eaa2ad8bd0f986f02ddfdce098392d4; waiting for receiver on route demo
transfer complete
```

## Integrity result

PowerShell SHA-256 verification:

```text
source.txt   5BDD03D247AE38CF7FCC6E9205A3BAA3D52AE8AEF2A8138429D3C86C7900E87B
received.txt 5BDD03D247AE38CF7FCC6E9205A3BAA3D52AE8AEF2A8138429D3C86C7900E87B
```

The hashes matched exactly.

## What this validates

This manual run validates the M1 implementation path:

- the workspace builds on Windows;
- a sender can expose content through the relay;
- a receiver can request it by content ID;
- the transfer completes through the relay path;
- the reconstructed output is byte-for-byte identical to the source.

## What this does not validate

M1 is still a local prototype. This test does **not** establish:

- anonymity;
- resistance to traffic correlation;
- multi-hop routing;
- peer discovery;
- NAT traversal;
- hostile-relay resistance beyond the current encrypted-session design;
- BitTorrent interoperability;
- Internet-scale behaviour.

Those remain later milestones and research questions.
