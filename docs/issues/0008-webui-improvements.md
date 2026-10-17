# Issue #8 — WebUI: free-disk-space, availability bar, display density

**Crate:** `librtbit` (webui + small backend) · **Effort:** M

Three low-effort UX wins from qBittorrent 5.2.0. The frontend is React 19 + Vite 8
+ Tailwind 4 + Zustand, embedded into the binary via `include_str!`
(`librtbit/src/http_api/webui.rs:1-41`) after `webui/post-build` renames bundles
to fixed `assets/index.{js,css}`. API base URL resolved in
`webui/src/http-api.ts:46-62`.

## (a) Free-disk-space indicator

**Backend: not exposed yet.** No disk/statfs endpoint exists in
`librtbit/src/http_api/handlers/`.

Plan:
1. New handler (e.g. `handlers/stats.rs` or extend `handlers/configure.rs`)
   returning free/total bytes for the relevant save path(s). Use a crate already
   in the tree if present, else `nix::sys::statvfs` (Unix) / `GetDiskFreeSpaceEx`
   (Windows) behind `cfg`. Report per default-save-path; optionally per category
   path. Cache briefly (statvfs is cheap but called on a poll).
2. Add `DiskSpaceInfo` to `webui/src/api-types.ts`; add `getDiskSpace()` to
   `webui/src/http-api.ts`.
3. Render in a footer/status component (`Footer.tsx` or new
   `DiskSpaceIndicator.tsx`): "X GiB free". Warn-color under a threshold.

## (b) Per-torrent piece-availability bar

**Largely already built** — this issue is mostly *surfacing* it.

- Backend endpoint exists: `GET /torrents/{id}/haves` (SVG or binary bitfield),
  `librtbit/src/http_api/handlers/torrents.rs:113-193` (`x-bitfield-len` header
  on binary at `:188`).
- Frontend component exists: `webui/src/components/compact/PiecesCanvas.tsx`
  (canvas bar, fetches via `API.getTorrentHaves`, adaptive 2s/30s poll).
- API client: `webui/src/http-api.ts:401-402` (`getTorrentHaves`).
- Stats already carry `min_piece_availability` / `avg_piece_availability`
  (`webui/src/api-types.ts:231-232`); `"availability"` is a known column
  (`columnStore.ts:23`, sortable in `TorrentTable.tsx:30`).

Remaining work:
1. Confirm/insert `PiecesCanvas` rendering into the row/detail view
   (`TorrentTableRow.tsx`) — it overlaps a future torrent-detail panel, so
   prefer placing it there rather than inline per row for performance.
2. Wire the `availability` column cell to `avg_piece_availability` if not already
   rendered.

## (c) Compact display-density toggle

- View mode (`full`/`compact`) lives in `webui/src/stores/uiStore.ts:11-75` but is
  **in-memory only** (not persisted); default by width (≥1024 ⇒ compact).
- Persisted UI prefs precedent: `columnStore.ts:261-373` uses localStorage keys
  `rtbit-column-{widths,visible,order}`.

Plan:
1. Add `displayDensity: "compact" | "normal" | "spacious"` to `uiStore.ts`,
   persisted to localStorage (mirror the `columnStore` pattern). Persist
   `viewMode` too while here.
2. Thread density into row height / padding in `CompactLayout.tsx` and
   `TorrentTableRow.tsx` (and `CardLayout.tsx`).
3. Add a toggle in the toolbar (`Toolbar.tsx`).

## Build / test

- `cd webui && npm run build` then the Rust build picks up `dist/` via
  `include_str!`; remember `post-build` must run to produce fixed asset names.
- Frontend: component/store unit tests where the harness exists; manual golden-path
  check via `/run` or the verify skill.
- Backend disk endpoint: unit test the byte math; integration test the route +
  auth.

## Risks / notes

- `include_str!` means the WebUI must be rebuilt and committed (or built in CI)
  for backend changes to ship the new assets — confirm the CI/Docker build runs
  the webui build (`Dockerfile`, `.woodpecker.yml`).
- Disk-space syscall differs per-OS; gate with `cfg` and test on Linux (the deploy
  target, Node B).
- Shared crate → coordinate publish with StackArr.
