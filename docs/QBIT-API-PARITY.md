# qBittorrent WebUI API parity

rustTorrent ships a qBittorrent WebUI API v2 compatibility layer
(`crates/librtbit/src/http_api/handlers/qbit_compat.rs`, mounted at
`/api/v2`) so that *arr apps and other qBittorrent integrations can talk to
it. This document describes how parity with upstream qBittorrent is measured
and enforced, and where we currently stand.

## Current standing (upstream WebAPI 2.16.2, refreshed 2026-10-17)

**69 of 93 in-scope endpoints routed (~74%)** — 26 full, 43 partial — plus 2
legacy aliases (`torrents/pause`, `torrents/resume`) that upstream removed in
WebAPI 2.11. 37 of the 130 upstream endpoints were descoped on 2026-08-30
(marked `out_of_scope`, enforced as unrouted): `search/*` (Indexarr covers
discovery), `rss/*` (native `/rss` + Indexarr are the RSS story), `clientdata/*`,
SSL parameters, and qBittorrent-internal app misc (email, cookies, API keys,
processInfo, getDirectoryContent).

Counts are taken from `qbit_parity_spec.json`; `cargo test -p swarmforge
qbit_parity -- --nocapture` prints the same scoreboard.

Decisions of record: advertised `webapiVersion` stays **2.11.3** and the
rename-family bug is now fixed *forward* (see below); the remaining heavy engine
items (a real download queue, per-torrent rate limits, move-storage, reannounce,
rename) **will be built**, not stubbed — they account for most of the 24
still-`missing` in-scope endpoints.

| Controller | Implemented / in scope | Notes |
|---|---|---|
| `auth` | 2 / 2 | login (per-client backoff) / logout with SID cookies |
| `app` | 6 / 10 | version info (5.x line), `defaultSavePath`, minimal preferences (7 descoped) |
| `torrents` | 46 / 60 | lifecycle, categories, tags (persisted), trackers (add/remove/edit), reannounce, rename (name/file/folder), setLocation/setSavePath, pieces, file prio, per-torrent + share limits, export, webseeds; queue-position endpoints are no-ops |
| `transfer` | 10 / 13 | info (real session limits) + session rate limits + alt-speed mode + session pause/resume |
| `sync` | 1 / 2 | `maindata` (full-update snapshots; no per-client rid deltas) |
| `torrentcreator` | 4 / 4 | task API over the native `create_torrent` (in-memory tasks) |
| `log` | 0 / 2 | no persistent log ring buffer |
| `rss`, `search`, `clientdata` | — | descoped entirely |

### Compat hardening (2026-10-17)

Changes to the compat layer from the polish review (Q1–Q4, H5, the compat half
of H4, and tag persistence):

- **Version**: `app/version` returns `v5.0.0` (was `v0.0.1`). *arr apps gate
  features on this string. `webapiVersion` is unchanged.
- **Preferences**: `app/preferences` reports the seeding-limit fields
  (`max_ratio_enabled`, `max_ratio`, `max_ratio_act`, `max_seeding_time_enabled`,
  `max_seeding_time`) as disabled, using qBittorrent's default values.
- **`app/setPreferences`**: unknown and unsupported fields are ignored, and
  `announce_port` is optional. Read-modify-write clients that post the whole
  preferences object now succeed. A present but invalid `announce_port` still
  returns 400.
- **Sessions**: a compat session expires after one hour of *inactivity* (each
  authenticated request slides the expiry), and idle entries are pruned whenever
  a new session is created.
- **Login backoff**: after five failed `auth/login` attempts from one client IP,
  each further attempt waits twice as long as the last (1s, 2s, 4s, … capped at
  15 minutes), and the response is 429 with the wait time. A successful login
  clears the client's record. Backoff state is in memory only. **Caveat:** behind
  a reverse proxy every client shares the proxy's IP, so one client's failures
  can delay the others. Use per-IP limits at the proxy if that matters.
- **Cookies and origin**: the SID cookie is `SameSite=Strict`. POST requests
  that carry an `Origin` header must name the same authority as `Host`, or they
  get 403. Requests without `Origin` (native clients, curl) are unaffected.
- **Tags**: tags are written to `qbit-tags.json` in the session data directory
  (the default JSON persistence folder) after every change, so they survive
  restarts. An unreadable file is logged and treated as empty. It is replaced on
  the next tag change.
- **Queue and force-start**: `torrents/setForceStart` is routed to resume.
  `torrents/topPrio`, `bottomPrio`, `increasePrio` and `decreasePrio` succeed as
  no-ops, since there is no download queue and `queueing_enabled` is `false`.
- **Errors**: compat handlers return plain-text errors through one helper, with
  lowercase messages. Serde parse details are no longer echoed to clients.

### Rename-family bug (fixed 2026-08-30)

We advertise `webapiVersion` **2.11.3**, so modern clients (qbittorrent-api,
newer *arr releases) use the post-2.11 vocabulary. This is now served:

- `torrents/stop` / `torrents/start` are routed (sharing the pause/resume
  handlers); the pre-2.11 `torrents/pause` / `torrents/resume` remain as aliases.
- `torrents/info` emits the 2.11 state strings `stoppedDL` / `stoppedUP`.
- `matches_filter` accepts `stopped` / `running` (and still the old
  `paused` / `resumed`), so `filter=stopped` no longer returns every torrent.
- `torrents/add` reads the 2.11 `stopped` form field as well as `paused`.

### Conflict audit (2026-08-30)

No hard route conflicts: the native API owns `/` and the compat layer owns
`/api/v2`; nothing native is mounted under `/api/*`, so every endpoint that was
then missing had a free path. Auth layering is also sound — the main
Bearer/Basic middleware is applied before the qbit router is nested, so
`/api/v2` correctly runs its own SID-cookie auth against the same credential
store. **Trap**: future qbit sub-routers (`sync`, `rss`, `log`, …) must be
nested inside `protected_router` in `make_qbit_router`, or they ship
unauthenticated.

The version-skew conflicts (items 1–5) were the 2.11 pause→stop rename family
and the rate-limit state disagreement; all are now resolved:

1. ~~**`torrents/stop`/`start` missing**~~ — routed (see the rename-family
   section above).
2. ~~**State strings** `pausedDL`/`pausedUP`~~ — `torrents/info` now emits
   `stoppedDL`/`stoppedUP`.
3. ~~**Filter values** `stopped`/`running`~~ — `matches_filter` now recognises
   both the 2.11 and pre-2.11 spellings, so `filter=stopped` no longer returns
   every torrent.
4. ~~**`torrents/add` `stopped` field**~~ — now read alongside `paused`.
5. ~~**Rate-limit state disagreement**~~ — `transfer/downloadLimit`/
   `uploadLimit`/`setDownloadLimit`/`setUploadLimit` now read and write the same
   session limiter as the native `/torrents/limits` API, and
   `transfer/speedLimitsMode`/`toggleSpeedLimitsMode`/`setSpeedLimitsMode`
   reflect the native `/speed/alt` toggle. `transfer/info` now also reports the
   real session `dl_rate_limit`/`up_rate_limit` (was hardcoded to 0).

Still open:

6. ~~**`app/setPreferences` rejects instead of ignoring**~~ — resolved (see
   compat hardening above).
7. **RSS data-model gap** (affects future bridging): the native store is flat
   (feeds keyed by name, rules with one feed + one regex); qBittorrent has
   folder hierarchies (`Folder\Feed` paths) and rules keyed by name with
   `mustContain`/`mustNotContain`/`affectedFeeds[]`. A bridge needs flat-folder
   emulation and a rule-shape mapping; `rss/addFolder`/`rss/moveItem` have no
   clean mapping.

### Suggested parity tiers

Tier 1 (high value) and the thin backend bridges from tier 2 are now largely
done: `torrents/stop`/`start`, `sync/maindata`, `app/defaultSavePath`,
`torrents/tags` + tag CRUD, `torrents/trackers`, `torrents/addTrackers`,
`torrents/addPeers`, `torrents/filePrio`, `torrents/pieceStates`/`pieceHashes`,
`torrents/export`, `torrents/webseeds`, `torrents/count`, the `transfer/*Limit*`
+ speed-limits-mode family, and `transfer/pauseSession`/`resumeSession`.

Remaining work, roughly by cost:

1. **Needs a new engine method** (the `will be built` items): a real download
   queue behind `topPrio` / `bottomPrio` / `increasePrio` / `decreasePrio`
   (routed as no-ops for now), `setSuperSeeding`, `toggleSequentialDownload`,
   `setAutoManagement`, `setDownloadPath` (incomplete-file path). Force-start
   gets its own state once a queue exists (`setForceStart` currently resumes). And the
   **auto-enforcement** half of share limits (`setShareLimits` currently stores
   and reports the limits but does not auto-pause/remove at the threshold — a
   periodic session task is the v2). Done since: per-torrent rate limits
   (`ratelimit_override` on `ManagedTorrentShared`, enforced by the live
   limiter), `reannounce` (signals the live re-discovery notify), **file/folder/
   display rename** (see below), and **`setLocation`/`setSavePath`**
   (whole-torrent relocation — see below).
2. **Thin bridges still open**: `torrents/setComment`, web-seed mutation
   (`addWebSeeds` / `editWebSeed` / `removeWebSeeds` — `web_seed_urls` is
   currently immutable). (`torrentcreator/*` is now done — an in-memory task
   store over the native `create_torrent`; tasks are not persisted.)
3. **Probably out of scope**: `search/*` (plugin system),
   `app/sendTestEmail`, `app/processInfo`, `clientdata/*`,
   `torrents/SSLParameters`, `log/*`.

### Relocation (setLocation, v1, 2026-08-31)

`torrents/setLocation` and `torrents/setSavePath` move every file to a new root:

- New `TorrentStorage::move_root(new_root)` primitive: the filesystem backend
  moves each file to the same relative path under the new root (reusing the
  no-clobber rename core) and re-anchors its (now `RwLock`-wrapped) root; mmap
  forwards. All-or-nothing with rollback.
- The persistent anchor is a new `output_folder_override` on
  `ManagedTorrentShared` (seeded from `options.output_folder`, read by the
  storage factory's `create`), rather than unfreezing the immutable `options`.
- `ManagedTorrent::set_location()` is **stopped-only** (409 when live) and
  **same-filesystem only** — a cross-device `rename` (EXDEV) is refused and
  rolled back, not copied. `qbit_save_path` reports the new root after a move.
- **v2**: cross-filesystem relocation (async copy+delete, à la
  `move_completed_torrent`), live relocation, and persistence across restart.

> **⚠️ Restart caveat (rename and relocation both).** These overrides are
> in-memory only. Persistence still records the add-time `output_folder` and
> derives filenames from the immutable `.torrent` info, so after a
> rename/relocation **and a restart** the torrent re-adds at its original root
> with original names while the data sits at the new location — a recheck finds
> nothing and re-downloads, orphaning the moved copy. Until v2 persistence
> lands, treat rename/relocation as effective only within the running session.

### File rename (v1, 2026-08-31)

`torrents/renameFile`, `renameFolder`, and `rename` (display name) are
implemented:

- New `TorrentStorage::rename_file(file_id, new_relative)` primitive: the
  filesystem backend moves the file on disk and re-points its cached open
  handle at the new path (mmap forwards; other backends default to an error).
- `ManagedTorrent::rename_files()` is **stopped-only** (returns 409 when live,
  which sidesteps live-handle / Windows-open-file / mmap-remap hazards): it
  validates the batch (relative paths, no `.`/`..`/root, no collisions),
  moves each file, then swaps in rebuilt `TorrentMetadata` with updated
  `file_infos` and prunes emptied source dirs. All-or-nothing with rollback.
- `torrents/rename` sets a `name_override` on `ManagedTorrentShared`, surfaced
  in `torrents/info` / `sync/maindata` via `ManagedTorrent::name()`.
- **v1 limitations**: renames are not persisted across restarts (re-derived
  from the immutable `.torrent` info on load), and require the torrent stopped.
  Live rename is the documented v2, gated behind the differential-test harness.

## The parity checker framework

The source of truth is
`crates/librtbit/src/http_api/handlers/qbit_parity_spec.json`: one entry per
upstream endpoint (`controller/action`), with its HTTP method and a parity
status:

- `full` — routed, semantics close enough for real clients
- `partial` — routed, but with stubbed fields or gaps (see `notes`)
- `missing` — not routed; candidate for future work
- `out_of_scope` — not routed by explicit decision

Entries with `"upstream": false` are legacy endpoints we serve that upstream
has removed.

Tests in `crates/librtbit/src/http_api/handlers/qbit_parity.rs` enforce the
spec against the real router (they run in the normal
`cargo test --workspace` CI job):

- `spec_matches_router` — probes every spec entry against
  `make_qbit_router()`; `full`/`partial` must be routed, `missing`/
  `out_of_scope` must 404. Implementing or removing an endpoint without
  flipping its spec status fails CI.
- `every_compat_route_is_tracked_in_spec` — scans the route literals in
  `qbit_compat.rs` so no compat route can land untracked.
- `routed_responses_match_declared_shapes` — every routed entry declares a
  `response` (`content`: `json`, `text` or `bittorrent`; JSON entries add a
  `schema` with the documented upstream field set and types). The test builds
  a session holding a real two-file torrent (with a category, a tag and a
  finished torrent-creator task, so collections are non-empty), probes every
  routed GET, plus every POST that accepts a no-op probe, and validates each
  body. Upstream fields we do not emit yet are listed per entry in
  `missing_fields` (dotted paths, `*` for an array element or map value); a
  listed field that starts appearing fails the test, so the list cannot go
  stale. The schema syntax is described in the `qbit_parity.rs` module docs.
- `torrent_lifecycle_through_compat_router` — add (multipart) → stop → start
  → stop → recheck → setCategory → delete, driven only through the compat
  router against a real session, asserting what a client observes via
  `torrents/info` after each step (and validating its shape every poll).
- `parity_summary` — prints the scoreboard
  (`cargo test -p swarmforge qbit_parity -- --nocapture`).

### Testing scope (the parity checker is tier 0, not the whole story)

Durable testing is scoped as a ladder; tiers 1 and 2 are in place (WI-25),
3 and 4 are tracked as WI-26:

1. **Shape conformance** (done): `routed_responses_match_declared_shapes`.
2. **Behavioral lifecycle** (done): `torrent_lifecycle_through_compat_router`.
3. **Client-replay fixtures** (WI-26): recorded Sonarr / qbittorrent-api
   request sequences replayed against the router.
4. **Differential harness** (WI-26, nightly): identical requests against a
   real qBittorrent container and rustTorrent, responses diffed field-by-field
   with an allowlist for intentionally-stubbed fields.

Definition of done for every endpoint: route + spec status flip + schema +
lifecycle coverage.

### Workflow

- **Implementing an endpoint**: add the route, flip the spec entry to
  `full`/`partial`, add its `response` contract (schema from the upstream
  WebUI API docs; list fields we do not emit in `missing_fields`), note any
  semantic gaps in `notes`, and extend the lifecycle test if the endpoint
  changes torrent state.
- **Declaring non-goals**: set status to `out_of_scope` (kept enforced as
  unrouted).
- **Tracking upstream**: re-clone qBittorrent and run
  `scripts/refresh-qbit-parity-spec.py <checkout>` — it re-extracts the
  endpoint inventory and POST allowlist from the sources, preserves our
  statuses/notes, adds new upstream endpoints as `missing`, and flags
  upstream-removed endpoints.
