# Issue #11 — WebSeed (BEP 19) Phase 2: HTTP range download

**Crate:** `librtbit` · **Effort:** L · **Depends on:** #6 (SSRF hardening) landing first

## Problem

`url-list` web-seed URLs are parsed, stored, and logged, but never used to fetch
data. Implement an HTTP range-download path that fetches pieces from web seeds,
verifies them with the same hash/commit flow as peer downloads, and falls back to
/ complements peer download.

## Current state (file:line)

- Parse: `librtbit/src/session/mod.rs:690-697` collects `meta.url_list` into
  `Vec<String>`.
- Carried via `InternalAddResult.web_seed_urls`
  (`session/types.rs:361-367`, field `:366`) into
  `ManagedTorrentShared.web_seed_urls` (`torrent_state/mod.rs:197`); logged at
  `session/mod.rs:711`, `:931`. **Never fetched.**
- Peer piece-request loop: `torrent_state/live/peer_handler.rs:658-804`
  (`request_next_pieces`), chunk iteration `:729`, request send `:797`.
- Piece arrival + verify + commit: `peer_handler.rs:825-1033`
  (`on_received_piece`): `write_chunk` `:924`, `check_piece` `:972`,
  `mark_piece_hash_ok` `:980`, `mark_piece_hash_failed` `:1027`,
  `on_piece_completed` `:1013`.
- Reusable storage primitives: `FileOps::check_piece`
  (`file_ops.rs:193-269`, SHA-1 compare at `:249`) and `FileOps::write_chunk`
  (`file_ops.rs:321-370`, vectored write `:357`).
- Session HTTP client: `session/mod.rs:110` (`reqwest_client`).

## Proposed implementation

### Phase 1 — web-seed fetcher task

This is the **BEP 19 "GetRight" style** (HTTP/1.1 byte ranges on a base URL that
maps to the torrent's file layout). For each web-seed URL on a torrent with active
unfetched pieces, spawn a task that:

1. Picks a needed piece (coordinate with the chunk/piece tracker used by
   `request_next_pieces` so peers and web seeds don't double-fetch — treat the web
   seed as another "peer" in the piece picker, or reserve pieces to it).
2. Maps the piece's byte span to file(s) + offset(s) using the torrent's
   `lengths`/file layout, builds the URL(s) per BEP 19 (single-file: range on the
   URL; multi-file: `<base>/<name>/<path>` with ranges), and issues an
   **`Range:` GET** via the **SSRF-safe client from #6**.
3. Streams the response into the same commit path the peer loop uses:
   `write_chunk` → mark chunk → `check_piece` → `mark_piece_hash_ok` /
   `mark_piece_hash_failed` → `on_piece_completed`. **Reuse `FileOps`** rather than
   duplicating verification.

### Phase 2 — integration with the piece picker

- Represent a web seed as a virtual source in the picker so the existing
  rarest-first / endgame logic still applies and pieces aren't requested from both
  a peer and a web seed simultaneously (wasteful). Hook near
  `peer_handler.rs:658-686`.
- Respect global rate limits (`limits.rs`) for web-seed traffic too.

### Phase 3 — fallback / health policy

- Use web seeds when peer download stalls (no/too-slow peers) or always-on as a
  configurable supplement.
- Back off a web seed on repeated 4xx/5xx, hash failures, or range-unsupported
  responses; mark dead after N failures.
- Handle servers that ignore `Range` (return 200 full body) — either consume+slice
  or abandon that seed.

### Phase 4 — config + UI

- Per-torrent / global enable, optional per-seed enable/disable.
- Surface web-seed sources + their throughput in the WebUI (peers/sources view).

## Testing

- Unit: piece→(file,offset,length) range mapping for single- and multi-file
  torrents, including pieces spanning file boundaries.
- Integration: local HTTP server serving a known file with `Range` support; assert
  pieces fetched, verified, committed; assert SSRF guard rejects a web-seed URL
  pointing at loopback/metadata (ties to #6).
- Resilience: server returns 200 (no range), 416, 404 → seed backed off, peer path
  still completes the torrent.

## Risks / notes

- **Must use the #6 SSRF-safe client** — web-seed URLs are attacker-supplied.
- Avoid double-fetching the same piece from peer + web seed (picker integration is
  the crux).
- BEP 17 ("Hoffman style", `httpseeds`) is a different, older scheme — this issue
  is BEP 19 (`url-list`); don't conflate.
- Shared crate → coordinate publish with StackArr.
