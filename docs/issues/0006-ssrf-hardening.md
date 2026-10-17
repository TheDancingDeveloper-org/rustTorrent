# Issue #6 — SSRF hardening for server-side HTTP fetches

**Crate:** `librtbit` · **Effort:** M · **Priority:** do first (blocks #11, RSS download)

## Problem

The server fetches URLs that originate from untrusted input (torrent `url-list`,
RSS feeds, IP-blocklist URLs, magnets, indexarr proxy). Today every `reqwest`
client follows redirects with the default policy (up to 20 hops) and performs no
validation of the resolved destination IP or scheme. An attacker-supplied URL —
or a redirect from one — can therefore reach loopback, RFC1918, link-local, or
cloud-metadata endpoints (`169.254.169.254`). Mirrors qBittorrent 5.2.1
"prevent SSRF via HTTP redirection".

## Current state (file:line)

All clients follow redirects by default; none validate destination IP:

| Fetch site | Location | Notes |
|---|---|---|
| RSS feed body | `librtbit/src/rss/monitor.rs:35-38`, `:90-101` | builder sets only `timeout(30s)`; accepts `http`/`https`/**`file://`** |
| Torrent-from-URL | `librtbit/src/session/helpers.rs:14-43` (`torrent_from_url`, GET at `:30-34`) | uses session `reqwest_client`; called from `session/mod.rs:657-662` |
| IP-blocklist download | `librtbit/src/ip_ranges.rs:61-82` (`load_from_url`, `reqwest::get` at `:72`) | accepts `file://` or HTTP(S) |
| Indexarr proxy | `librtbit/src/http_api/handlers/indexarr.rs:37-63` (`proxy_get`), `:66-92` (`proxy_post_json`), `:107-114` | `base_url` from `RTBIT_INDEXARR_URL`; forwarded requests still follow redirects |
| Session global client | `librtbit/src/session/mod.rs:325-341` | supports SOCKS5 proxy + bind-device; no redirect/IP policy |
| Torrent POST endpoint | `librtbit/src/http_api/handlers/torrents.rs:55-96` (`h_torrents_post`, `is_url=true`) | entry point for user-supplied URL |

`ip_ranges.rs` already has an interval-tree `IpRanges` with a `has()` lookup
(`:30-145`, `:139-144`) but **no built-in private/reserved-range table** — that
must be added.

## Proposed implementation

### Phase 1 — a shared SSRF-safe fetch policy

Add `librtbit/src/ssrf.rs` exposing:

- `fn is_forbidden_ip(ip: IpAddr) -> bool` — true for loopback, private (RFC1918),
  link-local (incl. `169.254.0.0/16` / `fe80::/10`), unique-local (`fc00::/7`),
  CGNAT (`100.64.0.0/10`), broadcast/multicast/unspecified, and IPv4-mapped IPv6
  forms of the above. Use `std::net::Ipv4Addr`/`Ipv6Addr` classification helpers
  plus explicit ranges for what std doesn't cover; unit-test each range.
- `fn validate_scheme(url: &Url) -> Result<()>` — allow only `http`/`https`
  (note: `file://` must be gated behind an explicit opt-in, see Phase 3).
- A constructor `build_safe_client(cfg) -> reqwest::Client` that sets
  `.redirect(reqwest::redirect::Policy::custom(...))` so each hop is validated:
  cap the chain (e.g. 5), reject scheme changes to non-HTTP, and reject hops
  whose host resolves to a forbidden IP.

Because `reqwest`'s redirect closure only sees the URL (not the resolved IP), pair
it with a resolver guard: install a custom DNS resolver
(`reqwest::ClientBuilder::dns_resolver`) that rejects forbidden IPs at resolution
time — this closes the TOCTOU / DNS-rebinding gap that a URL-only check leaves
open. This is the key design point.

### Phase 2 — adopt the safe client everywhere

Route every fetch site above through `build_safe_client` (or validate before the
existing session client is used):

- `rss/monitor.rs` — replace ad-hoc builder.
- `session/helpers.rs::torrent_from_url` — use safe client (or a safe variant of
  the session client) for user URLs.
- `ip_ranges.rs::load_from_url` — replace `reqwest::get`.
- `http_api/handlers/indexarr.rs` — `RTBIT_INDEXARR_URL` is operator-configured,
  so validate it **once at startup**, not per request; keep proxy fast.

### Phase 3 — scheme / opt-in policy

`file://` is currently accepted in several places (RSS, blocklist, torrent URL).
Decide per call site whether local-file loading is intended (blocklist: probably
yes for operators; RSS/torrent-from-URL: no). Gate `file://` behind an explicit
config flag, default off for any path reachable from the network API.

## Testing

- Unit tests for `is_forbidden_ip` across every range (v4, v6, mapped).
- Redirect tests using a local mock server (e.g. `wiremock`) that 302-redirects to
  `http://127.0.0.1:.../` and to `http://169.254.169.254/` — assert both rejected.
- DNS-rebinding test: resolver returns a forbidden IP → request rejected.
- Regression: a normal public URL still fetches.

## Risks / notes

- Don't break the SOCKS5 proxy / bind-device path in `session/mod.rs:325-341`;
  when an outbound proxy is configured the IP guard may need to defer to the proxy
  (document the trade-off — egress through a trusted proxy is the operator's call).
- Cloud-metadata `169.254.169.254` must be explicitly covered.
- Coordinate with **StackArr** before publishing (shared crate).
