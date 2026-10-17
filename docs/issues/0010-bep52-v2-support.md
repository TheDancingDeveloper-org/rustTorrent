# Issue #10 — BEP 52 Phase 2-5: full BitTorrent v2 support

**Crate:** `librtbit-core` (+ `librtbit`, `librtbit-sha1-wrapper`) · **Effort:** XL (multi-week)

## Problem

Hybrid v1+v2 **parsing** works (meta version read, `Id32`/SHA-256 type exists), but
the v2 data model and verification are not implemented: no `file_tree` parsing, no
`piece_layers` parsing, no merkle/hash-tree piece verification, and v2-only
torrents are rejected. Goal: full v2 download (hybrid and v2-only).

## Current state (file:line)

Parsing structs — `librtbit-core/src/torrent_metainfo.rs`:
- `TorrentMetaV1<Buf>` `:137-165` (has `info_hash: Id20` `:164`, SHA-1 only).
- `TorrentMetaV1Info<Buf>` `:177-217`: `pieces` (v1 SHA-1 concat) `:183`,
  `piece_length` `:185`, `meta_version` `:192`, `length`, `files` `:213`.
  **No `file_tree` field, no `pieces_root` per file.**
- `TorrentMetaV1File` `:552-567`.
- v1 info-hash computation `:26-39` (`torrent_from_bytes`: SHA-1 of raw info bytes,
  `:37-38`).
- v2 detection helpers: `is_v2()` `:486` (`meta_version == Some(2)`), `is_hybrid()`
  `:491` (`has_v1_pieces() && is_v2()`); v2-only **rejection** at `:411-412`
  (`if !has_v1_pieces() → Err(V2OnlyNotSupported)`).
- v1 hash accessors `get_hash`/`compare_hash` `:464-478`.

Error variants (`librtbit-core/src/error.rs:24-60`): `V2MissingFileTree` `:26`,
`V2FileTreeRootIsFile` `:28`, `V2FileTreeDotComponent` `:30`,
`V2MissingMetaVersion` `:32`, `V2UnsupportedMetaVersion` `:34`,
`V2MissingPieceLayers` `:36`, `V2MissingPieceLayersEntry` `:38`,
`V2PieceLayersWrongSize` `:40`, `V2PieceLayerCountMismatch` `:42`,
`V2PieceLayersRootMismatch` `:44`, `V2SmallFileShouldNotHavePieceLayers` `:46`,
`V2SmallFileMissingPiecesRoot` `:48`, `V2ZeroLengthFileHasPiecesRoot` `:50`,
`V2InvalidPieceLength` `:52`, `V2InvalidTorrent` `:54`, `V2OnlyNotSupported` `:58`,
`V2HybridFileListMismatch` `:60`. **The error model is already designed; the
parsing/verification that would emit these is not written.**

Hash types — `librtbit-core/src/hash_id.rs`: `Id20` `:180`, `Id32` `:182` with
`truncate_for_dht()` `:189-193`. SHA-256 wrapper exists but is not integrated:
`librtbit-sha1-wrapper/src/lib.rs` `ISha256` `:14-26`, `Sha256` alias `:150-153`.

v1 piece verification today: `librtbit/src/file_ops.rs:249`
(`compare_hash(piece_index, sha1.finish())`).

## Proposed implementation (phased)

### Phase 2 — `file_tree` + `piece_layers` parsing (`librtbit-core`)

1. Add `file_tree` parsing to `TorrentMetaV1Info` (`torrent_metainfo.rs:177-217`):
   the recursive `file_tree` dict where each leaf is `{ "": { length, pieces root,
   [attr] } }`. Emit the existing `V2FileTree*` errors on malformed input (root is
   file, `.`/`..` components, etc.).
2. Flatten `file_tree` into the same file-list shape v1 uses (path + length +
   `pieces_root: Id32`), so downstream code can treat hybrid/v2 uniformly.
3. Parse top-level `piece_layers` dict (sibling of `info`): map `pieces_root` →
   concatenated layer hashes. Validate sizes against `V2PieceLayers*` errors.
4. For **hybrid** torrents, validate v1 `files` and v2 `file_tree` describe the same
   files (`V2HybridFileListMismatch`).

### Phase 3 — v2 info-hash + identity

1. Compute the **v2 info-hash** = SHA-256 of the raw info dict (store as `Id32`),
   alongside the v1 SHA-1 for hybrids. Extend `torrent_from_bytes`
   (`torrent_metainfo.rs:26-39`) and the `TorrentMetaV1` struct to carry both.
2. Use `Id32::truncate_for_dht()` (`hash_id.rs:189-193`) for DHT/tracker lookups
   that need 20 bytes; use full `Id32` for v2 peer protocol identity.
3. Decide the canonical internal identity for a torrent (info-hash key in session
   maps) — likely keep v1 hash as primary for hybrids, add v2 hash as an alias so
   either magnet form resolves.

### Phase 4 — merkle / hash-tree piece verification

1. Build the per-file merkle tree from `piece_layers`: leaves are SHA-256 of each
   16 KiB block; the layer hashes let you verify a piece without the full tree.
   Use the `ISha256` wrapper (`librtbit-sha1-wrapper:14-26`).
2. Add a v2 verification path parallel to `file_ops.rs:249`: instead of one SHA-1
   per piece, hash 16 KiB blocks → fold up to the piece root and compare against the
   `piece_layers` entry. Padding/last-block rules per BEP 52.
3. Abstract piece verification so the live download loop calls "verify piece N"
   without caring about v1 vs v2 (a `PieceVerifier` enum/trait).

### Phase 5 — v2-only download + peer protocol

1. Remove the `V2OnlyNotSupported` rejection (`torrent_metainfo.rs:411-412`) once
   the above works; build the file list from `file_tree` when `pieces` is absent.
2. v2 peer protocol additions: `hash request`/`hashes`/`hash reject` messages
   (BEP 52) to fetch missing hash-tree layers from peers; v2 handshake/extension
   differences. (Lives in `librtbit-peer-protocol` + the live peer handler.)
3. Hybrid swarms: connect to both v1 and v2 peers for the same content.

## Testing

- Parse real-world fixtures: a v2-only torrent, a hybrid torrent, and malformed
  ones that should hit each `V2*` error variant.
- v2 info-hash matches a known reference (libtorrent-generated fixture).
- Merkle verification: correct piece verifies; a single corrupted block fails;
  last/short blocks handled.
- End-to-end (later): download a v2-only torrent in a local two-node swarm.

## Risks / notes

- Largest issue by far; land it **phase by phase**, each phase shippable
  (parsing → identity → verification → download).
- Touches `librtbit-core` (depended on widely — check the App→Lib matrix) and
  `librtbit-peer-protocol`. Publish carefully; coordinate with StackArr.
- The error model already exists, which de-risks Phase 2 design — match those
  variants exactly.
- Don't break v1: keep the v1 path untouched and add v2 alongside.
