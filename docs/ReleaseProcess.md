# rustTorrent Release Process

GitHub is the source of truth for source code, CI, releases, GHCR, and the
rusttorrent.dev website. The historical Forgejo/Woodpecker path is retired.

## Versioning

The current pre-release is `0.2.0-beta.4`.

- Crate version: `crates/rtbit/Cargo.toml`
- Release tag: `v0.2.0-beta.4`
- Docker tag: `ghcr.io/thedancingdeveloper-org/rusttorrent:v0.2.0-beta.4`

## Branching model

Promotion flow: **`dev` → `main` → cut release (`v*` tag) → prod.**

- **`dev`** is the integration branch. Feature/dependency PRs target `dev`.
  Pushing `dev` runs CI and publishes the preview image
  `ghcr.io/thedancingdeveloper-org/rusttorrent:dev` (+ immutable `sha-<commit>`).
- **`main`** is the promoted, pre-release branch. Promote `dev → main` by PR.
  Pushing `main` runs CI and publishes `…:staging` (+ `sha-<commit>`) and
  deploys the website. `main` is protected and its version stays at the current
  pre-release until a release is cut.
- **`v*` tag** (cut from `main`) is the release lever. A pre-release tag
  (`-alpha`/`-beta`/`-rc`) publishes the image as `…:<tag>` and `…:beta` and a
  GitHub pre-release. A stable tag publishes `…:<tag>` and `…:latest`. Both
  publish the binaries (+ `.deb`) to the download host and the GitHub release (+ `sha-<commit>`).
  The soak-then-promote flow is in [RELEASING.md](../RELEASING.md).

Nothing auto-deploys the running prod service from these workflows; publication
is to GHCR + the download host, and the deployment platform (Komodo) pulls the
tag it is configured for. A deployment that previously tracked `…:dev` for
"latest main" must repoint to `…:staging`, since `…:dev` now follows the `dev`
branch.

## CI Flow

Pushes and tags run the workflows under `.github/workflows/`.

The Rust workflow builds the workspace from this repository. The library crates
live under `crates/` and are built in place; no sibling checkouts are needed.

`dev` branch pushes publish the GHCR image as `dev`; `main` branch pushes
publish it as `staging`; both add an immutable `sha-<commit>` tag.

Release tags build:

- `rtbit-<tag>-linux-x86_64`
- `rtbit-<tag>-windows-x86_64.exe`
- `rtbit-<tag>-amd64.deb`
- `SHA256SUMS-<tag>.txt`
- Docker manifest for `linux/amd64` and `linux/arm64`

Tag releases publish artifacts to the download host and create the canonical
GitHub release. The container workflow publishes the multi-arch GHCR image.
Promotion of a soaked pre-release to stable is the manual `promote-release.yml`
workflow; see [RELEASING.md](../RELEASING.md).

Release notes are checked in as `docs/releases/<tag>.md`; the GitHub release
workflow uses that file verbatim and uploads the binaries and checksums.

GitHub marks a tag whose semantic version contains `alpha`, `beta`, or `rc` as a
pre-release, and any other tag as a stable release.
The Docker image must keep `org.opencontainers.image.source` set to
`https://github.com/TheDancingDeveloper-org/rustTorrent`. GitHub uses that OCI source
metadata to associate the GHCR container package with the public repository so
it appears on `https://github.com/TheDancingDeveloper-org/rustTorrent/packages`.

## Auth and Permission Notes

Use the workspace `docs/AUTH-VALIDATION.md` before touching Infisical, GitHub,
GHCR, or Komodo state.

Known permission failure modes:

- If the auth check reports a missing `INFISICAL_CLIENT_ID`, the running
  MyDevEnv2 container has not picked up the Komodo stack env values. Redeploy
  `prod-mydevenv2` from outside the active session; restarting the current
  container terminates the agent session.
- Binary releases use the workflow-scoped `GITHUB_TOKEN` with `contents: write`.
- Container publication uses the repository `GHCR_TOKEN` secret and requires
  `packages: write`. If GHCR login or publication fails, validate or rotate that
  repository secret with the destination-organization credential documented in
  the workspace guidance.
- Download-host publication uses the repository `DEPLOY_SSH_KEY` secret.
- Do not restore the retired Woodpecker release path or the revoked
  `GITHUB_PAT` / `GH_RELEASE_TOKEN` credentials.

## Historical v0.1.0-beta.1 Outcome

Woodpecker pipeline 71 published the Forgejo release and download artifacts on
2026-06-12. The initial GHCR and GitHub release stages failed because the
Woodpecker `gh_release_token` secret no longer had usable GitHub/GHCR access.
The token was rotated into Infisical `cicd/prod` as `GITHUB_PAT` and
`GH_RELEASE_TOKEN`, and the Woodpecker repo secret `gh_release_token` was
updated with `manual`, `push`, and `tag` event access.

- Forgejo release ID: `416`
- GitHub release ID: `338417179`
- Linux x86_64, Windows x86_64, Debian amd64, and SHA256 assets: published to
  Forgejo and GitHub
- Forgejo Docker image: published for `linux/amd64` and `linux/arm64`
- Historical GHCR image: `ghcr.io/ausagentsmith-org/rusttorrent:v0.1.0-beta.1`,
  `latest`, and `beta` published for `linux/amd64` and `linux/arm64`
- GitHub mirror: release tag `v0.1.0-beta.1` dereferences to the release-prep
  commit `6d040d0`; release follow-up documentation is on `main`
- GitHub issues `#11` through `#15`: closed with direct links to the `librtbit`
  implementation commit and the rustTorrent release-integration commit

This section records the retired Forgejo/Woodpecker release outcome only. The
credentials and repair procedure used for beta.1 must not be reused.

Do not print tokens, use `set -x`, or write credentials to repo files.
