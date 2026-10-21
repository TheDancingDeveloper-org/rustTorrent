# Contributing to rustTorrent

Thanks for helping. rustTorrent is Apache-2.0 licensed; by contributing you agree
that your contribution is licensed under the same terms.

## Branches and releases

- Branch from **`main`** and open the pull request against `main`. Branches are
  short-lived; there is no integration or long-lived release branch.
- A pull request lands through the merge queue, which runs the full suite on
  the exact tree `main` will become. Do not rebase a queued PR by hand.
- A `v*` tag cut from `main` is a release. It publishes the binaries, `.deb`
  packages, checksums and the production container image. See
  [docs/ReleaseProcess.md](docs/ReleaseProcess.md).

## Building and testing

Install a recent stable Rust toolchain (CI builds with 1.95) and Node.js 24 for
the web UI.

```sh
# Targeted Rust checks for the crates you touched (matches CI). CI runs the
# full workspace in the merge queue, so do not run it all before every push.
export CARGO_BUILD_JOBS=6
cargo fmt --all -- --check
cargo check -p <crate> --no-default-features --features default-tls
cargo clippy -p <crate> --no-default-features --features default-tls -- -D warnings
cargo nextest run -p <crate> --all-targets --locked --no-default-features --features default-tls

# Web UI
cd crates/librtbit/webui && npm ci && npm test && npm run lint && npm run build

# Repository hygiene (versions, licence files, documented paths and image tags)
make check-repo
```

[docs/TESTING.md](docs/TESTING.md) describes the three test layers, including the
Playwright browser suite, and the deterministic mock tracker used by the tests.

## Documentation

- User-facing documentation lives in `README.md`. Keep it accurate for the
  current release; release-specific detail belongs in `CHANGELOG.md` and the
  `docs/releases/<tag>.md` files.
- Add a `CHANGELOG.md` entry under **Unreleased** for any user-visible change.
- Architecture and design notes live under `docs/`.

## Code conventions

- Follow the rules in [CLAUDE.md](CLAUDE.md) for the repository, including the
  naming rules for the WebUI API v2 compatibility layer.
- Keep changes small and focused, and add tests for behaviour changes.

## How changes land

- **Required checks.** A pull request must pass five checks: `policy`, `rust`,
  `webui`, `gui-e2e` and `runner-policy / runner-policy`. They are small on
  purpose; the merge queue is where the full workspace runs.
- **Targeted checks on a pull request.** The `rust` check builds, lints and
  tests only the crates your diff touches, with a shared build cache and
  `cargo-nextest`. A change to the lockfile, the root `Cargo.toml`, the
  toolchain, `.cargo/`, `ci/`, `.github/` or an unmapped crate falls back to
  the full workspace. [docs/ci.md](docs/ci.md) has the path-to-crate mapping.
- **The queue proves the exact tree.** `merge_group` runs the full workspace,
  the web UI and its browser suite. A PR that only touches one crate should
  stay well inside the targeted-check budget.
- **Versions bump at release, not in feature PRs.** Every package shares the
  `crates/rtbit/Cargo.toml` version, which the release tag and the shipped
  binary use. Change it in the release PR or at tag time, not as part of a
  feature, and do not rebase to resolve version numbers. A release PR bumps
  versions only; it does not carry feature work.
- **Release changes get a dry run.**
  [.github/workflows/release-dry-run.yml](.github/workflows/release-dry-run.yml)
  builds the release artefacts without publishing, signing or creating a
  release, on PRs that touch release tooling and nightly on `main`. A release
  tag should not be the first time the packaging path runs.

## Reporting security issues

Do not open a public issue for a vulnerability. Follow [SECURITY.md](SECURITY.md).
