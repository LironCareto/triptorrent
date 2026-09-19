# ADR 0007: Keep BitTorrent identifiers namespaced and compatibility explicit

- Status: Accepted
- Date: 2026-09-20

## Context

TripTorrent aims to reuse the BitTorrent ecosystem without silently inheriting its metadata and endpoint exposure. The current TripTorrent prototype identifies raw single-file bytes with BLAKE3. BitTorrent v1 and v2 identify exact bencoded `info` dictionaries with SHA-1 and SHA-256 respectively. Identical payload bytes may therefore have different torrent identities, and multifile torrents also commit to a path tree and layout.

Classic trackers, Mainline DHT, PEX, local discovery, web seeds, and direct peers expose information that the M3 discovery direction intends to separate. Importing an artifact cannot safely imply permission to use those mechanisms.

## Decision

Keep the TripTorrent content identifier independent. Record validated `btih` and `btmh` values as typed aliases, with provenance and verification state. Metadata alone creates unverified aliases; only payload validation binds them to a TripTorrent identity. Do not claim lossless multifile mapping until a canonical TripTorrent tree identity is specified.

Adopt an import-first architecture and place future classic networking in a separate adapter. Define three explicit policies: TripTorrent-only, dual-network, and classic. TripTorrent-only fails closed. No mode may silently activate a less private path. M3 discovery capabilities remain separate from content identifiers and are never published through classic hints.

Allow future bridges only as explicit correlation points. Explore BEP 10 negotiation only for clients already operating on a classic connection; it is not a privacy bootstrap.

## Alternatives considered

- Use the v2 infohash as the TripTorrent ID. Rejected because it commits to metadata, excludes v1-only torrents, and makes naming/layout choices part of TripTorrent identity.
- Derive a TripTorrent ID from a BitTorrent hash. Rejected because it does not verify payload bytes and preserves arbitrary torrent-construction choices.
- Automatically fall back to classic peers. Rejected because it changes endpoint and content-query exposure without consent.
- Route all interoperability through bridges. Rejected as the default because bridges observe both networks and complicate authorization and availability.

## Consequences

Implementations maintain more than one identifier and may hash bytes with BLAKE3, SHA-1, and SHA-256. This costs CPU and manifest storage but can occur in one streaming read. Imported metadata remains useful before payload acquisition without being confused with available content.

Clients must expose selected and active network modes. Classic operation remains possible with unmodified BitTorrent peers, but it carries classic privacy properties. TripTorrent-native behavior remains independently implementable and does not depend on the Rust daemon or any single client.

The exact wire formats, URI syntax, multifile identity, classic adapter, and bridge protocol remain later work. See [RFC 0004](../../rfcs/0004-namespaced-bittorrent-compatibility.md) and the [M6 research report](../M6_BITTORRENT_INTEROP_RESEARCH.md).
