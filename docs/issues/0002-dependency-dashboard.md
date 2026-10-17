# Issue #2 — Dependency Dashboard (Renovate)

**Crate:** — (CI/config only) · **Effort:** ongoing maintenance, not a code task

## What this is

This is Renovate's auto-generated dashboard issue, not a feature. It lists detected
dependency updates (rate-limited and pending) and any repo problems Renovate hit.
It is maintained by the Renovate bot; you action items by ticking checkboxes (which
tells Renovate to open the corresponding update PR) or by merging the PRs Renovate
raises.

Config lives at `renovate.json` (repo root).

## Reported repo problems (from the dashboard body)

- ⚠️ `Rate limit exceeded for api.github.com, as no hostRules set for this host.
  Please set a GITHUB_COM_TOKEN` — Renovate is hitting GitHub's API unauthenticated.
  Fix: provide a `GITHUB_COM_TOKEN` to the Renovate runner (read-only PAT is
  enough; it is only used to look up release notes / GitHub-hosted deps). Store it
  with the other CI secrets per `AGENTS.md` (Infisical `cicd`) and inject it into
  the Renovate job — **do not** commit it.
- ⚠️ `No tool releases found.` — Renovate's tool-version datasource found nothing;
  usually harmless. Revisit only if a `mise`/`asdf`/tool-version manifest is meant
  to be tracked.

## Notable pending updates (triage, don't bulk-merge)

Several are **major** version bumps that need a human eye + green CI:

- Rust crates (major / risky): `axum-extra` 0.12, `rand` 0.10, `reqwest` 0.13,
  `sha1` 0.11, `rusqlite` 0.39, `nix` 0.31, `quick-xml` 0.39, `bollard` 0.20,
  `criterion` 0.8, `metrics-exporter-prometheus` 0.18, `rlimit` 0.11,
  `signal-hook` 0.4, `tokio` 1.51.1 (lockfile).
- Frontend (major): `vite` 8, `@vitejs/plugin-react` 6, `typescript` 6,
  `vite-plugin-svgr` 5, `eslint` 10 monorepo.
- Docker tags: `rust` 1.94, `python` 3.14, `linuxserver/baseimage-alpine` 3.23,
  `prom/prometheus` 2.55.1, `prom/node-exporter` 1.11.1.

## Recommended actions

1. Set `GITHUB_COM_TOKEN` for the Renovate runner to clear the rate-limit warning
   and restore release-note enrichment.
2. Triage majors individually — especially `reqwest` (touches the #6 SSRF work),
   `rand`, `sha1` (touches v1 piece hashing / #10), and `tokio` (lockfile). Each
   should ride CI to green before merge; check the App→Lib dependency matrix in
   `AGENTS.md` since several of these crates also live in the shared `librtbit*`
   and `nzb*` crates.
3. Routine non-major bumps (patch/minor) can be batched once CI is green.

No source change is required to "close" this issue; it stays open as Renovate's
living dashboard. Track the token fix and major-bump triage as the actual work.
