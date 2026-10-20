# Testing

rustTorrent uses three complementary test layers.

## Rust unit and integration tests

Run the workspace suite with:

```sh
cargo test --workspace --exclude rtbit-desktop --no-default-features --features default-tls
```

The `librtbit` suite includes a deterministic in-process BEP 15 UDP tracker. It
tests connect, announce, and scrape packets through the production tracker
client. A full swarm test then creates a torrent, announces a seed and leech to
that tracker, verifies peer discovery without preconfigured peers, transfers the
payload over the production peer protocol, and compares the downloaded bytes.
No public tracker or internet peer is required.

Authentication tests cover token issuance and rotation, refresh revocation,
credential validation, on-disk reload, malformed credential files, and private
credential-file permissions. `http_api/auth_middleware_tests.rs` runs a real
in-process server to check how the router applies them: the pre-setup lock,
read-only API keys and Basic users refused on writes, and unknown bearer
tokens refused. qBittorrent compatibility tests cover its special
category filter values in addition to numeric boundary behavior.

## Web UI unit and build checks

```sh
cd crates/librtbit/webui
npm ci
npm test
npm run lint
npm run build
```

## Release and packaging script checks

```sh
bash scripts/check-repo-hygiene.sh
bash scripts/test-publish.sh
bash packaging/scripts/test-render.sh
```

`test-render.sh` feeds the AUR, Homebrew, Flatpak and winget renderers a fake
checksum file. It checks that every placeholder is filled with the right
checksum and that each renderer fails when its artifact is missing. CI's
`scripts` job runs all three on every run.

## Browser GUI tests

Install Chromium once, then run the Playwright suite:

```sh
cd crates/librtbit/webui
npx playwright install chromium
npm run test:e2e
```

The suite starts the deterministic mock UI automatically and checks loading and
virtualization of 1,000 torrents, searching and status filters, selection and
pause actions, configuration, and the responsive mobile card layout. Use
`npm run test:e2e:headed` for an interactive browser.

Woodpecker runs all three layers on pushes, pull requests, tags, and manual
builds. Failed GUI runs retain a screenshot and Playwright trace in the job
workspace for diagnosis.

## WebUI API v2 compatibility: client replay and differential testing

Beyond the shape checks in `qbit_parity.rs`, two layers test the compat API the
way real clients use it. Both read the same fixtures in
`crates/librtbit/src/http_api/handlers/qbit_replay/`: recorded request
sequences from Sonarr's download-client proxy and the Python
`qbittorrent-api` library, each step with the client-visible outcome it
expects. `qbit_replay.rs` documents the fixture format.

**Client replay (per PR).** `qbit_replay.rs` replays every fixture in-process
against the compat router and asserts each step's expectations:

```sh
cargo test -p swarmforge --lib --features http-api qbit_replay
```

A divergence from upstream that is not fixed yet is recorded on the step as
`known_gap` (`{status, note}`). The replay asserts the current behaviour, so the
test fails when the gap is fixed until the entry is removed.

**Differential harness (nightly).** `scripts/qbit-diff/run.sh` starts a real
qBittorrent container (auth bypassed, DHT/LSD/PeX/queueing off) next to a
rustTorrent image, replays the fixtures against both in lockstep, and writes a
Markdown report. Status codes must match. Every JSON field the reference returns
must exist with the same type unless the parity spec lists it in that
endpoint's `missing_fields`. Fields named in a step's `diff.values` must be
equal. This is the only test that shows parity against upstream rather than
asserting it.

```sh
RTBIT_IMAGE=ghcr.io/thedancingdeveloper-org/rusttorrent:staging \
  scripts/qbit-diff/run.sh qbit-diff-report.md --report-only
```

**Sonarr end to end (nightly).** `tests/sonarr-e2e/run.sh` brings up Sonarr
and rustTorrent. Through Sonarr's API it configures rustTorrent as a
qBittorrent download client, runs Sonarr's own connection test, saves the
client, and checks Sonarr's health reports no download-client problem.

Both container checks run in `.github/workflows/compat-nightly.yml` (nightly
and on dispatch, against `:staging` by default). The diff job reports without
failing until the known gaps are closed. Dispatch it with `strict_diff` to gate
on a clean diff. Each driver runs in a container on the compose network, so it
also works where the docker daemon does not share the caller's network.
