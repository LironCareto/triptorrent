# RFC 0004: Namespaced BitTorrent Compatibility

- Status: Research recommendation
- Milestone: M6

## Summary

TripTorrent will treat BitTorrent v1 and v2 infohashes as typed aliases of verified content, retain an independent TripTorrent content identity, and isolate classic BitTorrent networking behind an explicit compatibility adapter. TripTorrent-native operation must never fall back automatically to classic discovery or direct peers.

This RFC selects an architecture for later implementation. It defines no stable wire encoding, URI syntax, or production BitTorrent stack.

## Motivation

Existing `.torrent` files, magnets, payloads, swarms, trackers, clients, and publishing workflows are valuable adoption paths. Their identity and privacy semantics differ from TripTorrent's. A v1 or v2 infohash commits to a bencoded `info` dictionary; the current TripTorrent ID commits to raw single-file bytes. Classic discovery also reveals content keys and endpoints that M3 intends to separate.

Compatibility therefore needs an explicit boundary. Treating all hashes as interchangeable or silently invoking a classic swarm would create ambiguous integrity claims and a privacy downgrade.

## Identifier model

Implementations must identify the namespace and algorithm of every identifier. The initial conceptual types are:

- `tt:blob`: current experimental BLAKE3 digest of one byte string;
- `bt:btih`: BEP 3 SHA-1 digest of exact validated `info` bytes;
- `bt:btmh`: BEP 52 SHA-256 multihash identity of exact validated `info` bytes.

These labels are conceptual and do not freeze serialized syntax. A hybrid torrent has both BitTorrent aliases. Aliases may be attached to TripTorrent content only after the payload satisfies both the BitTorrent metadata and TripTorrent manifest. Metadata-only aliases remain unverified and cannot advertise content availability.

Multifile torrents are path trees, not concatenated blobs. A later RFC must define a canonical TripTorrent tree identity before an implementation claims a lossless multifile identity mapping.

## Import model

Strict metainfo parsing must preserve the exact `info` bytes used for hashing and reject noncanonical or ambiguous bencoding. Parsers must bound input size, nesting, dictionaries, path count, piece data, and piece layers.

Import consists of independent steps:

1. Metadata import records exact metainfo, typed hashes, paths, flags, and inert network hints as unverified state.
2. Local association validates safe paths and every promised BitTorrent hash while calculating the TripTorrent ID and manifest from bytes.
3. Network acquisition runs only after the caller selects a network mode.

Trackers, web seeds, explicit peers, and unknown fields do not activate network access. The BEP 27 private flag is preserved and enforced by the classic adapter. Imported paths must remain within an isolated root and survive cross-platform normalization/collision checks before file creation.

Exporting preserved metainfo byte-for-byte preserves its BitTorrent identity. Generating metainfo is a new identity-producing operation with explicit version, paths, piece length, and network hints. Only verified local bytes may be exported as available content.

## Link model

Ordinary `btih` and `btmh` magnets are compatibility hints. They do not contain an M3 discovery capability and must not be presented as TripTorrent-private links.

A future TripTorrent link must represent these fields separately:

- TripTorrent content identity;
- native discovery capability;
- optional verified BitTorrent aliases;
- optional classic compatibility hints;
- requested or permitted network mode.

Discovery secrets must not be copied into fields that classic clients publish to trackers, DHT nodes, PEX peers, logs, or web services. Whether the final form uses a new magnet exact-topic namespace, a new URI scheme, or both remains open.

## Network modes

Every transfer has one explicit persistent policy:

### TripTorrent-only

Use only TripTorrent-native discovery and relayed transfer. Failure to find a native route is final. Do not contact classic trackers, Mainline DHT, PEX, local discovery, explicit peers, or web seeds.

### Dual

Run native and classic adapters as separately observable paths against the same verified content record. The user explicitly permits classic leakage. Results, errors, connections, and byte accounting remain attributable to their path.

### Classic

Use ordinary BitTorrent mechanisms according to their specifications and describe the operation as classic BitTorrent. No TripTorrent privacy property is claimed.

Imported metadata does not select `dual` or `classic`. Remote metadata cannot weaken local policy. Implementations must expose the configured mode and active path through APIs and user interfaces.

## Adapter boundary

The persistent node owns verified bytes, typed aliases, import provenance, and transfer policy. A native adapter uses the M3 discovery capability and TripTorrent relay/session protocol. A classic adapter may later implement trackers, Mainline DHT, peer exchange, local discovery, web seeds, direct peers, and uTP. The adapters do not share peer endpoints, discovery records, session identities, or fallback state.

Existing clients can reuse storage, torrent parsing, scheduling, and BitTorrent verification while adding native discovery, relay transport, TripTorrent verification, and mode UI. No behavior depends on the Rust reference implementation or its database.

## Bridges and extension negotiation

A bridge may acquire bytes in one network, verify every relevant identity, and provide them in another. It must be opt-in, labeled as a correlation point, obey private-torrent authorization, use independent network identities, and make no privacy claim against its operator. Default or invisible bridging is forbidden.

BEP 10 negotiation is worth prototyping for upgraded clients already using the classic path. It happens after classic peer discovery and handshake, so it cannot recover endpoint or infohash privacy already lost and cannot bootstrap TripTorrent-only mode.

## Security and privacy

Classic trackers associate requester IPs with infohashes. DHT nodes see lookup keys and source addresses. Tracker/DHT/PEX responses distribute peer endpoints. Local discovery broadcasts a hash and port. Web seeds see the requester and requested object/ranges. Direct peers see one another. Dual operation and bridges permit correlation by payload, alias, size, and timing.

V1 SHA-1 remains a compatibility identifier rather than the sole TripTorrent integrity root. When both v1 and v2 are promised, implementations verify both and must not accept stripping v2 as a downgrade. All acquired bytes are also verified against the requested TripTorrent identity before native publication.

## Alternatives

- Reusing v2 hashes as TripTorrent IDs conflates payload identity with metadata and has no answer for v1-only content.
- Deriving a TripTorrent ID from BitTorrent metadata cannot prove possession or byte equality.
- A default bridge concentrates visibility and weakens private-torrent policy.
- Direct classic interoperability in native mode exposes endpoints and silently changes the selected threat model.

## Migration

Implementation should proceed in bounded stages: offline artifact import and local verification; authenticated exchange of verified aliases over TripTorrent; an explicit classic adapter; opt-in bridge experiments; and capability negotiation for upgraded clients. Every stage remains useful if a later stage is delayed.

## Testability

Conformance tests must include canonical and malformed bencoding, exact infohash calculation, v1/v2/hybrid classification, magnets with repeated topics, safe and unsafe paths, alias verification, mode downgrade attempts, and same-payload/different-torrent cases. Network tests must assert that TripTorrent-only mode makes no classic DNS, tracker, DHT, multicast, web-seed, or peer connection.

The M6 vectors and `triptorrent-interop` research crate cover the offline subset. They are deterministic and make no network request.

## Unresolved protocol details

Later RFCs must define TripTorrent tree identity, exact alias binding and signatures, URI syntax, metadata bounds, mode negotiation, bridge authorization, and v2/hybrid edge-case policy. This RFC intentionally does not make M3 wire details normative.
