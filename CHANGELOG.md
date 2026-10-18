# Changelog

All notable changes to rustTorrent are recorded here. The full release notes for
each tag are in `docs/releases/<tag>.md`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions before 1.0 may include breaking changes between pre-releases; the
upgrade notes in each release describe what to back up and check.

## [Unreleased]

### Fixed

- WebUI API v2 compat: `torrentcreator/status` now emits `taskID` (was
  `taskId`), matching the upstream field name.

### Added

- WebUI API v2 compat conformance tests: every routed endpoint in
  `qbit_parity_spec.json` declares a response contract that is validated
  against a live torrent, plus an add/stop/start/recheck/setCategory/delete
  lifecycle test through the compat router.

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

[Unreleased]: https://github.com/TheDancingDeveloper-org/rustTorrent/compare/v0.1.0-rc.5...HEAD
[0.1.0-rc.5]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-rc.5
[0.1.0-rc.4]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-rc.4
[0.1.0-rc.3]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-rc.3
[0.1.0-rc.2]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-rc.2
[0.1.0-rc.1]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-rc.1
[0.1.0-beta.3]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-beta.3
[0.1.0-beta.2]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-beta.2
