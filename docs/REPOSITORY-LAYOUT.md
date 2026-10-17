# Repository layout

This page explains what each top-level path is for, and why some files that look
similar are kept separate. Architecture notes live in the other files under
`docs/`; the user-facing overview is in the [README](../README.md).

## Crates and packages

| Path | Contents |
|------|----------|
| `crates/rtbit` | The `rtbit` command-line program and server entry point (the default workspace member). |
| `crates/librtbit` | The core engine: sessions, torrent state, storage, HTTP API and the web UI (`crates/librtbit/webui`). Published as `swarmforge`. |
| `crates/librtbit-*` | Protocol building blocks: bencode, buffers, clone-to-owned, core, DHT, LSD, peer protocol, SHA-1 wrapper, tracker communication, UPnP and UPnP serving. Published as `swarmforge-*`. |
| `desktop/` | npm workspace and Tauri application (`desktop/src-tauri`). Excluded from the CI and Docker builds because it needs glib. |
| `fuzz/` | `cargo-fuzz` targets for the protocol parsers. Outside the workspace; needs a nightly toolchain. |
| `benchv2/` | Rust-native benchmark harness comparing rtbit with another client. Outside the workspace, with its own `Cargo.toml`, `Dockerfile` and `docker-compose.yml`. |

All crates and packages share the version in the release notes, currently
`0.1.0-rc.1`. `scripts/check-repo-hygiene.sh` (run with `make check-repo`) fails
if a manifest drifts from it.

## Container images and compose files

| Path | Purpose |
|------|---------|
| `Dockerfile` | Builds the published image from source, as a multi-stage build with the web UI compiled in. CI (`.github/workflows/container.yml`) builds this file. |
| `docker/Dockerfile` | A minimal `scratch` image that packages binaries built elsewhere, for example by `docker/Dockerfile.xx`. |
| `docker/Dockerfile.xx` | Cross-compiles `rtbit` for several platforms inside Docker, using `tonistiigi/xx`. |
| `docker/compose-examples/server.yaml` | Compose example for the `docker/` image. |
| `docker-compose.yml` | Builds the image from this checkout (`docker compose up --build`). Used for local development from source. |
| `docker-compose.example.yml` | Template for a self-hosted deployment. It pulls the published image and uses host paths for downloads, with PUID, PGID and TZ settings. Copy it and edit the paths. |
| `docker-compose.local.yml` | Runs the published image with named volumes and no host paths. Useful for a quick trial on one machine. |
| `docker-compose.indexarr.yml` | Combined stack: rustTorrent, the Indexarr search service and PostgreSQL. See [IndexarrRustTorrentIntegration.md](IndexarrRustTorrentIntegration.md). |
| `root/etc/s6-overlay/` | Service scripts that the root `Dockerfile` copies into the image (`COPY root/ /`) for s6 process supervision. |

The compose files are kept separate because each one answers a different question:
build from source, copy as a template, try quickly, or run the Indexarr stack.
Merging them would mean choosing one set of defaults for all four cases.

## Benchmarks

| Path | Purpose |
|------|---------|
| `bench/` | Original benchmark harness: a Python seeder, mock seeder and tracker, orchestrator, Prometheus config and a `run.sh` driver, all in Docker. |
| `benchv2/` | Rust-native rewrite of the harness (see the table above). |

Both are kept because they are separate tools with separate dependencies, and
neither is run by CI. Retiring `bench/` once `benchv2/` covers its scenarios is
an open decision, not a settled one.

## Documentation and release material

| Path | Contents |
|------|----------|
| `README.md` | User overview, quick start and attribution. |
| `CHANGELOG.md` | Summary of changes by release. |
| `docs/releases/` | Full release notes for each tag, one file per tag. The release workflow publishes `docs/releases/<tag>.md` as the GitHub release body. |
| `docs/ReleaseProcess.md` | How releases are cut and published. |
| `docs/TESTING.md` | The test layers and how to run them. |
| `docs/` | Architecture and design notes, provenance and audits. |
| `docs/issues/` | Numbered design notes for planned features. |
| `CONTRIBUTING.md`, `SECURITY.md` | Contributor guidance and vulnerability reporting. |
| `LICENSE`, `NOTICE` | Apache-2.0 licence and upstream attribution. |

## Build, packaging and CI

| Path | Purpose |
|------|---------|
| `Cargo.toml`, `Cargo.lock` | The Cargo workspace. |
| `package.json`, `package-lock.json` | The npm workspace for `desktop/` and the repository's formatting tool. |
| `Makefile`, `runme.md` | Developer shortcuts and a quick start for running the app locally. |
| `.github/workflows/` | CI, container publication, release and website workflows. |
| `ci/` | Migration policy checks and website tasks. |
| `scripts/` | Release, publishing and maintenance scripts, including `check-repo-hygiene.sh`. |
| `systemd/` | Unit and socket files for running `rtbit` as a system service. |
| `website/` | Static site for rusttorrent.dev, its live demo and deployment files. |
| `tests/sonarr-e2e/` | Docker Compose setup for an end-to-end run with Sonarr against rustTorrent. |
| `design/`, `LOGOS/`, `resources/` | Design audits, logo assets and a macOS entitlements file for debugging. |
