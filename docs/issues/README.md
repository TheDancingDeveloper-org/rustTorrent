# rustTorrent Open-Issue Implementation Plans

This directory holds grounded implementation design documents for the open issues
tracked on Forgejo (`https://repo.indexarr.net/indexarr/rustTorrent/issues`).

Each document records:

- **Current state** — what already exists, with `file:line` references.
- **What's missing** — the concrete gap the issue describes.
- **Proposed implementation** — a phased plan against the real code.
- **Affected crates** — most logic lives in the shared `librtbit*` crates, not in
  this repo (`crates/rtbit/src/main.rs` is a thin binary wiring).
- **Testing, risks, effort.**

> **Architecture note.** rustTorrent's binary is `crates/rtbit` (≈1.2k lines of
> web-server / qBittorrent-API-v2 compat wiring). The engine, HTTP API, WebUI,
> DHT, and protocol code live in the workspace crates under `crates/librtbit*`,
> built from this repository. **`librtbit` is also consumed by StackArr**, so any lib change
> must follow the publish + version-bump workflow in the root `AGENTS.md`
> ("Lib Change Workflow") and be checked against the App→Lib dependency matrix.

## Issue Index

| # | Title | Primary crate(s) | Effort | Doc |
|---|---|---|---|---|
| 6 | SSRF hardening for server-side HTTP fetches | `librtbit` | M | [0006-ssrf-hardening.md](0006-ssrf-hardening.md) |
| 5 | Static API-key authentication | `librtbit` (http_api) | M | [0005-static-api-key-auth.md](0005-static-api-key-auth.md) |
| 9 | Run action/script on download/queue completion | `librtbit` (session) | M | [0009-completion-hook.md](0009-completion-hook.md) |
| 8 | WebUI: free disk space, availability bar, density | `librtbit` (webui + http_api) | M | [0008-webui-improvements.md](0008-webui-improvements.md) |
| 7 | Per-category share limits (ratio / seed-time) | `librtbit` | L | [0007-per-category-share-limits.md](0007-per-category-share-limits.md) |
| 11 | WebSeed (BEP 19) Phase 2: HTTP range download | `librtbit` | L | [0011-webseed-bep19.md](0011-webseed-bep19.md) |
| 12 | BEP 46 Phase 2: DHT mutable-item lookup | `librtbit-dht`, `librtbit` | L | [0012-bep46-mutable-magnets.md](0012-bep46-mutable-magnets.md) |
| 10 | BEP 52 Phase 2-5: full BitTorrent v2 support | `librtbit-core`, `librtbit` | XL | [0010-bep52-v2-support.md](0010-bep52-v2-support.md) |
| 2 | Dependency Dashboard (Renovate) | — (config) | — | [0002-dependency-dashboard.md](0002-dependency-dashboard.md) |

Effort key: **M** ≈ a focused day or two · **L** ≈ multi-day · **XL** ≈ multi-week.

## Suggested ordering

1. **#6 SSRF** first — it is foundational and explicitly must land *before*
   WebSeed (#11) and RSS download paths grow.
2. **#5 API key** and **#9 completion hook** — self-contained, high automation value.
3. **#8 WebUI** — mostly frontend; the availability bar is already built.
4. **#7 share limits** — note seed-ratio / seed-time enforcement does **not exist
   yet even globally**, so this is "build the feature, then make it per-category".
5. **#11 / #12 / #10** — protocol work, largest and best done one crate at a time.
