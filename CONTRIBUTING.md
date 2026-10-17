# Contributing to rustTorrent

Thanks for helping. rustTorrent is Apache-2.0 licensed; by contributing you agree
that your contribution is licensed under the same terms.

## Branches and releases

- Open pull requests against **`dev`**, the integration branch.
- `dev` is promoted to **`main`**. Pull requests into `main` are squash-merged,
  and `main` must be up to date with its required checks.
- A `v*` tag cut from `main` is a release. It publishes the binaries, `.deb`
  packages, checksums and the production container image. See
  [documentation/ReleaseProcess.md](documentation/ReleaseProcess.md).

## Building and testing

Install a recent stable Rust toolchain (CI builds with 1.95) and Node.js 24 for
the web UI.

```sh
# Rust workspace checks (matches CI)
cargo fmt --all -- --check
cargo check --workspace --exclude rtbit-desktop --no-default-features --features default-tls
cargo test --workspace --exclude rtbit-desktop --no-default-features --features default-tls
cargo clippy --workspace --exclude rtbit-desktop --no-default-features --features default-tls -- -D warnings

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
  `RELEASE_NOTES_<tag>.md` files.
- Add a `CHANGELOG.md` entry under **Unreleased** for any user-visible change.
- Architecture and design notes live under `docs/`.

## Code conventions

- Follow the rules in [CLAUDE.md](CLAUDE.md) for the repository, including the
  naming rules for the WebUI API v2 compatibility layer.
- Keep changes small and focused, and add tests for behaviour changes.

## Reporting security issues

Do not open a public issue for a vulnerability. Follow [SECURITY.md](SECURITY.md).
