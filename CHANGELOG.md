# Changelog

All notable changes to rustTorrent are recorded here. The full release notes for
each tag remain in the `RELEASE_NOTES_<tag>.md` files at the repository root.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions before 1.0 may include breaking changes between pre-releases; the
upgrade notes in each release describe what to back up and check.

## [Unreleased]

### Changed

- Repository hygiene: added `LICENSE` full text, `NOTICE`, `CONTRIBUTING.md`,
  `SECURITY.md` and this changelog; corrected stale paths in `runme.md` and the
  `Makefile`; unified the desktop package version with the `rtbit` release
  version.

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

[Unreleased]: https://github.com/TheDancingDeveloper-org/rustTorrent/compare/v0.1.0-rc.1...HEAD
[0.1.0-rc.1]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-rc.1
[0.1.0-beta.3]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-beta.3
[0.1.0-beta.2]: https://github.com/TheDancingDeveloper-org/rustTorrent/releases/tag/v0.1.0-beta.2
