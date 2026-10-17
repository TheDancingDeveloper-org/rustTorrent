# rustTorrent polish review — 2026-10-17

Review-only pass over the daemon, HTTP API (native + WebUI API v2 compat), web
UI behaviour, desktop app, docs, packaging and release. Pure visual-design
findings are out of scope (covered by the separate GUI audit). Paths are
relative to the repo root; line numbers are approximate as of `origin/main`
at the time of review. Items marked *(verify)* were inferred from a code read
and should be confirmed before acting.

Abbreviations: `qbit` = `crates/librtbit/src/http_api/handlers/qbit_compat.rs`,
`main` = `crates/rtbit/src/main.rs`, `auth` = `crates/librtbit/src/http_api/auth.rs`,
`webui` = `crates/librtbit/webui/src`, `tauri` = `desktop/src-tauri`.

---

## 1. Quick wins (small, low-risk, visible)

| # | Area | Finding | Fix | Refs |
|---|------|---------|-----|------|
| Q1 | API compat | `app/version` returns `"v0.0.1"` while `webapiVersion` claims 2.11.3 — *arr apps gate features on the app version | Return a 5.x-consistent string | `qbit:559`, `qbit:563` |
| Q2 | API compat | `torrents/setForceStart` not routed → Sonarr/Radarr "Force Started" initial state 404s | Route to start handler | `qbit` router, `qbit_parity_spec.json` |
| Q3 | API compat | `app/preferences` lacks seeding-limit fields (`max_ratio_enabled`, `max_seeding_time*`, `max_ratio_act`) that *arr reads | Emit them as disabled | `qbit:584-602` |
| Q4 | API compat | Compat sessions expire 1h after login regardless of use; expired entries never pruned | Sliding expiry; prune in `create_session` | `qbit:44-75` |
| Q5 | Config | Corrupt `credentials.json` parses to `None` → auth silently disabled, setup reopens | Fail closed / refuse start with clear error | `auth:141-145` |
| Q6 | Config | Invalid `CORS_ALLOW_REGEXP` silently ignored; var lacks `RTBIT_` prefix | Warn/fail; add `RTBIT_CORS_ALLOW_REGEXP` alias | `crates/librtbit/src/http_api/mod.rs:181-183` |
| Q7 | Logging | `--log-file` default filter `librtbit=debug` fills disks | Default `info` | `main:82` |
| Q8 | Startup | Banner prints only the API bind address | Print web UI URL, auth state ("setup required at …"), data/listen paths | `main:~1098` |
| Q9 | Ops | No unauthenticated health endpoint; no Docker `HEALTHCHECK` | `/healthz` + `HEALTHCHECK` | `crates/librtbit/src/http_api/mod.rs`, `Dockerfile` |
| Q10 | Web UI | Pasting a magnet anywhere does nothing; dragged magnet/URL text ignored; unsupported dropped files give no feedback | Global paste handler; read `text/uri-list`; toast on reject | `webui/rtbit-web.tsx:~180-296` |
| Q11 | Web UI | Keyboard shortcuts exist but are undiscoverable | `?` overlay; shortcut hints in tooltips/context menu | `webui/hooks/useKeyboardShortcuts.ts:46-108` |
| Q12 | Web UI | Empty state is bare "No torrents"; same text when a filter hides everything | Actionable empty state + "No matches — clear filters" | `webui/components/TorrentTable.tsx:~396` |
| Q13 | Web UI | Backspace bound to delete *(verify always via modal)* | Delete key only; always open confirm modal | `useKeyboardShortcuts.ts:85` |
| Q14 | Docs | `runme.md` points at nonexistent `../libs/librtbit/webui` | Fix path or delete | `runme.md:22` |
| Q15 | Docs | Image tag inconsistent (`:beta` in README vs `:latest` in compose) | Pick one; document tag policy | `README.md:42`, `docker-compose.example.yml:3`, `docker-compose.local.yml:3` |
| Q16 | Docs | Parity doc counts stale ("46 missing" → 29; "111 missing" obsolete); "Remaining work" numbering broken by inserted sections | Refresh from spec counts | `docs/QBIT-API-PARITY.md` |
| Q17 | Packaging | Compose example hardcodes a personal `TZ`; Indexarr compose hardcodes a postgres password | `Etc/UTC`; `${POSTGRES_PASSWORD:?}` | `docker-compose.yml`, `docker-compose.indexarr.yml` |
| Q18 | Packaging | Desktop crate `authors` still names the upstream author with a personal email | Set to project maintainers | `desktop/src-tauri/Cargo.toml:6` |
| Q19 | Release | No `SECURITY.md` for a network-facing daemon | Add with private reporting channel | repo root |
| Q20 | Supply chain | Dependabot (targets `dev`) and Renovate (default branch, plus `website/renovate.json`) both configured → duplicate PRs | Keep one | `.github/dependabot.yml`, `renovate.json`, `website/renovate.json` |

## 2. High impact, larger effort

| # | Area | Finding | Fix | Refs |
|---|------|---------|-----|------|
| H1 | Security / first run | Before setup, every route is open and `/auth/setup` is claimable by whoever reaches it first; Docker binds `0.0.0.0:3030` and compose publishes it | Loopback-only until setup, or a one-time setup token printed to the log | `crates/librtbit/src/http_api/mod.rs:236-239`, `auth:230-275`, `Dockerfile` |
| H2 | Security | `/fs/browse` defaults to `/`, no root jail (open pre-setup per H1) | Restrict to configured roots | `crates/librtbit/src/http_api/handlers/configure.rs:98-113` |
| H3 | Security | Password stored in plaintext (0600) | argon2 hash + migration | `auth:127-176` |
| H4 | Security | No login rate limit/ban on native or compat login; compat cookie lacks `SameSite`, no Origin check | Per-IP backoff/ban; `SameSite=Strict`; Origin/Host check on POST | `qbit:523-544`, `qbit:540`, `auth` |
| H5 | API compat | `app/setPreferences` uses `deny_unknown_fields` and requires `announce_port` → read-modify-write clients get 400 | Optional fields, ignore unknown, 200 | `qbit:499-502`, `qbit:604-635` |
| H6 | Feature parity | No on-complete script/webhook (qBittorrent, Transmission, Deluge all have one) | `--on-complete-cmd` with env vars + optional webhook | `main`, `crates/librtbit` session events |
| H7 | Desktop | `.torrent` file association registered but launch args ignored; no single-instance → second instance fights for the port | `tauri-plugin-single-instance`, forward args to add flow | `tauri/tauri.conf.json:29`, `tauri/src/main.rs` |
| H8 | Desktop | No `magnet:` handler (desktop deep-link or browser `registerProtocolHandler`) | `tauri-plugin-deep-link`; web `registerProtocolHandler` | `tauri/tauri.conf.json:8`, `webui` |
| H9 | Desktop | Engine start failure → stderr + `process::exit(1)`; GUI users see nothing | Native error dialog with cause, log path, port hint | `tauri/src/main.rs:~512` |
| H10 | Web UI | No connection-lost detection; UI silently goes stale | Banner with backoff retry; keep last data visible | `webui/http-api.ts` |
| H11 | Web UI | Accessibility behaviour: ~2 `aria-*` attributes total; icon buttons unlabeled; table lacks grid semantics; modal focus trap *(verify)*; no `aria-live` | Labels, roles, focus management, live regions | `webui/components/IconButton.tsx`, `ActionBar.tsx`, `Modal.tsx`, `ProgressBar.tsx` |
| H12 | Supply chain | Release artifacts: SHA256SUMS only — no signature, SBOM or provenance; container build sets `provenance: false` | cosign/minisign, `attest-build-provenance`, SBOM, `provenance: mode=max` | `.github/workflows/release.yml:61`, `.github/workflows/container.yml:58` |
| H13 | Release infra | Release upload uses root SSH to a hard-coded private address with runtime `ssh-keyscan` (TOFU) | Pinned host key in secret, non-root deploy user, address in config | `.github/workflows/release.yml:72-76` |
| H14 | CI security | Actions pinned by tag not SHA; all jobs on self-hosted runners incl. publishing | SHA-pin; ensure fork PRs can't reach runners; ephemeral publish runners | `.github/workflows/*.yml` |

## 3. Medium

- **Graceful shutdown**: forced kill 5s after SIGTERM then 1s runtime shutdown may cut persistence flushes — make configurable, default ~30s, log pending work. `main:560-574` *(verify)*
- **Credential reset**: `RTBIT_HTTP_BASIC_AUTH_USERPASS` ignored when stored creds exist — add `rtbit auth reset` or document precedence. `main:723-735`
- **Indexarr env vars bypass clap** (absent from `--help`, unvalidated, no warning on URL-without-ENABLED). `main:763-773`
- **Log rotation & format**: no rotation for `--log-file`; no `--log-format json`. `crates/rtbit/src/tracing_subscriber_config_utils.rs`
- **systemd unit**: relative `ExecStart=rtbit`, no `Restart=on-failure`, no `TimeoutStopSec`, no hardening; `rtbit.conf` missing many vars. `systemd/rtbit.service`, `systemd/rtbit.conf`
- **No config file** (env/flags only) — optional TOML `--config` and `rtbit config dump`. `main:68`
- **RSS DB failure** only a warning; surface in a status endpoint. `main:~1077`
- **Socket-activation failure** logs and continues — make fatal or explain fallback. `main:~1084`
- **Missing parity features vs qBittorrent/Transmission/Deluge**: queueing (max active downloads/seeds), disk-space precheck/low-disk pause, share-ratio/seed-time enforcement (compat `setShareLimits` stored in memory, not enforced), `.part` incomplete extension, HTTP proxy / proxy-trackers-only (SOCKS5 only, `main:242-245`), local-file IP filter + refresh (`main:282-288`).
- **Compat tags in memory only** — lost on restart; *arr uses them. `qbit:37-40`
- **Compat queue-priority endpoints missing** — safe while `queueing_enabled: false` (`qbit:600`); add no-op stubs.
- **Compat error messages inconsistent** (case, wording, serde text leaked). `qbit:1096`, `qbit:1697`, `qbit:610-624`
- **`sync/maindata`** always full snapshot; ensure `full_update: true`, deltas later.
- **Desktop tray**: close likely quits instead of hiding *(verify)*; "Pause All" keeps its own flag that drifts from real state, label never flips, errors dropped; no Show/Add/Settings items; left-click-only show fails on Linux AppIndicator. `tauri/src/main.rs:303-357`
- **Desktop**: no autostart, no updater/new-version check.
- **Web UI preferences**: only column layout persisted; sort, filters, sidebar, detail tab/size lost on reload. `webui/stores/columnStore.ts:291-397`, `uiStore`
- **Web UI shortcuts missing**: Space (pause/resume), Enter (details), Ctrl+O, add-magnet; shift/ctrl range select *(verify)*.
- **Settings**: 11 tabs, no search, no restart-required markers, no unsaved-changes guard. `webui/components/config/*Tab.tsx`
- **Session expiry UX**: tokens in `localStorage`; no "session expired" message / return-to *(verify)*. `webui/stores/authStore.ts:32-52`, `LoginPage.tsx`
- **Context menu** not keyboard-reachable. `webui/components/TorrentContextMenu.tsx`
- **Versions disagree**: `rtbit` 0.1.0-rc.1, `swarmforge-*` 0.1.0, desktop 0.0.1. Use `version.workspace = true` and sync desktop. `crates/rtbit/Cargo.toml:3`, `desktop/package.json:4`, `desktop/src-tauri/Cargo.toml:4`
- **Licensing/attribution**: `LICENSE` Apache-2.0 only, `docs/FORK_PROVENANCE.md` says Apache-2.0/MIT; no `NOTICE`, no fork copyright.
- **No CHANGELOG / CONTRIBUTING**: three root `RELEASE_NOTES_*.md`; contributor docs are agent-oriented.
- **Two Dockerfiles** (root vs `docker/`); root uses unpinned `rust:alpine`. `Dockerfile:4`
- **README**: state that arm64 images are tag/release only (`container.yml:56`); document volume purposes; compose should warn the port is LAN-published and show a `127.0.0.1:` binding and auth env.

## 4. Low

- Native API lacks a versioning policy; Swagger covers only native API — link the parity doc. `crates/librtbit/src/http_api/mod.rs:30-70`
- Compat accepts cookie only (no Basic) — optional. `crates/librtbit/src/http_api/mod.rs:298-303`
- Compat 403 text "Not authenticated" vs upstream "Forbidden"; `torrents/add` 500 vs 415 on invalid file. `qbit:2791`, `qbit:1567`
- `-v` help doesn't list levels or mention `RUST_LOG`; port 4240 default not shown in `--help` (`main:72`, `main:780-785`); `umask` error doesn't echo value (`main:58-62`).
- `--http-api-allow-create` off by default → UI "create torrent" fails opaquely; return a clear error.
- Native tokens in-memory only → restart logs everyone out; document or persist refresh tokens. `auth:9-120`
- Single user, no read-only role / API key for automation.
- Toast system absent (only blocking `AlertModal`) — use toasts for non-decisions.
- Delete modal: list names/count for bulk deletes.
- Tray quit doesn't confirm while transfers are active.
- Search text not reflected in URL.
- Settings shortcut (Ctrl+,) not hinted on the gear button.
- Mobile drawer backdrop: no Esc / focus return. `webui/rtbit-web.tsx:~260`
- Desktop bundle `targets: "all"` — verify Linux `.desktop` MimeType for torrent/magnet and Windows signing.
- Distribution: only static binaries + GHCR; no deb/rpm/brew/winget/AUR/flatpak or macOS builds (cargo-dist/nfpm later).
- Root clutter: `bench/` + `benchv2/`, `root/etc` overlay, four compose files, `runme.md`, release notes; `docs/` vs `documentation/`; mixed naming (rtbit / rustTorrent / swarmforge / librtbit) — consolidate and add a naming note.

## 5. Already good

- Graceful SIGINT/SIGTERM with second-signal force exit (`main:548-565`); auto-raised open-file limit (`main:612-614`).
- Constant-time credential compare, 0600 credentials file, strict default CORS allowlist (`auth:170-184`, `crates/librtbit/src/http_api/mod.rs:171-194`).
- Broad `RTBIT_*` env mapping via clap.
- Feature breadth: watch folder, completed folder, categories, tags, alt-speed + scheduler, RSS feeds/rules, session persistence (JSON/postgres), fastresume, Prometheus metrics.
- Compat layer: auth covers all sub-routers; `spec_matches_router` CI guard; correct `torrents/info` paging and 2.11 filter aliases (`qbit:812-816`, `qbit:972-985`); parity doc per-controller counts accurate.
- Web UI: first-run setup page; drag-drop overlay with file-selection step; delete confirmation with unticked "delete files"; status/category/tracker filters, column menu, context menu, bulk action bar; good shortcut base.
- Desktop: live-speed tray tooltip; native completion/failure notifications.
- Release: SHA256SUMS, GHCR retention workflow, CODEOWNERS, grouped dependency updates; README quickstart (binary, subcommand, ports) matches code.

---

## Suggested order of work

1. **Close the first-run exposure** (H1, H2, Q5) — the only items that can hurt users today.
2. **\*arr compatibility quick fixes** (Q1–Q4, H5) — cheap and they unblock the main integration use case.
3. **Credential hardening** (H3, H4).
4. **Ops quick wins** (Q6–Q9, Q17) plus systemd/log rotation.
5. **Web UI behaviour quick wins** (Q10–Q13), then the connection-lost banner (H10).
6. **Docs/repo hygiene** (Q14–Q16, Q18–Q20, licence/NOTICE, CHANGELOG, version unification).
7. **Supply chain** (H12–H14).
8. **Desktop OS integration** (H7–H9, tray, autostart, updater).
9. **Feature parity** (H6 on-complete hook, queueing, share limits, disk-space guard), then accessibility (H11) and the remaining medium/low items.

## Proposed work packages (parallel-safe)

Each package owns a distinct file set so workers don't collide.

| WP | Title | Scope | Owns |
|----|-------|-------|------|
| WP1 | Native auth & first-run hardening | H1, H2, H3, Q5, native half of H4, credential reset command | `crates/librtbit/src/http_api/auth.rs`, `http_api/mod.rs` (auth middleware), `handlers/configure.rs` |
| WP2 | WebUI API v2 compat fixes | Q1–Q4, H5, compat half of H4, tags persistence, error helper, queue stubs, parity-spec + `docs/QBIT-API-PARITY.md` refresh (Q16) | `handlers/qbit_compat.rs`, `qbit_parity_spec.json`, `docs/QBIT-API-PARITY.md` |
| WP3 | Daemon CLI, startup & logging | Q6–Q8, Indexarr clap args, log rotation/JSON, shutdown timeout, help-text items, optional config file | `crates/rtbit/src/main.rs`, `crates/rtbit/src/tracing_subscriber_config_utils.rs` |
| WP4 | Health, container & service packaging | Q9 (`/healthz` route as a new handler file + Dockerfile HEALTHCHECK), Q17, systemd unit/conf, Dockerfile consolidation/pinning, compose warnings | `Dockerfile`, `docker/`, `docker-compose*.yml`, `systemd/` (+ new health handler file; coordinate route registration with WP1) |
| WP5 | Engine feature parity | H6 on-complete hook/webhook, queueing limits, share-ratio enforcement, disk-space guard, `.part` extension | `crates/librtbit/src/` session/torrent modules (not `http_api/`) |
| WP6 | Web UI add-flow & feedback | Q10, Q12, H10, toast system, magnet `registerProtocolHandler`, session-expired UX | `webui/rtbit-web.tsx`, `webui/http-api.ts`, `webui/stores/authStore.ts`, new toast component |
| WP7 | Web UI keyboard, prefs & settings | Q11, Q13, missing shortcuts, persisted view prefs, settings search/restart markers, context-menu keyboard access, H11 accessibility | `webui/hooks/`, `webui/stores/columnStore.ts`, `uiStore`, `webui/components/` (excluding files in WP6) |
| WP8 | Desktop OS integration | H7–H9, tray fixes, close-to-tray, autostart, updater, bundle MimeType/signing | `desktop/` |
| WP9 | Release & supply chain | H12–H14, Q20, SHA-pinning, runner policy | `.github/workflows/`, `.github/dependabot.yml`, `renovate.json` |
| WP10 | Docs, licensing & repo hygiene | Q14, Q15, Q18, Q19, NOTICE/licence, CHANGELOG, CONTRIBUTING, version unification, root clutter, naming note, README arm64/volumes | `README.md`, `docs/` (except parity doc), `documentation/`, root files, `Cargo.toml` versions |

Coordination notes: WP1 and WP4 both touch route registration in
`http_api/mod.rs` — let WP1 own that file and have WP4 hand it a one-line
route. WP10's version unification touches `desktop/package.json`; sequence it
after WP8 or limit it to the version field.
