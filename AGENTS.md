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
