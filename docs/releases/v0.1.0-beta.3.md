# rustTorrent v0.1.0-beta.3

## Summary

rustTorrent 0.1.0-beta.3 improves migration safety, torrent recovery, and
qBittorrent compatibility. It adds fail-closed read-only payload access,
restores tracker lists that are missing from imported metainfo, supports force
rechecks, and corrects save-path handling for multi-file torrents.

This remains beta software. Back up the persistent session directory before
upgrading and avoid using it for data that cannot be downloaded again.

## Torrent recovery and verification

- Tracker URLs can be added to existing torrents at runtime and are retained
  across JSON and PostgreSQL session reloads.
- Restored trackers start announcing immediately for live torrents. This
  recovers qBittorrent imports whose effective tracker lists existed only in
  `.fastresume` data rather than the exported `.torrent` file.
- Native and qBittorrent-compatible force-recheck endpoints verify payloads
  while preserving whether a torrent was paused or live.
- The Web UI exposes **Force Recheck** in the torrent context menu with
  appropriate disabled-state handling.

## Read-only payload migration

- `--storage-read-only` and `RTBIT_STORAGE_READ_ONLY` opt into filesystem
  storage that refuses payload writes, creation, resizing, deletion, and
  directory removal.
- Existing payloads can be verified without modification, and missing payload
  paths remain absent instead of being created implicitly.
- qBittorrent `savepath` values are now treated as payload roots for multi-file
  torrents, including the single-entry multi-file metainfo edge case.
- Corrected output-folder roots persist through both JSON and PostgreSQL
  session backends and are reflected by qBittorrent-compatible API responses.

## SwarmForge library family

- The reusable BitTorrent engine and protocol packages are now published as
  the coordinated `swarmforge` / `swarmforge-*` 0.1.0 family on crates.io.
- Cargo package identities changed, while compatible Rust library and import
  names such as `librtbit::Session` and `librtbit_core::Id20` remain available
  through dependency aliases.
- All 12 family members must be migrated atomically in a consumer dependency
  graph. Mixing historical and SwarmForge package identities can duplicate
  shared types.
- The historical repositories and packages remain available as rollback
  sources. NGMS independently verified consumption of the complete public
  SwarmForge family before this release.

## Web UI and deployment

- Production `/web/` routes now use the same-origin API instead of incorrectly
  redirecting requests to the Vite development port.
- Source, CI, binary releases, containers, and website delivery now use the
  canonical GitHub repository and GitHub Actions workflows.
- Main builds publish immutable commit images and release tags publish public
  multi-architecture `linux/amd64` and `linux/arm64` images to GHCR.

## Upgrade notes

- No intentional destructive session or RSS database migration is included.
- Keep `/home/rtbit/db` and `/home/rtbit/cache` persistent and back them up
  before upgrading.
- For qBittorrent migrations, use read-only storage for the first verification
  pass and confirm the payload root expected by each imported torrent.
- Consumers of the reusable libraries should follow the atomic migration and
  rollback guidance in `docs/SWARMFORGE-0.1.0-RELEASE.md`.
- `latest` and `beta` move to this release after publication. Pin
  `v0.1.0-beta.3` when reproducibility matters.

## Validation

- `cargo fmt --all -- --check`
- `cargo check --workspace --exclude rtbit-desktop --no-default-features --features default-tls`
- `cargo test --workspace --exclude rtbit-desktop --no-default-features --features default-tls`
- `cargo clippy --workspace --exclude rtbit-desktop --no-default-features --features default-tls -- -D warnings`
- Web UI unit tests, lint, production build, and Playwright browser tests
- Website structural validation
- GitHub release binary build and multi-architecture container publication

## Downloads

- Linux x86_64: `rtbit-v0.1.0-beta.3-linux-x86_64`
- Windows x86_64: `rtbit-v0.1.0-beta.3-windows-x86_64.exe`
- Debian/Ubuntu amd64: `rtbit-v0.1.0-beta.3-amd64.deb`
- Checksums: `SHA256SUMS-v0.1.0-beta.3.txt`
- Docker: `ghcr.io/thedancingdeveloper-org/rusttorrent:v0.1.0-beta.3`
- Source: automatic `.zip` and `.tar.gz` archives on the GitHub release tag

Release files are attached to the canonical GitHub release and are also
published at `https://dl.rusttorrent.dev/v0.1.0-beta.3/`.
