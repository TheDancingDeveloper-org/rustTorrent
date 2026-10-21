# CI: pull requests, the merge queue, and `main`

A pull request gets fast feedback on what it changed. The merge queue proves
that the exact tree `main` will become still passes everything. Without a
queue, every PR ran the whole suite and each merge forced sibling PRs to rebase
and run it all again.

## What runs where

| Event | `ci.yml` | `policy` / `runner-policy` |
|---|---|---|
| `pull_request` | only the areas the diff touches (below) | full |
| `merge_group` (the queue) | **everything** | full |
| `push` to `main`/`dev` | everything | full |

Only `pull_request` narrows. A `merge_group` entry, a push and a manual run all
take the full workspace (`rust_full=true`), and the five required contexts are
the same on every event.

Container-backed compat checks (the real-qBittorrent differential harness and
the Sonarr end-to-end smoke) run nightly in `compat-nightly.yml`, not per PR.
See [TESTING.md](TESTING.md#webui-api-v2-compatibility-client-replay-and-differential-testing).

### Path scoping on a pull request

`ci.yml`'s `classify changes` job diffs the PR against its merge base:

| Changed paths | Runs |
|---|---|
| `.github/`, `ci/`, `.cargo/`, `Dockerfile*`, `.dockerignore`, `rust-toolchain.toml` | everything, as in the queue |
| `crates/librtbit/` (outside the web UI) | `rust`, scoped to `swarmforge` |
| `crates/rtbit/` | `rust`, scoped to `rtbit` |
| `crates/librtbit/webui/`, `desktop/src/` (the splash, tested by `e2e/desktop-splash.spec.ts`) | `webui`, `gui-e2e` |
| `Cargo.toml`, `Cargo.lock`, `fuzz/`, `tests/`, `benchv2/`, any other `crates/*` path | `rust`, full workspace |
| `website/` | `website` |
| anything else (docs, design, the rest of desktop) | nothing; the skipped jobs report success |

The scope is deliberately conservative. `cargo` resolves `--features` against
the packages it selects, and `default-tls` exists only on `swarmforge`
(`crates/librtbit`) and `rtbit`. So only those two crates can be named in a
narrowed run; any other crate, a new member, a root-level file or a change
that alters how the workspace resolves (the lockfile, the root manifest, the
toolchain, cargo config, workflows and task scripts) takes the full workspace.
The scope is also widened when the classifier sees a Rust change it cannot
attribute to a package, so it never silently runs nothing.

### Cache and test runner

The `rust` job restores a shared `Swatinem/rust-cache` cache keyed on the
toolchain and the manifests, and runs `cargo nextest` instead of `cargo test`.
Only the queue and manual runs save the cache: a pull request is untrusted
input, and letting it write the shared key would let one PR evict or poison
what every other run restores. Both actions are pinned by commit SHA.

`cargo nextest` does not run doctests. Every doc fence in this workspace is
`text`, `ignore` or `no_run` with no assertions, so there is nothing for it to
skip today; if a doctest gains assertions, add a `cargo test --doc` step rather
than assume nextest covered it.

The `scripts` job is not path-scoped: it runs `scripts/check-repo-hygiene.sh`,
`scripts/test-publish.sh`, `packaging/scripts/test-render.sh` and
`scripts/test_import_qbit.py` on every run,
because they take seconds and guard files outside every scoped path.

`ci.yml` no longer uses `paths-ignore`. A required check that never starts
leaves a PR or queue entry waiting forever; a skipped job reports success.

### Concurrency

`ci`, `policy` and `runner-policy` cancel in progress per ref (`github.ref`),
so a force-push cancels the superseded run. `policy` also keys its group on
the event and ignores pushes to `gh-readonly-queue/**`, so a queue branch push
cannot cancel the required `merge_group` run. Each queue entry has its own
`gh-readonly-queue/main/pr-N-<sha>` ref, so entries never cancel each other.

### Runners

Queue runs use the same `self-hosted` label as PR runs. The `rust` job runs on
the local tailnet runners, `[self-hosted, Linux, X64, tailnet]`, the same
idle node-f runners the other jobs use. It used to target the GCP spot pool
(`gcp-rust`), which scales from zero; jobs waited up to 68 minutes for it, so
that label was retired on 2026-10-21. No workflow targets GCP or spot labels.
The runner-policy gate only requires a literal `self-hosted` and no dynamic
`runs-on`, so the new labels need no allowlist change.

## CodeQL runs on tagged releases and manual dispatch only

`codeql.yml` (advanced setup, `rust` and `javascript-typescript`) has no
`pull_request`, `merge_group` or branch `push` trigger. It runs for tagged
releases only, from two callers:

| Release build | How CodeQL is triggered |
|---|---|
| `v*` tag pushed by hand (pre-release or stable) | `push` tag trigger in `codeql.yml` |
| Stable promotion (`promote-release.yml`) | `codeql` job calls `codeql.yml` with the stable tag |

The stable tag is created with the workflow token, which fires no push event,
so the promotion must call CodeQL explicitly. There is no `release:` trigger,
which would double-run the tag builds. Dev builds and any other ad-hoc scans
are manual `workflow_dispatch` runs only: `container.yml` does not call
`codeql.yml`, and the `ref` input chooses what to scan.

A called workflow inherits the caller's ref, so `analyze` would otherwise
upload a stable-tag scan as `refs/heads/main`. `codeql.yml` therefore resolves
`inputs.ref` to a full `refs/...` ref (accepting `refs/...`, `heads/...`,
`tags/...`, or a branch or tag name; empty means `github.ref`) and passes the
checked-out commit (`git rev-parse HEAD`) as `sha`, so results are filed
against the scanned tag or branch.

GitHub's default CodeQL setup is turned off for this repository, so
`codeql.yml` is the only code-scanning configuration. Results land in the
Security tab under the `/language:<lang>` categories. The workflow is not a
required check, because it does not run on pull requests.

## Release dry run

`.github/workflows/release-dry-run.yml` runs the same build `release.yml`
runs on a tag, with no publishing: no deploy, no Authenticode signing, no
provenance attestation, no cosign signature, no GitHub release, and no release
secrets. It runs:

- on a pull request that touches `.github/workflows/release*.yml`,
  `packaging/**`, `scripts/publish.sh` or `systemd/**`;
- nightly at 03:17 UTC against `main`, so breakage no PR touched still
  surfaces before a release;
- on manual dispatch.

It derives the tag from the `crates/rtbit/Cargo.toml` version (the release
version `scripts/check-repo-hygiene.sh` enforces everywhere), builds the Linux
and Windows binaries, the `.deb`, the `.rpm`, the service-files tarball, the
vendored-crates tarball and the CycloneDX SBOM, checks the checksum manifest,
and uploads the set as a 7-day artifact. A release tag is therefore not the
first time the packaging path runs.

Neither the dry run nor `release.yml` sets a job-level `RUSTFLAGS`. Each
target's profile environment is exported inside its own build step, and
`.cargo/config.toml` scopes its rustflags to the `arm*` targets, so the
Windows cross build cannot leak flags into the host build (the failure mode
rustnzb #264 hit).

## Landing

Enqueue with `gh pr merge --auto --rebase` (or the workspace `land-pr`, which
detects the queue and follows the entry's `merge_group` runs). Do not rebase a
PR by hand while it is queued.

## The merge queue (operator)

The queue ruleset on `main` (`POST repos/TheDancingDeveloper-org/rustTorrent/rulesets`):

```json
{"name":"main merge queue","target":"branch","enforcement":"active",
 "conditions":{"ref_name":{"include":["refs/heads/main"],"exclude":[]}},
 "rules":[{"type":"merge_queue","parameters":{"merge_method":"REBASE",
   "max_entries_to_build":3,"min_entries_to_merge":1,"max_entries_to_merge":3,
   "min_entries_to_merge_wait_minutes":5,"grouping_strategy":"ALLGREEN",
   "check_response_timeout_minutes":60}}]}
```

All five required checks are live on `main` as branch protection contexts:
`policy`, `rust`, `webui`, `gui-e2e` and `runner-policy / runner-policy`.
Each of them posts on both `pull_request` and `merge_group`, so a queue entry
never waits for a context that will not start. If a required context is ever
renamed, rename it in branch protection in the same change, or the queue stalls.
