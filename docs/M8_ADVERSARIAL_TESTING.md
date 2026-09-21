# M8 Adversarial Testing

M8 attacks the implemented M1-M7 boundaries and the assumptions behind the M3 research model. It does not add a production DHT, change the selected architecture, deploy a testnet, or establish an anonymity claim.

## Reproducible validation

The core suite is deterministic and runs on stable Rust, including Windows:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p triptorrent-overlay --example m8_monte_carlo
cargo run -p triptorrent-overlay --example m8_traffic_analysis
```

Protocol and interoperability parsers also have bounded property tests. Optional longer libFuzzer campaigns use the isolated `fuzz/` package and require `cargo-fuzz` plus a supported LLVM toolchain:

```bash
cargo fuzz run protocol -- -max_total_time=300
cargo fuzz run interop -- -max_total_time=300
```

The Linux CI smoke job uses nightly Rust, fixed seeds and 1,000 executions per target. The normal suite neither requires nightly Rust nor Internet access.

On Windows, run campaigns from a Visual Studio Developer PowerShell so the LLVM AddressSanitizer runtime is on `PATH`. If using an ordinary PowerShell, locate the runtime installed by Visual Studio Build Tools and add its directory for the current session:

```powershell
$asan = Get-ChildItem "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2022\*\VC\Tools\MSVC\*\bin\Hostx64\x64\clang_rt.asan_dynamic-x86_64.dll" |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $asan) { throw 'Install the MSVC AddressSanitizer component with Visual Studio Build Tools.' }
$env:PATH = "$($asan.Directory.FullName);$env:PATH"
cargo +nightly fuzz run protocol -- -runs=1000
cargo +nightly fuzz run interop -- -runs=1000
```

Without that runtime path, Windows displays a missing `clang_rt.asan_dynamic-x86_64.dll` dialog and the parent fuzz command appears to hang. The M8 host completed 100-run `protocol` and `interop` campaigns after the path was configured. Local Windows campaigns remain optional. Fuzzer discoveries must be minimized and copied into an ordinary regression test before a fix is accepted.

## Attack surfaces and controls

- At the M8 revision, application and overlay Postcard decoding rejected inputs over 1 MiB and 256 KiB respectively, validated semantic collection bounds, manifest shape, availability padding, identities, routes and error lengths, and exercised experimental conformance vectors. M9 supersedes this external codec with Testnet v1.
- Relay registration accepts restricted 128-byte route names. Concurrent registration readers (256), pending routes (1,024), active pairs (256), stale registrations (30 seconds) and opaque frames (1 MiB) are bounded. Slow or malformed connections are isolated; duplicate roles and completed-route reuse fail closed.
- The M2 registry bounds peers (4,096), relays (256), content IDs per peer (256), discovery results (16) and queued assignments per provider (64). The control service caps concurrent handlers at 256.
- Manifests retain the fixed 32 KiB chunk size and cap content at 32 GiB / 1,048,576 chunks. Resume state is capped at 40 MiB and cannot promote bytes unless each claimed chunk verifies.
- The local API caps connections, targets, request bodies and error strings, rejects ambiguous headers and pipelining, preserves loopback-only defaults and authenticates mutations. Database rows and content IDs are validated before paths are derived.
- Bencoding is capped at 4 MiB, depth 64 and 100,000 nodes. Magnet size and parameter counts and sanitized path component length are bounded. Strict v1 piece counts, minimal v2 file trees and hybrid length consistency are checked. These functions perform no network I/O.

Named regression tests cover relay opacity, no direct native peer connection, corrupt-chunk rejection, verified resume, bounded retries, API authentication, loopback defaults, token redaction, persistent pause, malformed-protocol failure and the absence of an automatic network-mode downgrade.

## Discovery experiments

`m8_monte_carlo` runs 12 fixed seeds for vanilla Kademlia, blinded Kademlia, distributed rendezvous and separated multi-stage discovery. It varies 100-260 nodes, 5-30% malicious nodes, 0-20 failed nodes, targeted Sybils, attacker-prefix groups, 4-12 replicas, 1-5 paths, 0-50% routing contamination and repeated-key popularity.

The checked-in run produced:

| Candidate | Success min/median/max | Eclipse median | Rounds / messages | Honest / malicious replicas | Requester-key / provider-key observers | Same / colluding adversary | Repeated key |
|---|---:|---:|---:|---:|---:|---:|---:|
| vanilla | 0.0 / 75.0 / 100.0% | 25.0% | 2.8 / 16.9 | 6.4 / 1.5 | 8.4 / 14.9 | 66.6 / 69.4% | 70.8% |
| blinded | 25.0 / 75.0 / 100.0% | 25.0% | 2.8 / 17.3 | 6.3 / 1.6 | 8.6 / 14.7 | 65.2 / 66.6% | 70.8% |
| rendezvous | 18.7 / 62.5 / 100.0% | 50.0% | 4.1 / 30.8 | 6.6 / 1.3 | 0.0 / 0.0 | 0.0 / 0.0% | 68.7% |
| multi-stage | 25.0 / 75.0 / 100.0% | 25.0% | 6.5 / 55.0 | 6.6 / 1.3 | 0.0 / 0.0 | 0.0 / 0.0% | 75.0% |

In the fixed single-prefix targeted-Sybil scenario, closest-node placement put 8/8 replicas on Sybils and vanilla discovery succeeded 0/100 times. Prefix-diverse placement put 4/8 replicas on Sybils; separated multi-stage discovery succeeded 99/100 times, at higher control-message cost. Diversity helps under this assumption but does not establish Sybil resistance. These are model outputs under the checked-in assumptions, not Internet probabilities.

## Traffic metadata experiment

`m8_traffic_analysis` generates only timestamps, direction, framed sizes and totals for six deterministic M4 transfer shapes:

| Scenario | Content bytes | Providers | Rate B/s | Retries / pauses | Frames | Framed bytes | Duration ms |
|---|---:|---:|---:|---:|---:|---:|---:|
| small-single | 65,536 | 1 | unlimited | 0 / 0 | 8 | 66,181 | 7 |
| small-paused | 65,536 | 2 | 32,768 | 1 / 1 | 14 | 66,646 | 8,010 |
| medium-single | 1,048,576 | 1 | unlimited | 0 / 0 | 68 | 1,054,504 | 67 |
| medium-swarm | 1,048,576 | 4 | 262,144 | 2 / 0 | 84 | 1,058,680 | 4,299 |
| large-single | 4,194,304 | 1 | unlimited | 0 / 0 | 260 | 4,217,140 | 259 |
| large-paused | 4,194,304 | 4 | 524,288 | 3 / 1 | 278 | 4,230,676 | 13,399 |

Content-size rank was recovered in 12/12 pair comparisons; a nearest-size classifier identified 6/6 scenarios; repeated shapes had an identical direction/size fingerprint. The modeled bootstrap-to-relay delta was 100 ms. Current transfers are therefore readily distinguishable in this controlled model. Padding, batching, cover traffic and timing defenses remain separate design research.

This experiment uses the real message encoder but a synthetic deterministic clock and transport model. It does not measure OS scheduling, Internet jitter or encrypted packets on a live network.

## Result and limits

The implementation defects found during M8 are fixed and retained as regressions. M2 registration spoofing, failure-report abuse and deterministic selection remain accepted scaffold limitations. Targeted Sybils, routing-table poisoning, collusion and traffic correlation remain architectural risks. See [M8 findings](M8_FINDINGS.md) and the [threat model](THREAT_MODEL.md).

**READY FOR M9**, limited to a clearly labelled controlled public testnet. This recommendation means the tested implementation has no known unresolved M9-blocking crash, trivial unbounded state, integrity/authentication bypass, arbitrary file deletion, secret logging or silent downgrade. It is not a security or anonymity certification.
