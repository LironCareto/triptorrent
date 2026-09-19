# M6 BitTorrent Interoperability Research

Status: completed research; no production BitTorrent networking is implemented.

## 1. Executive summary

TripTorrent should keep its own namespaced content identity and attach verified BitTorrent v1 and v2 identifiers as aliases. A BitTorrent infohash commits to an `info` dictionary, while the current TripTorrent identifier commits to one byte string. They are different claims and must never be substituted for one another.

Importing a `.torrent` or magnet creates unverified metadata and compatibility hints. Reading and verifying the payload is required before assigning a TripTorrent content ID. TripTorrent-only, dual-network compatibility, and classic BitTorrent operation are explicit modes. TripTorrent-only mode fails closed and never queries a tracker, the Mainline DHT, PEX, local discovery, web seeds, or direct classic peers.

The recommended architecture is two separable layers: a TripTorrent-native path using M3-style private discovery and relayed transfer, and an optional classic BitTorrent adapter. A verified payload may be advertised in either or both networks, but each path keeps its own identifiers, discovery, connections, state, and privacy label. Bridges may be useful later as explicitly operated endpoints; they are correlation points and are not a default path. BEP 10 capability negotiation is worth a later experiment for upgraded clients, but it cannot make the initial classic handshake or discovery private.

## 2. Source status and scope

This analysis uses the official BEP index and specifications. [BEP 3](https://www.bittorrent.org/beps/bep_0003.html) is Final; [BEP 5](https://www.bittorrent.org/beps/bep_0005.html), [9](https://www.bittorrent.org/beps/bep_0009.html), [10](https://www.bittorrent.org/beps/bep_0010.html), [11](https://www.bittorrent.org/beps/bep_0011.html), [12](https://www.bittorrent.org/beps/bep_0012.html), [14](https://www.bittorrent.org/beps/bep_0014.html), [19](https://www.bittorrent.org/beps/bep_0019.html), [23](https://www.bittorrent.org/beps/bep_0023.html), [27](https://www.bittorrent.org/beps/bep_0027.html), and [29](https://www.bittorrent.org/beps/bep_0029.html) are Accepted. [BEP 52](https://www.bittorrent.org/beps/bep_0052.html) is Draft. [BEP 47](https://www.bittorrent.org/beps/bep_0047.html) supplies padding files needed by multifile hybrids. Current [libtorrent 2.x documentation](https://www.libtorrent.org/reference-Torrent_Info.html) confirms the deployed model in which one torrent can retain both v1 and v2 hashes. Status does not imply universal client support; implementations must negotiate and test features independently.

M6 validates parsing and mapping only. It does not add a tracker client, DHT client, BitTorrent peer protocol, uTP, bridge, or dual-network runtime.

## 3. BitTorrent v1, v2, and hybrid identity

| Form | Identity | Payload structure | Consequence |
| --- | --- | --- | --- |
| v1 | SHA-1 of the exact bencoded `info` dictionary | SHA-1 piece list; pieces may cross file boundaries | Names, layout, piece length, and hashes affect identity |
| v2 | SHA-256 of the exact bencoded `info` dictionary | Per-file SHA-256 Merkle trees over 16 KiB leaves; files align to pieces | The infohash is still metadata identity, not a bare payload digest |
| hybrid | Both v1 SHA-1 and v2 SHA-256 over one compatible `info` dictionary | Both representations, including v1 padding rules for multifile alignment | Both full aliases identify the same torrent only after consistency validation; some peer/tracker fields use the truncated v2 form |

Canonical bencoding matters. Importers must hash the exact validated `info` byte slice, reject duplicate or unsorted dictionary keys and noncanonical integers/string lengths, and must not decode then re-encode before hashing. Hybrid importers calculate and retain both hashes and validate both payload structures.

The vectors demonstrate the distinction. The same 30-byte payload has TripTorrent ID `db5d…512d`; changing only the v1 torrent name changes `btih` from `208b…7c51` to `18cc…f30e`. A single-file TripTorrent blob can therefore have many BitTorrent aliases. A multifile torrent is an ordered path tree and cannot safely be represented as concatenated bytes; a future TripTorrent tree/bundle identity is required before lossless multifile import.

## 4. TripTorrent and BitTorrent content models

The current TripTorrent prototype hashes raw single-file bytes with BLAKE3 and divides them into fixed 32 KiB chunks with BLAKE3 chunk hashes. It should retain that namespace. Store aliases as typed records such as `tt:blob`, `bt:btih`, and `bt:btmh`, with provenance and verification state. Never infer one namespace from another without the committed metadata or bytes.

BitTorrent pieces cannot generally be reused as TripTorrent chunks: v1 permits arbitrary piece sizes and cross-file pieces; v2 has 16 KiB Merkle leaves but exposes piece-layer nodes based on its selected piece length; TripTorrent uses 32 KiB flat chunks. An importer should stream bytes once through all required hashers and build the TripTorrent manifest while validating the BitTorrent representation. The CPU work is duplicated but I/O need not be.

Approximate flat hash storage per payload byte is:

| Manifest | Hash overhead |
| --- | ---: |
| TripTorrent, 32 KiB chunks | 0.0977% |
| v1, 256 KiB pieces | 0.00763% |
| v2 piece layer, 256 KiB pieces | 0.0122% |
| TripTorrent + v1 | 0.1053% |
| TripTorrent + v2 | 0.1099% |
| all three | 0.1175% |

These figures exclude bencoding, file-tree roots, Merkle padding, and path metadata. Metadata alone can establish BitTorrent aliases, but it cannot prove local payload presence or calculate the TripTorrent ID.

## 5. `.torrent` import and export semantics

`triptorrent import file.torrent` should conceptually perform three distinct operations:

1. **Metadata import:** strictly parse the metainfo, retain the exact `info` bytes, calculate typed v1/v2 aliases, validate hybrid consistency and piece layers, and store an `unverified` record. No content is advertised.
2. **Local payload association:** resolve every path inside an isolated import root, validate all BitTorrent hashes, compute the TripTorrent ID/manifest in the same read, copy verified bytes into managed storage, and mark aliases verified.
3. **Network acquisition:** require a separately selected network mode. Metadata import never starts classic discovery or transfer.

Names and path components are untrusted. Reject absolute paths, empty/`.`/`..` components, embedded separators, controls, platform-reserved characters/names, trailing dots/spaces, normalization collisions, case-fold collisions on case-insensitive filesystems, duplicate target paths, symlink escapes, and unreasonable depth/count/size. Display names never become paths without sanitization.

Trackers (`announce`, `announce-list`), web seeds (`url-list`), explicit sources, and unknown top-level fields remain inert, provenance-tagged hints. The `private` flag stays bound to the exact info dictionary. Unknown `info` fields must be preserved because they affect identity; unsupported security-sensitive semantics block network use. Piece layers are outside `info` in v2 and must be checked against file roots before trust.

Export is allowed only from verified content. Re-emitting an imported metainfo byte-for-byte preserves its identity. Generating a new torrent requires an explicit version, name/path tree, piece length, tracker/web-seed policy, and private setting, and creates new BitTorrent identity. A future multifile TripTorrent model is required for lossless new multifile exports.

## 6. Magnet semantics

[BEP 9](https://www.bittorrent.org/beps/bep_0009.html) defines `xt=urn:btih:…` for v1 and `xt=urn:btmh:1220…` for the full v2 SHA-256 multihash. A hybrid magnet can contain both. Repeated exact topics and hints must be retained. `dn` is display text, `tr` is a tracker hint, `x.pe` exposes an explicit peer, and deployed `ws`, `as`, and `xs` values identify network sources.

An ordinary magnet is an import hint or an explicit classic-mode fetch request. A BitTorrent hash becomes a verified alias only after metadata and payload verification. It never supplies the random M3 discovery capability and therefore carries no TripTorrent discovery privacy claim.

A future TripTorrent link should carry logically separate fields for TripTorrent content identity, M3 discovery capability, optional BitTorrent aliases, and optional compatibility hints. A new magnet `xt` namespace can aid existing parsers, while a dedicated URI may make mode and capability handling clearer. Syntax remains deliberately unfrozen. Discovery secrets must not appear in tracker, DHT, display-name, or other fields that classic clients propagate.

## 7. Discovery compatibility matrix

| Mechanism | Exposure | TripTorrent-only | Explicit compatibility/classic mode |
| --- | --- | --- | --- |
| Tracker / compact response (BEP 3/23) | Tracker sees requester IP, infohash, timing and counters; peers receive endpoints | Forbidden | Allowed with visible consent |
| Mainline DHT (BEP 5) | Queried nodes see source IP and infohash-derived key; responses expose endpoints | Forbidden | Allowed |
| PEX (BEP 11) | Connected peers exchange further peer endpoints; poisoning/amplification remain possible | Forbidden | Allowed after classic connection |
| Local discovery (BEP 14) | Multicast reveals infohash and port to the local/site network | Forbidden | Separately enabled |
| Explicit peer (`x.pe`) | Both endpoints connect directly and learn each other | Forbidden | Allowed |
| Web seed (BEP 19) | HTTP server sees requester IP, object/path, ranges, timing and volume | Forbidden | Allowed |
| M3 TripTorrent discovery | Capability-derived key is separated from content identity; relays/gateways divide knowledge under non-collusion assumptions | Required path | May operate independently beside classic discovery |

None of the classic mechanisms meets the intended M3 privacy model. uTP (BEP 29) changes congestion-controlled transport over UDP but does not hide endpoints or content identifiers.

## 8. Transfer architecture comparison

| Architecture | Benefit | Leakage / downgrade | Unmodified clients | Decision |
| --- | --- | --- | --- | --- |
| Import only | Reuses artifacts and verified bytes with the smallest trusted surface | No classic leakage unless separately selected; no access to classic-only peers | Produce/consume artifacts only | Implement first |
| Dual network | Reuses existing swarms while native peers use TripTorrent | Classic path exposes hashes/endpoints and permits cross-network timing correlation | Yes, on classic side | Recommended optional adapter |
| Bridge/gateway | Makes classic availability useful to TripTorrent-only recipients | Bridge sees both identities and timing, becomes a correlation/abuse point | Yes, through bridge | Later, explicit and independently operated |
| BEP 10 native extension | Upgraded clients can negotiate capabilities and reuse schedulers | Negotiation occurs after classic discovery/handshake has already exposed endpoints and infohash | No TripTorrent behavior from unmodified peers | Worth a later experiment, never a privacy bootstrap |
| Direct peer interoperability | Maximum classic swarm reach | Direct endpoint exposure; classic wire and discovery inherit classic privacy | Yes | Only in compatibility/classic mode |

The selected design is import-first plus an optional dual-network adapter. The node core owns verified content and typed aliases; discovery and transport adapters consume those records independently. There is no automatic fallback between adapters.

## 9. Downgrade and privacy modes

| Mode | Discovery and data path | Failure behavior | Privacy statement |
| --- | --- | --- | --- |
| `triptorrent-only` | TripTorrent link capability, M3 discovery, TripTorrent relayed transfer | Fail closed if no native route exists | Only documented TripTorrent properties apply |
| `dual` | Native and classic adapters run as separately reported paths | Each path may fail independently; enabling requires explicit consent | Classic activity loses endpoint/content-query protections |
| `classic` | Trackers/DHT/PEX/LSD/web seeds/direct peers as configured | Ordinary BitTorrent behavior | No TripTorrent privacy claim |

Modes are persistent, API-visible transfer policy. Import does not select a mode. Active network paths and any transition must be observable and auditable. Received metadata cannot request a weaker mode. A peer cannot convert absence of native providers into classic fallback. Future capability negotiation must authenticate the selected mode and reject stripping or substitution.

## 10. Client adoption model

An existing client can add TripTorrent without replacing its torrent engine. The minimum boundary is:

- parse and retain TripTorrent links, capabilities, and typed aliases;
- add an alternate discovery provider beside tracker/DHT/PEX;
- add the TripTorrent relay/session transport as another peer source;
- adapt verified TripTorrent chunks to the existing disk and swarm scheduler;
- run both BitTorrent and TripTorrent verification where aliases are claimed;
- persist explicit per-transfer mode and show the active network path.

libtorrent-based applications could place this around metadata/session abstractions, while each UI keeps its own presentation. TripTorrent remains a protocol with conformance vectors; no reference daemon or Rust-specific state is required.

## 11. Private torrents

[BEP 27](https://www.bittorrent.org/beps/bep_0027.html) puts `private=1` inside `info` and requires peers to obtain peers only from the private tracker, disabling DHT, PEX, and other peer sources. This is swarm membership policy and tracker access control, not anonymity: the tracker and direct peers still see endpoints and the infohash.

On import, preserve and enforce the flag on the classic adapter. Do not advertise the torrent to TripTorrent discovery, export it without the flag, bridge it, or obtain peers from another source unless the user has separate authority and explicitly creates a new distribution policy. Metadata-only inspection remains safe; network activation is blocked by default.

## 12. Compatibility threat matrix

| Threat | Classic mechanism | Effect / required control |
| --- | --- | --- |
| Requester-content link | Tracker, DHT, web seed | Observer sees IP plus infohash/object; unavailable in TripTorrent-only mode |
| Endpoint disclosure | Tracker responses, DHT, PEX, LSD, direct peers | Classic peers learn routable addresses; mode UI must state this |
| Cross-network correlation | Dual mode or bridge | Matching payload, aliases, size and timing link identities; use independent sessions and disclose residual risk |
| Malicious metadata | `.torrent`, magnets, BEP 9 | Strict bencoding, bounds, path isolation, hash verification, inert unknown hints |
| SHA-1 collision/downgrade | v1 identity | Treat `btih` as a compatibility alias; verify full expected metadata and payload; prefer v2 when both are promised, never discard v2 |
| Magnet poisoning | Trackers, sources, fetched metadata | Exact-topic verification; hints confer no authority; reject hash mismatch |
| Active mode downgrade | Stripped capability or unavailable native peers | Authenticated mode binding later; never automatic classic fallback |
| Bridge observation | Gateway participates in both networks | Explicit trust/correlation role; no privacy claim against the bridge |
| Resource exhaustion | Huge trees, piece layers, repeated hints | Bound input size, depth, entries, paths, pieces, sources, and hashing work |

Enabling classic mechanisms loses protection of requester/provider endpoints from each other, concealment of content lookup keys from tracker/DHT participants, and separation between requester identity and content. Payload integrity remains available, but it is not an anonymity property.

## 13. Recommended architecture and rejected alternatives

Adopt multiple typed identifier namespaces. Keep the canonical TripTorrent identity independent and bind v1/v2 aliases only after verification. Introduce an eventual tree identity before claiming multifile equivalence. Use a strict import pipeline and a content store shared by isolated network adapters. Implement import-only first, then an explicitly enabled dual-network adapter. Consider opt-in bridges and BEP 10 negotiation only after their correlation and downgrade behavior can be tested. M3 discovery keys and capabilities remain separate from every BitTorrent infohash and from classic discovery.

Rejected directions:

- **Use v2 SHA-256 directly as the TripTorrent ID:** it identifies the `info` dictionary and inherits path/piece choices; v1-only content has no v2 ID.
- **Derive a TripTorrent ID from an infohash:** this mistakes metadata possession for payload verification and preserves arbitrary torrent construction choices.
- **Use one untyped hash field:** equal-looking digests can have different algorithms and semantics; it invites substitution.
- **Automatic classic fallback:** it violates the selected privacy path and makes failure behavior unsafe.
- **Default bridge:** it centralizes cross-network visibility and policy.

## 14. Migration strategy and later milestones

1. **Artifact reuse:** ship strict offline `.torrent`/magnet parsing, typed unverified aliases, path validation, and verified local-payload association.
2. **Alias exchange:** let TripTorrent nodes advertise verified aliases inside authenticated native metadata without publishing M3 capabilities to classic systems.
3. **Explicit dual operation:** add a separate classic adapter and per-transfer `dual`/`classic` consent, with path-specific status and logs.
4. **Controlled bridges:** experiment with independently operated, opt-in bridges and measure correlation, abuse, and throughput.
5. **Native client adoption:** specify capability negotiation and conformance tests for independent clients; evaluate BEP 10 only for sessions already in classic mode.

M7 should expose content identity by namespace, alias verification state, private-torrent restrictions, selected mode, active discovery and transfer paths, and a clear consent step before classic networking. It should never label imported metadata as available content or imply that a classic magnet has M3 privacy.

Later protocol work must resolve the canonical multifile identity, alias-signing/binding format, TripTorrent URI syntax, mode negotiation, metadata size limits, private-torrent authorization, bridge policy, v2 interoperability edge cases, and the M3 wire protocol. Those questions do not block the M6 architecture and are deliberately not implemented here.

## Deterministic research evidence

`test-vectors/m6-bittorrent-interop.json` contains synthetic v1, v2, hybrid, magnet, malformed-bencoding, path, and same-payload/different-identity cases. `triptorrent-interop` is an offline research crate that validates those vectors. The vectors require no Internet access and are not yet normative protocol fixtures.
