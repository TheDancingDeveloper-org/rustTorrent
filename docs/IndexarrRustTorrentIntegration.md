# Indexarr + rustTorrent Integration

## Overview

rustTorrent integrates with [Indexarr](https://github.com/TheDancingDeveloper-org/indexarr-rs) to provide torrent index browsing directly from the web UI. Users can search a massive decentralized torrent index, view recently indexed torrents, and download with a single click — all without leaving rustTorrent.

The integration is **fully optional** — controlled by env vars, defaulting to off. When disabled, no Indexarr-related UI or endpoints are exposed.

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                    Browser                           │
│          rustTorrent Web UI (React)                  │
│  ┌──────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ Torrents  │  │ Browse Index │  │ Indexarr     │  │
│  │   Page    │  │    Page      │  │  Setup       │  │
│  └─────┬─────┘  └──────┬───────┘  └──────┬───────┘  │
└────────┼───────────────┼──────────────────┼──────────┘
         │               │                  │
         │ /torrents     │ /indexarr/*      │ /indexarr/identity/*
         ▼               ▼                  ▼
┌─────────────────────────────────────────────────────┐
│              rustTorrent (Axum)                       │
│                                                      │
│  Torrent Management    Indexarr Proxy Handlers        │
│  (existing endpoints)  (handlers/indexarr.rs)         │
│                        ┌─────────────────────┐       │
│                        │ Injects API key      │       │
│                        │ via X-Api-Key header │       │
│                        └──────────┬──────────┘       │
└───────────────────────────────────┼──────────────────┘
                                    │ HTTP (internal network)
                                    ▼
┌─────────────────────────────────────────────────────┐
│              Indexarr (FastAPI)                       │
│         --workers http_server,sync                   │
│                                                      │
│  REST API (/api/v1/)         P2P Sync               │
│  ├── GET /search             ├── Gossip protocol    │
│  ├── GET /recent             ├── Delta export/import│
│  ├── GET /trending           └── Peer discovery     │
│  ├── GET /torrent/{hash}                            │
│  ├── GET /identity/status                           │
│  ├── POST /identity/acknowledge                     │
│  └── GET/POST /system/sync/preferences              │
└──────────────────────┬──────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────┐
│              PostgreSQL 17                            │
│  torrents, torrent_content, torrent_files, etc.      │
└─────────────────────────────────────────────────────┘
```

## Configuration

### Environment Variables

| Env Var | Default | Description |
|---------|---------|-------------|
| `RTBIT_INDEXARR_ENABLED` | `false` | Master toggle. Must be `true` or `1` to enable. |
| `RTBIT_INDEXARR_URL` | _(none)_ | Indexarr base URL (e.g. `http://indexarr:8080`). Required when enabled. |
| `RTBIT_INDEXARR_API_KEY` | _(none)_ | API key injected into all proxied requests. Must match Indexarr's `INDEXARR_TORZNAB_API_KEY`. |

When disabled:
- All `/indexarr/*` proxy endpoints return `{"enabled": false}` or 404
- The "Browse Index" tab is hidden in the UI
- No impact on existing torrent management functionality

### Indexarr Configuration

Indexarr should run with `--workers http_server,sync` (sync-only relay mode):

| Indexarr Env Var | Value | Purpose |
|---------|-------|---------|
| `INDEXARR_TORZNAB_API_KEY` | Same as `RTBIT_INDEXARR_API_KEY` | Shared API key |
| `INDEXARR_SYNC_ENABLED` | `true` | Enable P2P sync |
| `INDEXARR_SYNC_PEERS` | `'["https://bootstrap.indexarr.net"]'` | Bootstrap peers |
| `INDEXARR_WORKERS` | `http_server,sync` | No DHT crawling |

## Backend: Proxy Handlers

File: `crates/librtbit/src/http_api/handlers/indexarr.rs`

All endpoints extract `indexarr_url` and `indexarr_api_key` from `HttpApiOptions`, inject the API key as an `X-Api-Key` header, and forward the request to Indexarr.

| rustTorrent Endpoint | Proxied To (Indexarr) | Purpose |
|-----|-----|-----|
| `GET /indexarr/status` | `GET /health` | Connectivity check |
| `GET /indexarr/search?q=...` | `GET /api/v1/search?q=...` | Full-text search with 16+ filters |
| `GET /indexarr/recent` | `GET /api/v1/recent` | Recently resolved torrents |
| `GET /indexarr/trending` | `GET /api/v1/trending` | Top torrents by seed count |
| `GET /indexarr/torrent/{hash}` | `GET /api/v1/torrent/{hash}` | Torrent detail with files |
| `GET /indexarr/identity/status` | `GET /api/v1/identity/status` | Onboarding status |
| `POST /indexarr/identity/acknowledge` | `POST /api/v1/identity/acknowledge` | Confirm key saved |
| `GET /indexarr/sync/preferences` | `GET /api/v1/system/sync/preferences` | Category preferences |
| `POST /indexarr/sync/preferences` | `POST /api/v1/system/sync/preferences` | Set category preferences |

Routes are registered in `handlers/mod.rs` within `make_api_router()`.

## Frontend

### Navigation

The toolbar is a two-row design:

**Row 1 (nav bar):** Logo + version | Page tabs (Torrents / Browse Index) | Filter input | Config/Logs/Dark mode icons

**Row 2 (action bar):** Add torrent buttons + bulk actions — only visible on the Torrents page

Page navigation is controlled by `uiStore.currentPage` (`"torrents" | "indexarr"`). The sidebar is hidden on the Indexarr page. Clicking any sidebar filter switches back to the Torrents page.

### Components

| Component | File | Purpose |
|-----------|------|---------|
| `IndexarrBrowse` | `components/IndexarrBrowse.tsx` | Main browse page — search + recent tabs |
| `IndexarrSetup` | `components/IndexarrSetup.tsx` | First-boot setup (identity + categories) |
| `RootContent` | `components/RootContent.tsx` | Routes between torrent view and Indexarr view |
| `Toolbar` | `components/Toolbar.tsx` | Nav bar with page tabs |

### State Management

| Store | Key State | Purpose |
|-------|-----------|---------|
| `uiStore` | `currentPage` | Page navigation (`"torrents"` / `"indexarr"`) |
| `indexarrStore` | `status`, `identity`, `searchResults`, `recentTorrents` | All Indexarr-related state |

### API Client

`IndexarrAPI` object in `http-api.ts` (separate from the main `API` object):

```typescript
IndexarrAPI.getStatus()              // GET /indexarr/status
IndexarrAPI.search(query, filters)   // GET /indexarr/search?q=...
IndexarrAPI.getRecent(limit)         // GET /indexarr/recent
IndexarrAPI.getTrending(limit)       // GET /indexarr/trending
IndexarrAPI.getIdentityStatus()      // GET /indexarr/identity/status
IndexarrAPI.acknowledgeIdentity()    // POST /indexarr/identity/acknowledge
IndexarrAPI.getSyncPreferences()     // GET /indexarr/sync/preferences
IndexarrAPI.setSyncPreferences(...)  // POST /indexarr/sync/preferences
```

### TypeScript Types

Defined in `api-types.ts`:

- `IndexarrStatus` — connectivity and readiness
- `IndexarrSearchResult` — search result with info_hash, name, size, content_type, trackers, etc.
- `IndexarrSearchResponse` — paginated search results with facets
- `IndexarrRecentItem` — recently indexed torrent
- `IndexarrIdentityStatus` — onboarding state (needs_onboarding, contributor_id, recovery_key)
- `IndexarrSyncPreferences` — category selection and sync settings

## Data Flow: Search to Download

```
1. User types in search box on Browse Index page
2. Frontend debounces (300ms) → calls IndexarrAPI.search()
3. GET /indexarr/search?q=query&content_type=movie&sort=seeders
4. Proxy: GET http://indexarr:8080/api/v1/search?q=... + X-Api-Key
5. Indexarr: PostgreSQL FTS → results with tracker URLs
6. Results rendered in table with download buttons

7. User clicks download
8. buildMagnet(info_hash, name, trackers) → "magnet:?xt=urn:btih:..."
9. API.uploadTorrent(magnet, { list_only: true }) → metadata resolved
10. FileSelectionModal opens — user picks files
11. API.uploadTorrent(magnet, { only_files: [...] })
12. rustTorrent starts downloading
```

## First-Time Setup Flow

1. User enables Indexarr integration (env vars) and starts the stack
2. Opens rustTorrent web UI → "Browse Index" tab appears
3. Clicks "Browse Index" → setup wizard shown (identity not yet acknowledged)
4. **Step 1:** Contributor ID and recovery key displayed. User copies key and clicks "I have saved my recovery key"
5. **Step 2:** Category checkboxes (movie, tv_show, music, etc.). User selects desired categories and clicks "Save Preferences"
6. Browse page loads — search and recent tabs available
7. Setup is accessible later via the gear icon on the Browse Index page

## Docker Deployment

### Combined Stack

Use `docker-compose.indexarr.yml` in the rustTorrent repo:

```bash
# Create .env
cat > .env <<EOF
RTBIT_DOWNLOADS=/path/to/downloads
RTBIT_COMPLETED=/path/to/completed
INDEXARR_TORZNAB_API_KEY=$(python3 -c "import secrets; print(secrets.token_hex(32))")
INDEXARR_SYNC_PEERS=["https://bootstrap.indexarr.net"]
EOF

# Start
docker compose -f docker-compose.indexarr.yml up -d

# Open http://localhost:3030/web/
```

Services:
- `postgres` — PostgreSQL 17 for Indexarr
- `indexarr` — sync + web server mode, internal port 8080 (not exposed to host)
- `rtbit` — port 3030 (web UI + API), port 4240 (BitTorrent)

### Separate Stacks

Run Indexarr independently and point rustTorrent to it:

```bash
# On rustTorrent
RTBIT_INDEXARR_ENABLED=true
RTBIT_INDEXARR_URL=http://your-indexarr-host:8080
RTBIT_INDEXARR_API_KEY=your-shared-api-key
```

## Security

- API key is never sent to the browser — injected server-side by the proxy
- Indexarr port 8080 is not exposed to the host in the combined Docker stack
- rustTorrent's auth middleware (JWT/Basic) protects all `/indexarr/*` proxy endpoints
- Indexarr's `require_admin_key` dependency provides defense-in-depth on the search/recent endpoints
- Magnet links are constructed client-side from data already received — no additional round-trips needed

## Files Changed

### Indexarr
- `indexarr/api/routes/search.py` — API key enforcement + trackers in search results
- `indexarr/api/app.py` — API key enforcement + trackers in recent results

### rustTorrent Backend
- `crates/librtbit/src/http_api/handlers/indexarr.rs` — Proxy handler module (new)
- `crates/librtbit/src/http_api/handlers/mod.rs` — Route registration
- `crates/librtbit/src/http_api/mod.rs` — `HttpApiOptions` fields
- `crates/rtbit/src/main.rs` — Env var reading

### rustTorrent Frontend
- `webui/src/api-types.ts` — Indexarr TypeScript interfaces
- `webui/src/http-api.ts` — `IndexarrAPI` client
- `webui/src/stores/uiStore.ts` — Page navigation state
- `webui/src/stores/indexarrStore.ts` — Indexarr state store (new)
- `webui/src/components/IndexarrBrowse.tsx` — Browse page (new)
- `webui/src/components/IndexarrSetup.tsx` — Setup wizard (new)
- `webui/src/components/Toolbar.tsx` — Redesigned nav bar with page tabs
- `webui/src/components/Sidebar.tsx` — Auto-switch to Torrents page on filter click
- `webui/src/components/RootContent.tsx` — Page routing
- `webui/src/rtbit-web.tsx` — Sidebar visibility based on current page

### Docker
- `docker-compose.indexarr.yml` — Combined stack (new)
- `.env.example` — Indexarr env var documentation
