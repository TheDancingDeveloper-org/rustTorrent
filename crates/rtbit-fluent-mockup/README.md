# rtbit-fluent-mockup

A static mockup of the rustTorrent UI rebuilt with [fluent2-egui](https://github.com/TheDancingDeveloper-org/fluent2-egui)
(Fluent 2 for egui). It exists to judge a Fluent look for rustTorrent before any real port: it
uses about 200 fake torrents and has no backend. Nothing it shows is real, and no action leaves the
process.

The screens follow the React web UI in `crates/librtbit/webui/src/components`: the same columns,
detail tabs, context-menu order, settings tabs and fields, RSS tabs, log view and login page.

## Run it

Native (eframe, glow):

```sh
cargo run -p rtbit-fluent-mockup
```

Web (wasm through [trunk](https://trunkrs.dev), as fluent2-egui's gallery does):

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk --version 0.21.14
cd crates/rtbit-fluent-mockup
trunk serve            # http://127.0.0.1:8080
trunk build --release  # static bundle in dist/
```

The manual workflow `.github/workflows/fluent-mockup-web.yml` builds the same bundle and uploads
it as the `rtbit-fluent-mockup-web` artifact.

### Start-up flags (native)

For screenshots and quick checks the native build takes:

| flag | opens |
|---|---|
| `--screen login`, `rss`, `logs` | that page instead of the torrent list |
| `--screen settings --tab N` | the settings dialog on tab N (0 General … 9 RSS) |
| `--screen add`, `files`, `upload`, `delete` | the magnet, file-selection, multi-upload or delete dialog |
| `--detail N` | detail-pane tab N (0 Overview … 5 Pieces) |
| `--dark`, `--compact` | dark theme, compact density |

## What is in it

- **Toolbar:** page tabs (Torrents, RSS, Logs), torrent filter, Add magnet, Upload, Resume, Pause,
  Delete (enabled with a selection) and a selected-count badge, Settings, Logs, a View menu
  (density, column picker, sidebar, shortcuts), theme toggle and Log out.
- **Sidebar:** Status, Categories (with Uncategorized) and Trackers (with Trackerless), with counts.
- **Torrent table:** fluent2 `DataGrid`, virtualised, with multi-select (checkboxes, Shift and Ctrl),
  sortable columns and 20 columns from the web UI (14 shown by default). Speeds, progress and
  pieces animate once a second.
- **Context menu** on rows, in the web UI's order: Resume, Pause, Force Recheck, Configure Files,
  Sequential, Super-Seed, queue moves, limits, Set Category submenu, copy actions, Delete.
- **Detail pane:** Overview (with a piece strip), Files (`TreeView`), Peers and Trackers
  (`DataGrid`s), Speed (`Sparkline`s) and Pieces (a custom-painted bitfield with hover detail).
- **Dialogs:** Add torrent (magnet) leading to file selection, Upload Multiple Torrents, delete
  confirmation with "Also delete downloaded files", keyboard shortcuts, and Settings with all ten
  tabs (General, Folders, Speed, Connections, Queue, Seeding, DHT, Security, HTTP API, RSS) and a
  settings search.
- **RSS page:** Items, Feeds and Rules tabs with Add/Edit Feed and Add/Edit Rule dialogs.
- **Logs:** a live fake log stream with level chips, a text filter, Pause/Resume, Follow, Copy
  and Clear.
- **Login page**, **toasts** for every action, and a **status bar** with alt-speed toggle, global
  speeds and totals, a speed sparkline, peers, uptime and the API link.
- **Themes:** Fluent light and dark; Compact, Medium and Touch density.

Keyboard: `M` magnet, `Delete` delete, `B` sidebar, `Ctrl+,` settings, `Ctrl+L` logs, `?` help.

## Screenshots

In [`docs/`](docs/), at 1440×900, generated from the native build under Xvfb:

| screen | light | dark |
|---|---|---|
| Torrents | [torrents-light](docs/torrents-light.png) | [torrents-dark](docs/torrents-dark.png), [compact](docs/torrents-compact-dark.png) |
| Detail tabs | [files](docs/detail-files-light.png), [trackers](docs/detail-trackers-light.png), [pieces](docs/detail-pieces-light.png) | [peers](docs/detail-peers-dark.png), [speed](docs/detail-speed-dark.png) |
| Context menu | [context-menu-light](docs/context-menu-light.png) | [context-menu-dark](docs/context-menu-dark.png) |
| Add and delete | [add-magnet](docs/add-magnet-light.png), [upload](docs/upload-light.png) | [add-files](docs/add-files-dark.png), [delete](docs/delete-dark.png) |
| Settings | [general](docs/settings-general-light.png), [connections](docs/settings-connections-light.png), [rss](docs/settings-rss-light.png) | [speed](docs/settings-speed-dark.png), [security](docs/settings-security-dark.png) |
| RSS | [rss-light](docs/rss-light.png) | [rss-dark](docs/rss-dark.png) |
| Logs | [logs-light](docs/logs-light.png) | [logs-dark](docs/logs-dark.png) |
| Login | [login-light](docs/login-light.png) | [login-dark](docs/login-dark.png) |

## Notes

- fluent2-egui is pinned by git rev (main after 0.2.0). The `v0.2.0` tag paints text-field fills
  over their text (fixed upstream in WI-1913), so the tag is not usable for a form-heavy UI.
- fluent2-egui's `Card` paints its shadow over its content; the login page and RSS tiles use a
  plain `egui::Frame` card instead (`src/widgets.rs`).
- Not wired, by design: per-torrent speed and seed limits, new categories, folder browsing beyond
  a fixed list, real file pickers. Each shows a toast instead.

## Checks

```sh
cargo clippy -p rtbit-fluent-mockup --all-targets
cargo nextest run -p rtbit-fluent-mockup   # unit tests and the egui_kittest smoke tests
cargo build -p rtbit-fluent-mockup --target wasm32-unknown-unknown
```
