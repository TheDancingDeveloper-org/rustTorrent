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

Container-backed compat checks (the real-qBittorrent differential harness and
the Sonarr end-to-end smoke) run nightly in `compat-nightly.yml`, not per PR.
See [TESTING.md](TESTING.md#webui-api-v2-compatibility-client-replay-and-differential-testing).

### Path scoping on a pull request

`ci.yml`'s `classify changes` job diffs the PR against its merge base:

| Changed paths | Runs |
|---|---|
| `.github/`, `ci/`, `Dockerfile*`, `.dockerignore`, `rust-toolchain.toml` | everything, as in the queue |
| `crates/` (outside the web UI), `fuzz/`, `tests/`, `benchv2/`, `Cargo.*` | `rust` |
| `crates/librtbit/webui/`, `desktop/src/` (the splash, tested by `e2e/desktop-splash.spec.ts`) | `webui`, `gui-e2e` |
| `website/` | `website` |
| anything else (docs, design, the rest of desktop) | nothing; the skipped jobs report success |

The `scripts` job is not path-scoped: it runs `scripts/check-repo-hygiene.sh`,
`scripts/test-publish.sh` and `packaging/scripts/test-render.sh` on every run,
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

Queue runs use the same `self-hosted` label as PR runs. No workflow targets
GCP or spot labels.

## CodeQL runs on release builds only

`codeql.yml` (advanced setup, `rust` and `javascript-typescript`) never runs on
`pull_request`, `merge_group` or `push` to `main`. It runs for release builds
only, from three callers:

| Release build | How CodeQL is triggered |
|---|---|
| `v*` tag pushed by hand (pre-release or stable) | `push` tag trigger in `codeql.yml` |
| `:dev` image (`container.yml`, `dev` branch) | `codeql-dev` job calls `codeql.yml` with the commit SHA |
| Stable promotion (`promote-release.yml`) | `codeql` job calls `codeql.yml` with the stable tag |

The stable tag is created with the workflow token, which fires no push event,
so the promotion must call CodeQL explicitly. There is no `release:` trigger,
which would double-run the tag builds. `workflow_dispatch` is available for a
manual run, and the `ref` input chooses what to scan.

GitHub's default CodeQL setup is turned off for this repository, so
`codeql.yml` is the only code-scanning configuration. Results land in the
Security tab under the `/language:<lang>` categories. The workflow is not a
required check, because it does not run on pull requests.

## Landing

Enqueue with `gh pr merge --auto --rebase` (or the workspace `land-pr`, which
detects the queue and follows the entry's `merge_group` runs). Do not rebase a
PR by hand while it is queued.

## Turning the queue on (operator)

The workflows support a queue but do not require one. Until a ruleset with a
`merge_queue` rule exists on `main`, PRs merge as before, with the narrower PR
checks. The ruleset (`POST repos/TheDancingDeveloper-org/rustTorrent/rulesets`):

```json
{"name":"main merge queue","target":"branch","enforcement":"active",
 "conditions":{"ref_name":{"include":["refs/heads/main"],"exclude":[]}},
 "rules":[{"type":"merge_queue","parameters":{"merge_method":"REBASE",
   "max_entries_to_build":3,"min_entries_to_merge":1,"max_entries_to_merge":3,
   "min_entries_to_merge_wait_minutes":5,"grouping_strategy":"ALLGREEN",
   "check_response_timeout_minutes":60}}]}
```

Today only `policy` is a required check. The queue is worth having only if it
gates on the suite, so add `rust`, `webui`, `gui-e2e` and `runner-policy` to
the required checks. All of them post on `merge_group`.
