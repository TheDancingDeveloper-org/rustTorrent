# rustTorrent v0.1.0-beta.2

## Summary

rustTorrent 0.1.0-beta.2 is a substantial usability and operability release.
It introduces a polished desktop-style management interface, richer tracker and
session visibility, alternative speed limits, first-class RSS automation, and
a realistic public demo. It also hardens authentication, container startup,
protocol handling, and the release pipeline itself.

This remains beta software. Back up the persistent session directory before
upgrading and avoid using it for data that cannot be downloaded again.

## Web UI and torrent management

- The interface now uses a compact, information-dense layout with a unified
  toolbar, status sidebar, sortable torrent table, responsive mobile cards,
  configurable columns, detail panes, and dark mode.
- Torrent rows expose progress, transfer rates, received and uploaded totals,
  ETA, peer counts, ratio, category, queue position, sequential mode, and
  availability where the backend provides them.
- Bulk pause, resume, delete, category, queue, speed-limit, seed-limit,
  sequential-download, and super-seeding actions are available from the main
  table and context menu.
- Per-torrent detail tabs cover overview data, files, peers, pieces, transfer
  history, and trackers without leaving the main screen.
- Drag-and-drop and multi-file torrent upload flows include file selection,
  categories, output folders, and paused/start controls.

## Tracker and peer visibility

- Trackers now maintain explicit working, error, and not-contacted states with
  seeders, leechers, returned peers, announce timestamps, intervals, and the
  most recent error.
- The sidebar can filter torrents by tracker, and the detail view presents the
  tracker state in a dedicated table.
- HTTP and UDP tracker communications have expanded parsing, status, scrape,
  IPv4/IPv6, warning, error, and malformed-response coverage.
- Deterministic mock tracker and browser tests exercise discovery and the
  torrent-management golden path without relying on public infrastructure.

## Speed controls and scheduling

- Alternative (turtle-mode) download and upload limits can temporarily replace
  normal session limits and restore them when disabled.
- A weekly schedule supports selected weekdays and overnight time windows.
- The footer exposes the alternative-speed toggle alongside live session
  download, upload, peer, and uptime statistics.
- Rate conversion now saturates safely instead of narrowing large values.

## RSS, categories, and storage controls

- RSS feeds, feed history, download rules, filtering, polling intervals,
  categories, and manual item downloads are managed from a dedicated page.
- Torrent categories and category-specific save paths can be created, assigned,
  filtered, and removed from the UI.
- Download and completed folders can be configured and browsed through the web
  interface.
- Session, RSS, DHT, and authentication state continue to live under the
  configured persistent data/cache mounts.

## Authentication and API

- First-run username/password setup, access and refresh tokens, logout, and
  persisted credential loading protect the HTTP API and web interface.
- The automation-compatible API has broader torrent, category, queue, limit,
  preference, and status coverage for automation clients.
- Swagger remains available for the native HTTP API once authenticated.

## Website and live demo

- The public website and demo are now maintained in the application monorepo
  and deployed through the same validated CI pipeline.
- The live demo is generated directly from the current React UI source rather
  than a separately maintained imitation.
- Demo data includes fictional releases, realistic progress and peer activity,
  tracker states, categories, and completed/pending RSS history.
- Public download links now lead to GitHub Releases.

## Reliability and packaging

- The web UI, Rust workspace, desktop workspace membership, and Docker build
  have been consolidated so the shipped interface is built from the reviewed
  source tree.
- Container s6 initialization and service scripts are executable again, so
  ownership and runtime configuration hooks complete cleanly on startup.
- Persistent DHT, session, RSS, and credential state have been exercised in a
  standalone pinned-SHA deployment on Node B.
- Fuzz targets and expanded unit, functional, tracker-swarm, and Playwright
  coverage protect bencode, DHT, peer protocol, storage, HTTP API, and UI paths.

## Source and release distribution

- GitHub is the source of truth for the complete `main` branch and release tag.
- Linux, Windows, Debian, checksum, and source artifacts are published on the
  canonical GitHub Release and the download host.
- Multi-architecture Docker images for `linux/amd64` and `linux/arm64` are
  published directly to public GHCR under the version, `beta`, and `latest` tags.
- GitHub Actions verifies the release assets, checksums, and container
  architectures before the release is accepted.

## Validation

- `cargo fmt --all -- --check`
- `cargo check --workspace --exclude rtbit-desktop --no-default-features --features default-tls`
- `cargo test --workspace --exclude rtbit-desktop --no-default-features --features default-tls`
- `cargo clippy --workspace --exclude rtbit-desktop --no-default-features --features default-tls -- -D warnings`
- Web UI unit tests, ESLint, TypeScript production build, and deterministic
  Playwright browser suite
- Linux x86_64, Windows x86_64, Debian amd64, and multi-architecture Docker
  release builds
- Standalone Node B container smoke test with persistent authentication and
  session state

## Breaking changes and upgrade notes

- No intentional destructive session or RSS database migration is included.
- Authentication is enabled after first-run setup. Existing configured users
  should sign in with their current credentials; new installations must finish
  setup before using protected endpoints.
- Keep `/home/rtbit/db` and `/home/rtbit/cache` persistent. The recommended
  container also persists download and completed directories.
- Review automation clients after upgrading because the beta API and UI remain
  subject to change before 1.0.
- `latest` and `beta` move to this prerelease after publication. Pin
  `v0.1.0-beta.2` when reproducibility matters.

## Downloads

- Linux x86_64: `rtbit-v0.1.0-beta.2-linux-x86_64`
- Windows x86_64: `rtbit-v0.1.0-beta.2-windows-x86_64.exe`
- Debian/Ubuntu amd64: `rtbit-v0.1.0-beta.2-amd64.deb`
- Checksums: `SHA256SUMS-v0.1.0-beta.2.txt`
- Docker: `ghcr.io/thedancingdeveloper-org/rusttorrent:v0.1.0-beta.2`
- Source: automatic `.zip` and `.tar.gz` archives on the GitHub release tag

All downloadable files are attached to the canonical GitHub release and are
also published at `https://dl.rusttorrent.dev/v0.1.0-beta.2/`.
