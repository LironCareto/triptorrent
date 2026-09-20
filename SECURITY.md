# Security Policy

TripTorrent is experimental pre-alpha software and must not yet be relied upon for anonymity or security-sensitive use.

## Reporting vulnerabilities

Please avoid publishing exploitable vulnerabilities before maintainers have had a reasonable opportunity to investigate and coordinate a fix.

Until a dedicated private reporting channel is configured, open a GitHub issue **only for non-sensitive security design discussions**. Do not post exploit details for an unpatched vulnerability publicly.

Before M9, maintainers must enable GitHub **Private vulnerability reporting** under repository Settings → Security → Code security and analysis, and verify that a private report can be opened. This setting cannot be enabled by a committed repository file. Until it is enabled, do not publish exploit details; contact a maintainer through a previously established private channel. The public issue tracker remains appropriate only for non-sensitive design discussion.

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

The current M8 gate and known residual risks are recorded in [docs/M8_FINDINGS.md](docs/M8_FINDINGS.md). A “READY FOR M9” result permits only a controlled experimental testnet and is not a security or anonymity certification.
