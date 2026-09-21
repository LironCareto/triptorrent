# Security Policy

TripTorrent is experimental pre-alpha software and must not yet be relied upon for anonymity or security-sensitive use.

## Reporting vulnerabilities

Please avoid publishing exploitable vulnerabilities before maintainers have had a reasonable opportunity to investigate and coordinate a fix.

Until a dedicated private reporting channel is configured, open a GitHub issue **only for non-sensitive security design discussions**. Do not post exploit details for an unpatched vulnerability publicly.

GitHub **Private vulnerability reporting** is enabled for this repository; its API status was verified on 2026-09-21. Use the repository Security tab to submit exploitable vulnerability details privately. Before public M9 deployment, maintainers must also verify the reporter-facing submission flow without publishing or creating a fictitious vulnerability. The public issue tracker is only for non-sensitive design discussion.

Reports should include the affected revision, platform, minimal reproduction, impact, and whether the issue is remotely reachable. Maintainers should acknowledge receipt, preserve disclosure confidentiality, reproduce the issue, coordinate a regression test and publish remediation notes when disclosure is safe.

## Scope

Security issues include, among others:

- identity or address leakage;
- route deanonymisation;
- downgrade attacks;
- cryptographic misuse;
- replay;
- metadata leakage;
- eclipse/Sybil weaknesses;
- malicious peer exploitation;
- remote code execution or memory-safety defects;
- unsafe fallback to classic BitTorrent/direct networking.

The current M8 gate and known residual risks are recorded in [docs/M8_FINDINGS.md](docs/M8_FINDINGS.md). M9 implementation readiness permits only a controlled experimental testnet and is not a security or anonymity certification.
