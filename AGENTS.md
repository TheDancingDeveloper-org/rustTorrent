# Codex Guidance

This directory already has Claude project guidance.

Before changing files here:

- Read the workspace-level `AGENTS.md` before changing files.
- Read `CLAUDE.md` in this directory and follow it as local project guidance.
- Do not edit `CLAUDE.md` unless the user explicitly asks for that.

## CI and landing

PR checks are path-scoped and the merge queue runs the full suite. Land by
enqueuing (`gh pr merge --auto --rebase`), not by rebasing after every sibling
merge. See `docs/ci.md`.

The required `policy` check (`ci/migration-policy.sh`) fails any commit whose
author, committer or message (trailers included) contains the legacy
identifier in its `forbidden` variable. The graft mirror copies the graft
author into a `Claimed-Author:` trailer, so set `GRAFT_AUTHOR_EMAIL` to the
GitHub noreply address, never a personal one.
