# Issue #7 — Per-category share limits (ratio / seed-time)

**Crate:** `librtbit` · **Effort:** L

## Problem

Extend seed-ratio and seed-time limits to per-category granularity. qBittorrent
5.2.0; pairs with existing category support.

## Important finding

**Seed-ratio / seed-time enforcement does not exist yet — not even globally.**
This issue is therefore "build share-limit enforcement, then make it
category-aware", not a simple extension.

## Current state (file:line)

- Limits config today is bandwidth/peers only:
  `librtbit/src/limits.rs:9-18` (`LimitsConfig`: `upload_bps`, `download_bps`,
  `peer_limit`, `concurrent_init_limit`); `Limits` rate-limiter `:61-96`.
- qBit-compat surface has `max_seeding_time` / `seeding_time_limit` fields but
  they are **placeholders returning `-1`**, unenforced:
  `librtbit/src/http_api/handlers/qbit_compat.rs` (~`:280-290`).
- Categories: `TorrentCategory { name, save_path }`
  `librtbit/src/category.rs:8-13`; `CategoryManager`
  (`RwLock<HashMap<String, TorrentCategory>>`) `:15-83`; held at
  `session/mod.rs:128`; persisted `session/mod.rs:1321-1327`.
- Per-torrent category: `ManagedTorrentShared.category`
  `torrent_state/mod.rs:200`; set via `session/mod.rs:1311-1319`; read at
  `api.rs:256` and `session/mod.rs:1174`.
- Ratio data is available: `handle.stats()` exposes uploaded/downloaded
  (`torrent_state/stats.rs:70-83`), and seeding time can be derived from the
  completion timestamp.

## Proposed implementation

### Phase 1 — share-limit model + global enforcement

Define a reusable limit type:

```text
ShareLimits {
  ratio_limit: Option<f64>,          // stop seeding at this up/down ratio
  seed_time_limit: Option<Duration>, // stop seeding after this much seed time
  on_limit: ShareLimitAction,        // Pause | Remove | RemoveAndDelete
}
```

- Add global defaults to session settings (persisted).
- Add a periodic evaluator (a tick task, or extend the existing completion watcher
  `session/mod.rs:1146-1189`): for each seeding torrent, compute ratio from
  `stats()` and seed time from the finished timestamp; if a limit is hit, apply
  `on_limit`.
- Track "seeding since" — when a torrent completes (download-finished event,
  `torrent_state/live/mod.rs:682-687`), record the timestamp so seed-time is
  measurable across restarts (persist it).

### Phase 2 — per-torrent override

Add `Option<ShareLimits>` to `ManagedTorrentOptions`
(`torrent_state/mod.rs:111-122`), persisted per torrent. Resolution order:
**per-torrent → per-category → global default.**

### Phase 3 — per-category

Extend `TorrentCategory` (`category.rs:8-13`) with `Option<ShareLimits>`.
Persist (`persist_categories`, `session/mod.rs:1321-1327`). In the evaluator, when
a torrent has no per-torrent override, look up its category
(`ManagedTorrentShared.category`) and apply that category's limits.

### Phase 4 — API + WebUI

- Make the qBit-compat fields (`qbit_compat.rs` ~`:280-290`) return real values and
  accept setters (`setShareLimits`, category limit endpoints) for *arr compat.
- Native endpoints for category-limit CRUD.
- WebUI: per-category limit fields in the category editor; per-torrent override in
  the torrent context menu.

## Testing

- Unit: limit resolution precedence (torrent > category > global); ratio/seed-time
  threshold math; `on_limit` actions.
- Integration: a torrent crossing a ratio limit pauses; a category limit applies to
  members without overrides; seed-time survives a restart (timestamp persisted).
- qBit-compat: setter/getter roundtrip matches qBittorrent semantics
  (`-1` = unlimited, `-2` = use global).

## Risks / notes

- Persisting "seeding since" correctly across restarts is the subtle part — without
  it, seed-time limits reset on every boot.
- Define qBit's `-1`/`-2` sentinel semantics precisely for *arr compatibility.
- Removing/deleting on limit is destructive — guard with explicit action config and
  default to `Pause`.
- Shared crate → coordinate publish with StackArr.
