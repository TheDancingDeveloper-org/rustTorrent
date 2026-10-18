# rustTorrent

BitTorrent client written in Rust with a web UI.

## Repository & credentials (read before pushing or filing issues)

- **Source of truth is GitHub**: `origin` →
  `https://github.com/TheDancingDeveloper-org/rustTorrent.git`. Push code and
  file issues there.
- Do not inspect, fetch, compare, or push the historical Forgejo namesake
  unless the task explicitly concerns rollback or Forgejo deprecation. The
  workspace routing policy is `docs/GITHUB-MIGRATION-ROUTING.md`.
- **Credentials come from Infisical**, not from a logged-in `gh`/git session (there is none).
  This runtime has an injected machine identity (`INFISICAL_CLIENT_ID`/`INFISICAL_CLIENT_SECRET`);
  `mydevenv2-agent-auth check` confirms access. Do not declare yourself "blocked on auth" without
  fetching from Infisical first. Paths and token names are in the workspace-level `AGENTS.md`
  (Service Access table): `GIT_AUTH_TOKEN`/`FORGEJO_TOKEN`/`GITHUB_PAT` live in the `cicd` project,
  env `prod`. Never print token values.

## CI and landing

PR checks are path-scoped; the merge queue runs the full suite. Enqueue with
`gh pr merge --auto --rebase` rather than rebasing on every sibling merge. See
[docs/ci.md](docs/ci.md).

## Coding conventions

- **Do not reference the upstream qBittorrent project by name in code.** The
  WebUI API v2 compatibility layer must not spell the brand name in Rust
  identifiers, struct/field names, log messages, or response strings. Use the
  neutral `qbit` abbreviation (already the established prefix, e.g. `qbit_compat`,
  `QbitTorrentInfo`) or phrasings like "WebUI API v2 compat". Protocol-level
  constants the layer must emit for wire compatibility (state strings such as
  `stoppedDL`, the advertised `webapiVersion`, endpoint paths) are not brand
  references and are fine. Doc comments may mention the upstream project where
  needed to explain a compatibility decision, but keep it out of code proper.

## Codesight

Auto-generated codebase context map: `.codesight/CODESIGHT.md` — routes, schema, components, dependencies, and hot files. Regenerate with `npx codesight`.
