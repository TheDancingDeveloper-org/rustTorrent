# Issue #5 — Static API-key authentication

**Crate:** `librtbit` (http_api) + `crates/rtbit` wiring · **Effort:** M

## Problem

The web API authenticates via Basic auth and short-lived bearer tokens
(15 min access / 30 day refresh). Those suit interactive logins but are friction
for *arr apps and automation, which want a single long-lived, non-expiring secret.
Add long-lived, **revocable**, **scoped** (read / write) API keys alongside the
existing schemes. Stored **hashed**. Mirrors qBittorrent 5.2.0.

## Current state (file:line)

- Auth core: `librtbit/src/http_api/auth.rs`
  - Token TTLs `:9-10`; `TokenStore::validate_access_token` `:85-90`.
  - `CredentialStore` `:127-189` — username/password persisted to
    `credentials.json` (`:140`), file mode `0o600` (`:174`), `validate()`
    constant-time compare `:180-188`.
- Middleware / extraction: `librtbit/src/http_api/mod.rs`
  - Route-layer auth check `:209-291`; bearer extraction+validate `:237-248`;
    Basic extraction+validate `:250-285`; `constant_time_eq` `:100-108`.
  - Public endpoints bypass auth `:215-223`; "no creds configured ⇒ setup mode,
    allow all" `:225-235`.
- Auth config: `HttpApiOptions` `librtbit/src/http_api/mod.rs:79-97`
  (`basic_auth` `:82`, `token_store` `:85`, `credential_store` `:86`).
- Wiring: `crates/rtbit/src/main.rs:704-725` builds the credential store and
  env-var fallback `RTBIT_HTTP_BASIC_AUTH_USERPASS`.

## Proposed implementation

### Phase 1 — `ApiKeyStore` (auth.rs)

Model on `CredentialStore`. Persist to `api_keys.json` (mode `0o600`) next to
`credentials.json`.

```text
ApiKey {
  id: String,            // public, for display/revoke (e.g. "ak_<rand8>")
  name: String,          // human label ("sonarr")
  hash: String,          // Argon2/bcrypt OR HMAC-SHA256 of the secret; never store plaintext
  scope: Scope,          // Read | ReadWrite
  created_at, last_used_at: Option<…>,
  revoked: bool,
}
ApiKeyStore { keys: RwLock<Vec<ApiKey>> }  // create / list / revoke / validate(secret) -> Option<Scope>
```

Key format: emit `ak_<id>_<secret>` once at creation; store only the hash. On
validate, look up by `id` then verify `secret` against `hash` in constant time
(reuse `constant_time_eq` style). Prefer a real password hash (argon2 is already
a likely dep tree member) over plain SHA for the secret-at-rest.

### Phase 2 — middleware (mod.rs)

- Add `api_key_store: Option<Arc<ApiKeyStore>>` to `HttpApiOptions` (`:79-97`).
- In the auth check (`:209-291`), after bearer (`:248`) and before/after Basic,
  read the `X-Api-Key` header (qBittorrent-compatible header name); if present,
  `validate()` → on success attach the resolved `Scope` to request extensions.
- Enforce scope: a **read** key must be rejected on mutating routes. Add a small
  route classifier (write = POST/PUT/DELETE on torrent/config mutation paths) or
  tag routes; reject `Read` scope on write routes with 403.

### Phase 3 — management surface

- Endpoints to create / list / revoke keys (admin only — require an existing
  authenticated session, never an API key creating keys unless ReadWrite+explicit).
- Wire `ApiKeyStore::new(config_dir)` in `crates/rtbit/src/main.rs:704-725`
  alongside the credential store; add an env-var seed
  (`RTBIT_HTTP_API_KEY`) for headless bootstrapping.
- Surface create/list/revoke in the WebUI settings (see #8 patterns).

## Testing

- `ApiKeyStore`: create→validate roundtrip; revoked key fails; wrong secret fails;
  scope returned correctly; hash-at-rest (no plaintext in `api_keys.json`).
- Middleware: valid write key on write route ✓; read key on write route → 403;
  unknown key → 401; key + valid Basic both work independently.
- Persistence file mode `0o600`.

## Risks / notes

- Keep the "setup mode allows all" branch (`mod.rs:225-235`) honest — a configured
  API key counts as "credentials configured".
- Don't log key secrets. Log only `id`/`name`.
- Constant-time compare on the secret to avoid timing oracles.
- Shared crate → coordinate publish with StackArr.
