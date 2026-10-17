# rustTorrent v0.1.0-rc.1

## Summary

rustTorrent 0.1.0-rc.1 is the first release candidate for 0.1.0. It greatly
expands qBittorrent WebUI API v2 compatibility, folds in the security fixes and
dependency refresh accumulated since `0.1.0-beta.3`, and adopts the
`dev → main → tag` promotion pipeline for releases.

As a release candidate this is feature-oriented toward the 0.1.0 line and is
intended for validation. Back up the persistent session directory before
upgrading and avoid using it for data that cannot be downloaded again.

## qBittorrent WebUI API v2 compatibility

- Expanded WebUI API v2 parity from 19 to 64 endpoints, substantially widening
  the surface that existing qBittorrent clients and automation can drive
  unmodified.
- Added file, folder, and display renames, with a storage guard that refuses a
  rename that would clobber an existing destination.
- Added whole-torrent relocation via `setLocation` / `setSavePath`.
- Added the torrent-creator task API.
- Added `setShareLimits` with ratio and seeding-time limits stored and reported
  back through the compatible API.

## Security and dependency maintenance

- Replaced the unmaintained `parse_duration` crate with `humantime`.
- Bumped `quinn-proto` to a patched release and cleared the associated
  Dependabot advisory.
- Refreshed web UI dependencies and benchmarking-harness dependencies to
  patched versions.
- Adopted Dependabot version updates targeting the `dev` integration branch and
  merged the outstanding cargo, npm, and GitHub Actions bumps.

## Release pipeline

- Introduced the `dev → main → cut-release (v* tag) → prod` promotion flow.
- `dev` publishes the preview image `…:dev`; `main` publishes `…:staging`; a
  `v*` tag publishes the multi-arch prod image (`…:<tag>`, `…:latest`,
  `…:beta`) plus the binaries, `.deb`, and checksums.
- Every build additionally publishes an immutable `sha-<commit>` image tag.

## Upgrade notes

- No intentional destructive session or RSS database migration is included.
- Keep `/home/rtbit/db` and `/home/rtbit/cache` persistent and back them up
  before upgrading.
- For qBittorrent migrations, use read-only storage for the first verification
  pass and confirm the payload root expected by each imported torrent.
- `latest` and `beta` move to this release after publication. Pin
  `v0.1.0-rc.1` when reproducibility matters.

## Validation

- `cargo fmt --all -- --check`
- `cargo check --workspace --exclude rtbit-desktop --no-default-features --features default-tls`
- `cargo test --workspace --exclude rtbit-desktop --no-default-features --features default-tls`
- `cargo clippy --workspace --exclude rtbit-desktop --no-default-features --features default-tls -- -D warnings`
- Web UI unit tests, lint, production build, and Playwright browser tests
- Website structural validation
- GitHub release binary build and multi-architecture container publication

## Downloads

- Linux x86_64: `rtbit-v0.1.0-rc.1-linux-x86_64`
- Windows x86_64: `rtbit-v0.1.0-rc.1-windows-x86_64.exe`
- Debian/Ubuntu amd64: `rtbit-v0.1.0-rc.1-amd64.deb`
- Checksums: `SHA256SUMS-v0.1.0-rc.1.txt`
- Docker: `ghcr.io/thedancingdeveloper-org/rusttorrent:v0.1.0-rc.1`
- Source: automatic `.zip` and `.tar.gz` archives on the GitHub release tag

Release files are attached to the canonical GitHub release and are also
published at `https://dl.rusttorrent.dev/v0.1.0-rc.1/`.
