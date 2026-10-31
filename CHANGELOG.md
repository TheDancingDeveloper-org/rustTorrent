# Changelog

All notable changes to rustTorrent are recorded here. The full release notes for
each tag are in `docs/releases/<tag>.md`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions before 1.0 may include breaking changes between pre-releases; the
upgrade notes in each release describe what to back up and check.

## [Unreleased]

### Changed

- The desktop client is now the gpui-based `rtbit-fluent`. Releases ship
  `rtbit-fluent-<tag>-linux-x86_64`, `rtbit-fluent-<tag>-amd64.deb` (installs
  `/usr/bin/rtbit-fluent`) and its SBOM. The Windows desktop build
  (`rtbit-fluent-<tag>-windows-x86_64.exe`) is temporarily unavailable: it
  needs a native Windows runner, which does not exist yet (WI-2779). The
  `rtbit-egui` artifacts are no longer published; uninstall the `rtbit-egui`
  package and install `rtbit-fluent`. Some screens of the old client are not
  yet at parity.

### Removed

- The egui client (`rtbit-egui`) and the fluent mockup example.

## [0.2.0-beta.4] - 2026-10-30

### Added

- The native client (`rtbit-egui`) is packaged in the release for Linux and
  Windows: a `.deb` and a Windows executable are attached to the GitHub
  release. It stores credentials in the OS keychain on Linux only.

### Changed

- The native GUI runs on fluent2-egui 0.4.2 and egui 0.37, and popups now
  render above modal dialogs.
- Tags can be renamed, and one tag can be replaced in bulk across torrents.
- The web UI keeps torrent filters, sort and search in the URL history, and
  serves every built asset, including lazy page chunks.

### Fixed

- Adding a stopped torrent through the qbit API, and stopping a torrent that
  is still checking, no longer leave it in the wrong state. A cancelled check
  no longer overwrites a stopped torrent.
- Torrents are stopped before the bitfield flusher and the uTP dispatcher
  shut down.
- Each session flush writes its own temp file, so concurrent flushes do not
  clobber one another.

## [0.2.0-beta.3] - 2026-10-29

### Changed

- The native client's file list is a tree, the daemon error bar is the Fluent
  message bar, and the status filter is a navigation pane. Error text stays the
  fixed safe sentences.
- The published container image no longer needs the desktop workspace crates
  to build. This release does not include Indexarr or the native client.

## [0.2.0-beta.1] - 2026-10-21

### Added

- Session and API authentication: logins now survive a restart (sessions are
  persisted next to the credentials, hashed at rest), and revocable API keys
  (`Authorization: Bearer rtb_...`) can be created, listed and deleted through
  `GET`/`POST /auth/api_keys` and `DELETE /auth/api_keys/{id}`. A read-only
  account role is available for integrations that should not mutate the
  session. Sessions are bound to the credential they were issued for, so
  changing the admin password signs every existing session out, and logins are
  capped at 30 days.
- First-run setup is protected by a one-time setup token, printed in the daemon
  log and written to an owner-only `setup-token` file while setup is pending.
  Passwords are hashed with argon2, and an existing plaintext
  `credentials.json` is migrated automatically.
- An optional TOML configuration file (`--config` or `RTBIT_CONFIG`) for
  non-secret settings, with `CLI > environment > file > default` precedence,
  an error on unknown keys, and `rtbit config dump` to print the resolved
  settings.
- An on-complete hook: a shell command and/or a JSON webhook fired when a
  torrent finishes, with `RTBIT_EVENT`, `RTBIT_TORRENT_ID`, `RTBIT_INFO_HASH`,
  `RTBIT_TORRENT_NAME`, `RTBIT_OUTPUT_FOLDER` and `RTBIT_COMPLETED_AT` in the
  environment. It is not fired for torrents that were already complete when
  the session was restored.
- Engine controls for the active-download and active-seed limits, a minimum
  free-disk guard before starting a torrent, and `.part` file handling for
  in-progress payloads.
- Logging improvements: `--log-file-rotation never|hourly|daily`,
  `--log-file-max-files N`, and `--log-format text|json`.
- Operation improvements: an unauthenticated `/healthz` route, a Docker
  `HEALTHCHECK`, hardened systemd units (`Restart=on-failure`,
  `TimeoutStopSec=30`), and a per-user unit that runs `rtbit` by name.
- A startup banner reporting the Web UI URL, authentication state, state and
  download folders, and the listen address.
- Indexarr integration flags: `--indexarr-enabled`, `--indexarr-url`,
  `--indexarr-api-key` and their `RTBIT_INDEXARR_*` environment equivalents.
- Desktop OS integration: opening `.torrent` files and magnet links with the
  app, forwarding them to the running instance instead of starting a second
  one (including Linux `%U` handling), a tray icon, autostart, an updater, and
  a themed splash screen with a failure state and retry after a 20 s timeout.
- Web UI accessibility and keyboard work: design tokens and primitives, a
  redesigned app shell with a mobile navigation drawer and an "engine
  unreachable" panel with retry, labelled toolbar and search controls, an
  `aria-pressed` theme toggle, a torrent table with roving tabindex,
  `Shift+F10`/context-menu support and `aria-sort` headers, keyboard column
  resizing, 44 px rows on coarse pointers, and keyboard-operable torrent
  context menus.
- Web UI feature work: settings search, restart markers, an empty state and
  expiry notice for the queue, paste and drag-and-drop torrent adding, toasts,
  a connection-lost banner, a keyboard shortcuts overlay with view
  preferences, and consistent theming and contrast on the feature pages.
- WebUI API v2 compat: the torrent creator now accepts the full task option
  set (private, comment, URL seeds, `torrentFilePath`, `startSeeding`, and v2
  or hybrid formats).
- WebUI API v2 compat: sequential download and first/last piece priority are
  stored and reported (`toggleSequentialDownload`,
  `toggleFirstLastPiecePrio`, `seq_dl`, `f_l_piece_prio`), and `picker` uses
  them.
- WebUI API v2 compat: `log/main` and `log/peers` over a bounded in-memory log
  ring, and session peer bans through `transfer/banPeers`.
- WebUI API v2 compat: moving and renaming a running torrent now work and are
  persisted (`setLocation`, `setSavePath`, `renameFile`, `renameFolder` no
  longer require a stopped torrent).
- WebUI API v2 compat: `clientdata/load` and `clientdata/store` over a small
  JSON key/value file in the session directory, and
  `torrents/properties` now reports `comment`, `created_by` and
  `creation_date`.
- WebUI API v2 compat: trackers can be added, edited and removed on a live
  torrent, and force-reannounce works without a restart.
- Native torrent tags: a session tag registry plus a per-torrent tag set,
  persisted by the JSON session store (`tags.json` and per-torrent state). The
  WebUI API v2 compat tag endpoints (`torrents/tags`, `createTags`,
  `deleteTags`, `addTags`, `removeTags`, `setTags`), the `tags` field and `tag`
  filter of `torrents/info`, and `torrents/add`'s `tags` field now use them. A
  legacy `qbit-tags.json` is imported once at startup.
- Torrent queue ordering and force-start: each torrent has a persisted queue
  position that the policy pass follows when it fills the active-download and
  active-seed slots, and force-started torrents run outside the limits. The
  queue limits are saved with the session settings. WebUI API v2 compat:
  `topPrio`/`bottomPrio`/`increasePrio`/`decreasePrio` move torrents in the
  queue, `setForceStart` sets force-start, `torrents/info` reports `priority`,
  `force_start` and the `queuedDL`/`queuedUP`/`forcedDL`/`forcedUP` states, and
  `app/preferences` reads and sets `queueing_enabled`, `max_active_downloads`
  and `max_active_uploads`.
- Per-torrent speed limits are saved in session state and survive a restart,
  and have a native API (`GET`/`POST /torrents/{id}/limits`). WebUI API v2
  compat: `torrents/info` and `torrents/properties` report `dl_limit`/`up_limit`
  (were always -1), `setDownloadLimit`/`setUploadLimit` save the torrent, and
  `torrents/add` accepts `dlLimit`/`upLimit`.
- WebUI API v2 compat conformance tests: every routed endpoint in
  `qbit_parity_spec.json` declares a response contract that is validated
  against a live torrent, plus an add/stop/start/recheck/setCategory/delete
  lifecycle test through the compat router.
- WebUI API v2 compat: `transfer/getSpeedLimits` and `transfer/setSpeedLimits`
  over the native session limiter and alt-speed config; `transfer/info` and
  `sync/maindata` report real `dht_nodes` from the DHT routing tables.
- WebUI API v2 compat: `sync/maindata` now answers a known `rid` with a delta
  (changed fields only, plus `torrents_removed` / `categories_removed` /
  `tags_removed`) instead of a full snapshot every poll, and
  `sync/torrentPeers` is routed with the same rid encoding.
- Packaging: RPM packages (`rtbit-<tag>-x86_64.rpm`) alongside the Debian
  package, a CycloneDX SBOM in every release, and Authenticode signing of the
  Windows executable with an RFC 3161 timestamp. `verify-release.sh` checks
  the signature and timestamp.
- Packaging: per-release channel manifests for AUR (`rtbit-bin`), Homebrew,
  Flatpak (offline, with a signed vendor tarball) and winget. Rendering fails
  closed when a digest or placeholder is missing.
- Release verification now requires a valid keyless cosign signature over
  `SHA256SUMS` and every listed artefact before the release is published.

### Changed

- WebUI API v2 compat coverage grew from 66 to 81 of 97 in-scope endpoints
  (36 of them full, 45 partial) across `auth`, `app`, `torrents`, `transfer`,
  `sync`, `torrentcreator`, `clientdata` and `log`. `log` and `transfer` are
  now fully routed.
- `app/version` reports `v5.0.0` (was `v0.0.1`), which is the version gate
  several *arr apps check before enabling features. The advertised
  `webapiVersion` is still `2.11.3`.
- Torrent tags moved from `qbit-tags.json` to `tags.json` plus per-torrent
  state. A legacy `qbit-tags.json` is imported once at startup.
- `--log-file` now defaults to the `info` filter instead of
  `librtbit=debug,info`, so log files are less verbose unless asked.
- The compat layer is split into per-controller modules. This is
  behaviour-neutral and internal only.
- Web UI polling is visibility-aware, the speed tab reads from the session
  store, and feature pages are loaded lazily, reducing background work while
  the tab is hidden.
- The desktop app now builds for Linux and Windows only; macOS bundle targets
  were removed. The self-updater is behind the `self-update` feature so the
  Flatpak build can omit it.
- The `torrents` table gains columns for queue and share state, `.part`
  handling, output-folder roots, file renames and the have-bitfield. They are
  added with `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` on startup.

### Fixed

- WebUI API v2 compat: `torrentcreator/status` emits `taskID` (was `taskId`),
  matching the upstream field name.
- WebUI API v2 compat: `setPreferences` applies speed-limit and seeding-limit
  fields partially, ignores unknown fields, and treats `announce_port` as
  optional instead of rejecting the whole request.
- WebUI API v2 compat: session IDs expire after one hour of inactivity and
  idle sessions are purged, instead of accumulating for the process lifetime.
- Torrents that were queued, held for disk space or paused by a share limit no
  longer come back permanently paused after a restart; those flags are
  persisted and restored.
- Share limits, the uploaded byte total and the `.part` choice are persisted
  correctly, including when they change through the compat API. Session state
  is also persisted on a 60 s timer and on shutdown rather than only at
  selected points.
- PostgreSQL persistence no longer binds the same parameter twice, and writes
  are serialised behind a single lock.
- An on-complete command that hangs is bounded by a timeout, and resuming a
  torrent resets its inactivity clock for share limits.
- Tests no longer write compat tags into the user's real data directory
  (`qbit-tags.json`), which leaked tags between runs.
- Desktop: the app reads hashed credentials correctly, read-only accounts can
  log out, and the CLI no longer consumes the pending setup token.
- Web UI: the `M` shortcut reopens the shortcuts overlay reliably, focus
  follows Home/End scrolling, and the overflow menu is named "More options"
  and operable from the keyboard.
- Web UI status text meets WCAG AA contrast in both themes (emerald-800,
  amber-800 and red-700 on light; red-300 on dark).
- An invalid CORS allow regexp is logged as a warning instead of being
  silently ignored. `RTBIT_CORS_ALLOW_REGEXP` takes precedence over the bare
  `CORS_ALLOW_REGEXP`.

## [0.1.0-rc.5] - 2026-10-18

### Fixed

- Release workflow: pin cosign to v2.6.5. cosign v3 writes only the new bundle
  format and ignores `--output-signature`/`--output-certificate`, so the
  `v0.1.0-rc.4` release failed at signing and rc.4 has container images but no
  binary release.

## [0.1.0-rc.4] - 2026-10-18

### Fixed

- Release workflow: the sign and channels jobs install `gettext-base` when
  `envsubst` is missing, which `sigstore/cosign-installer` needs. The
  `v0.1.0-rc.3` release failed at signing, so rc.3 has container images but no
  binary release.

## [0.1.0-rc.3] - 2026-10-18

### Fixed

- Release packaging: the `.deb` and `.rpm` builds now find the `rtbit` binary
  (`expand: true` on the nfpm binary entry). The `v0.1.0-rc.2` release build
  failed at this step, so rc.2 has no binary release.

## [0.1.0-rc.2] - 2026-10-18

### Changed

- Repository hygiene: restored the full `LICENSE` text; added `NOTICE`,
  `CONTRIBUTING.md` and `SECURITY.md`; corrected stale `../libs` paths.
- Every crate and package is at version `0.1.0-rc.5`, including the `librtbit-*`
  crates, `rtbit`, the desktop app and `fuzz`/`benchv2`.
- Release notes moved from the repository root to `docs/releases/`, and
  `documentation/` was merged into `docs/`. The layout of each path is described
  in `docs/REPOSITORY-LAYOUT.md`.

### Fixed

- An invalid CORS allow regexp is now logged as a warning instead of being
  silently ignored. The regexp can be set with `RTBIT_CORS_ALLOW_REGEXP`; the
  bare `CORS_ALLOW_REGEXP` is still read as a fallback.

## [0.1.0-rc.1]

First release candidate for 0.1.0. Intended for validation; back up the
persistent session directory before upgrading.

### Added

- Expanded qBittorrent WebUI API v2 compatibility from 19 to 64 endpoints.
- File, folder and display renames, with a guard that refuses a rename which
  would overwrite an existing destination.
- Whole-torrent relocation via `setLocation` and `setSavePath`.
- Torrent-creator task API.
- `setShareLimits`, with ratio and seeding-time limits stored and reported
  through the compatible API.
- Release pipeline: `dev` → `main` → `v*` tag promotion. `dev` publishes `:dev`,
  `main` publishes `:staging`, and a `v*` tag publishes the multi-architecture
  image (`:<tag>`, `:latest`, `:beta`), binaries, `.deb` and checksums. Every
  build also publishes an immutable `sha-<commit>` tag.

### Changed

- Replaced the unmaintained `parse_duration` crate with `humantime`.
- Bumped `quinn-proto` to a patched release.
- Refreshed web UI and benchmark-harness dependencies.
- Dependabot version updates now target the `dev` branch.

## [0.1.0-beta.3]

### Added

- `--storage-read-only` and `RTBIT_STORAGE_READ_ONLY`: filesystem storage that
  refuses payload writes, creation, resizing, deletion and directory removal,
  so existing payloads can be verified without modification.
- Native and qBittorrent-compatible force-recheck endpoints, which keep the
  torrent's paused or live state.
- Web UI **Force Recheck** context-menu action.
- Tracker URLs can be added to existing torrents at runtime and persist across
  JSON and PostgreSQL session reloads.

### Changed

- Restored trackers start announcing immediately for live torrents, recovering
  qBittorrent imports whose tracker lists existed only in `.fastresume` data.
- qBittorrent `savepath` values are treated as payload roots for multi-file
  torrents, including the single-entry multi-file metainfo case.
- Corrected output-folder roots persist through both session backends and are
  reflected by the compatible API.
- The reusable engine and protocol packages are published as the `swarmforge`
  / `swarmforge-*` 0.1.0 family. Cargo package names changed; library import
  names such as `librtbit` are preserved through aliases. See
  `docs/SWARMFORGE-0.1.0-RELEASE.md` for the atomic migration guidance.
- Production `/web/` routes use the same-origin API.
- Release tags publish multi-architecture (`linux/amd64`, `linux/arm64`) images.

## [0.1.0-beta.2]

### Added

- Polished management interface with a torrent table, detail tabs (overview,
  files, peers, pieces, transfer history, trackers) and multi-file upload flows.
- Tracker state tracking (working, error, not contacted) with seeders, leechers,
  announce timestamps and the last error; tracker filter in the sidebar.
- Alternative (turtle-mode) speed limits and a weekly schedule.
- RSS feeds, feed history, download rules, categories with category-specific
  save paths, and configurable download and completed folders.
- First-run username and password setup, access and refresh tokens, and logout.
- Live demo generated from the current web UI source.

### Changed

- Source, CI, binary releases and website delivery moved into the monorepo.
- The Docker build and web UI were consolidated so the shipped interface is
  built from the reviewed source tree.

[Unreleased]: https://github.com/TheDancingDeveloper-org/rustTorrent/compare/v0.2.0-beta.4...HEAD
[0.2.0-beta.4]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.2.0-beta.4
[0.2.0-beta.3]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.2.0-beta.3
[0.2.0-beta.1]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.2.0-beta.1
[0.1.0-rc.5]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-rc.5
[0.1.0-rc.4]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-rc.4
[0.1.0-rc.3]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-rc.3
[0.1.0-rc.2]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-rc.2
[0.1.0-rc.1]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-rc.1
[0.1.0-beta.3]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-beta.3
[0.1.0-beta.2]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-beta.2
