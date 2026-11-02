# Fluent client parity checklist

This checklist compares the retired `rtbit-egui` client at the parent of
`b15018f` (removed by `b15018f`, WI-2770) with `rtbit-fluent` on graft line
`fluent-1`. “Done” means the Fluent client has the user-facing surface and
the corresponding bridge/client behavior; “missing” identifies follow-up
work; “dropped-on-purpose” records a deliberate boundary rather than a gap.

Sources: [`rtbit-egui` in history](https://github.com/TheDancingDeveloper-org/rustTorrent/tree/b15018f%5E/crates/rtbit-egui),
[`rtbit-fluent`](../crates/rtbit-fluent), and the shared action/data contracts
in [`rtbit-ui-core`](../crates/rtbit-ui-core).

## Screens and navigation

| Retired egui feature | Fluent status | Evidence / follow-up |
| --- | --- | --- |
| Connect screen: daemon URL, username, password, Connect | done | `screens/connect.rs`; bridge login and connection events are wired. |
| First-run setup: setup token, username, password, confirmation, Create account | done | `screens/connect.rs`; setup validation and bridge setup are wired. |
| Session navigation: Settings, RSS, Sign out | done | `shell.rs`; sign-out clears the session and returns to Connect. |
| Connection status indicator and window status | done | `widgets/status_bar.rs` and `shell.rs`. |
| Torrents screen and live polling | done | `screens/main_screen.rs`; snapshot polling, loading, and error states are present. |
| Torrent detail pane | done | `screens/detail.rs`; selected torrent drives the pane. |
| RSS screen | done | `screens/rss.rs`; feeds, items, rules, download, and matched episodes are present. |
| Settings screen | done | `screens/settings.rs`; all six retired tabs remain available. |

## Torrents screen

| Retired egui feature | Fluent status | Evidence / follow-up |
| --- | --- | --- |
| Torrent table columns: name, state, size, down, up, category, tracker | done | `screens/main_screen.rs` data table and snapshot formatting. |
| Status filter: All, Live, Downloading, Paused, Error, Initializing | done | Shared `StatusFilter` list and Fluent selectable filter. |
| Add torrent dialog | done | `screens/add_dialog.rs`. |
| Add by magnet link | done | Magnet validation requires `xt=urn:btih:` and submits through the bridge. |
| Add by `.torrent` file | done | Native file picker path and file bytes are supported. |
| Save path and optional category | done | Add dialog fields and `AddRequest`. |
| Start paused | done | Add dialog switch is included in the request. |
| Start/resume selected torrent | done | Toolbar and context menu dispatch `RowAction::Start`. |
| Pause selected torrent | done | Toolbar and context menu dispatch `RowAction::Pause`. |
| Delete selected torrent, with confirmation | done | Toolbar/context menu and `ConfirmModal` dispatch `RowAction::Delete`. |
| Keyboard shortcuts: add, paste magnet, delete, pause, select all, filter focus, refresh, context menu | done | `screens/main_screen.rs` key context and actions. |
| Empty, loading, filtered-empty, and request-error states | done | Main table rendering and action error bar. |

## Torrent detail

| Retired egui feature | Fluent status | Evidence / follow-up |
| --- | --- | --- |
| General tab with name, state, size, progress, rates, and metadata | done | `screens/detail.rs` `general` renderer. |
| Files tab with nested folders and files | done | Fluent file tree handles component paths and legacy slash-separated names. |
| Include/exclude file or folder selection | done | Checkbox and folder aggregation post file selections. |
| File priority label: Normal / Do not download / Mixed | done | `priority_label` and tree controls. |
| Pieces tab and piece progress summary | done | Pieces tab and empty/loading states are present. |
| Peers tab with address, state, download, and upload columns | done | `peers_table`. |
| Trackers tab with tracker URL, status, seeders, and leechers | done | `trackers_table`. |
| Add tracker URL and Add action | done | `screens/detail.rs` validates and submits the URL, reloads trackers, and preserves the selection and input on refusal. |
| No-selection, loading, and empty detail states | done | `empty_state_message` and async snapshot handling. |

## RSS

| Retired egui feature | Fluent status | Evidence / follow-up |
| --- | --- | --- |
| Feed list and selected feed items | done | `screens/rss.rs`; feed and item tables are rendered. |
| Item status: Downloaded / Download / No link | done | `item_status` and download action. |
| Download selected item | done | Bridge download action and disabled-state check. |
| Rules list with name, feeds, regex, priority, and enabled state | done | `rule_line` and rule table. |
| Add rule dialog/editor | done | Name, match regex, category, feed selection, enabled, and tracking fields are present. |
| Edit rule and matched episode history | done | Existing rule loads its fields and matched episodes. |
| Delete rule | done | Rule delete action is wired. |
| Rule validation and in-dialog save/cancel/error behavior | done | Client validation and bridge refusal remain in the editor. |
| Rule priority selector: Low / Normal / High | missing | Raise and implement [#287](https://github.com/TheDancingDeveloper-org/rustTorrent/issues/287) on graft line `fluent-1`; the current editor only displays the priority. |

## Settings fields

The retired client exposed six tabs. The Fluent implementation retains the
same tab set and field ownership; all rows below are present in the Fluent
draft, rendered form, validation, and section-specific save path unless noted.

| Tab | Fields and actions | Status |
| --- | --- | --- |
| General | Append `.part` to incomplete files; Save | done |
| Connection | Connect/read-write/keep-alive timeouts; upload slots; magnet timeout; listen IP and port; DHT; local discovery; UPnP; read-only peer exchange; proxy URL; clear proxy; IP filter; saved-vs-running network warning; Use startup flags; Save | done |
| Speed | Download/upload limits; peer limit; connection limit; alternative limits; alternative schedule enabled/start/end/days; Save | done |
| Folders | Download folder; completed folder; required-folder validation; Save | done |
| Seeding | Ratio limit; seed time; Save | done |
| Advanced | Maximum concurrent handshakes; fast-resume validation denominator; Save | done |
| Settings loading, validation errors, busy state, save notices, and restart notice | done | `SettingsScreen` async load/save state and validation messages. |

## Dialogs and boundaries

| Retired egui dialog or behavior | Fluent status | Evidence / follow-up |
| --- | --- | --- |
| Connect/setup forms and failure messages | done | Connect screen renders both modes and keeps errors visible. |
| Add-torrent modal with Cancel, file picker, validation, and submit | done | `AddDialog` and `validate_add`. |
| Delete-torrent confirmation | done | Main-screen `ConfirmModal`, Escape, and Enter handling. |
| RSS add/edit rule editor with Cancel, Save, feed selection, and matched episodes | done | RSS editor and episode section. |
| RSS delete-rule confirmation | done | Delete action is wired through the RSS screen. |
| OS keychain refresh-token persistence | done | Connect bridge uses `OsKeychain`; only refresh tokens are stored. |
| Native egui viewport/window controls and egui-specific theme installation | dropped-on-purpose | Fluent/GPUI owns the window shell and theme; these are toolkit implementation details, not product behavior. |
| Retired egui test harness and egui-only stand-ins | dropped-on-purpose | The Fluent crate has its own GPUI/unit/integration coverage; no compatibility shim is required. |

## Gap register

The only missing product features found in this audit are tracked by the two
new items above. No other egui screen, column, action, settings field, or
dialog is intentionally left without a status.
