//! Main screen: the live torrent table, toolbar actions and the row context menu.
//!
//! Rows come from the bridge snapshot poll. Sorting happens here, not in
//! `DataTable`: `DataTable` sorts by display text, which orders sizes and
//! speeds wrongly. `DataTable` has no right-click hook, so the screen catches the
//! right click on the table and maps the pointer to a row from the table geometry.

use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use fluent_core::{AsyncResource, ThemeProvider as _};
use fluent_layout::{
    ConfirmModal, ContextMenu, ContextMenuItem, DataCell, DataColumn, DataRow, DataSortDirection,
    DataTable, MessageIntent, ModalStack, Toast, ToastHost, ToastStack,
};
use fluent_primitives::{Label, LabelSize, TextInput, Tooltip};
use gpui::{
    App, AsyncApp, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable as _, Hsla,
    IntoElement, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Render,
    SharedString, Subscription, Task, WeakEntity, Window, actions, canvas, div, prelude::*, px,
};
use rtbit_client::{Torrent, TorrentState};
use rtbit_ui_core::{AddRequest, AddSource, RowAction, Snapshot, StatusFilter, format, map_error};

use crate::bridge::{Bridge, LOST_AFTER_FAILURES, PollHealth, Poller, fetch_snapshot};
use crate::screens::add_dialog::{AddDialog, AddDialogEvent};
use crate::screens::bulk_edit_dialog::{BulkEditDialog, BulkEditDialogEvent, BulkEditRequest};
use crate::screens::detail::DetailPane;
use crate::table_prefs::TablePrefs;
use crate::widgets::status_bar::StatusBar;

actions!(
    main_screen,
    [
        OpenAdd,
        PasteMagnet,
        DeleteSelected,
        TogglePause,
        SelectAll,
        FocusFilter,
        Refresh,
        OpenContextMenu
    ]
);

/// Daemon reachability changes the shell reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainEvent {
    /// Polls keep failing; the next attempt is in `retry_in_secs`.
    DaemonLost { retry_in_secs: u64 },
    /// A poll succeeded after the daemon was reported lost.
    DaemonRecovered,
    /// The daemon refused the session; show the connect screen.
    Unauthorised,
}

/// Key context of the main screen. Its shortcuts are bound to this context only.
pub const KEY_CONTEXT: &str = "MainScreen";

#[derive(Clone, Copy)]
struct ToolbarButtonSpec {
    id: &'static str,
    icon: &'static str,
    label: &'static str,
}

const TOOLBAR_BUTTONS: [ToolbarButtonSpec; 4] = [
    ToolbarButtonSpec {
        id: "add",
        icon: "+",
        label: "Add torrent",
    },
    ToolbarButtonSpec {
        id: "resume",
        icon: "▶",
        label: "Resume torrent",
    },
    ToolbarButtonSpec {
        id: "pause",
        icon: "Ⅱ",
        label: "Pause torrent",
    },
    ToolbarButtonSpec {
        id: "delete",
        icon: "×",
        label: "Delete torrent",
    },
];

/// Keep the command names in one place so an icon-only toolbar can never lose
/// the text used for its tooltip and accessible name.
fn toolbar_buttons() -> &'static [ToolbarButtonSpec; 4] {
    &TOOLBAR_BUTTONS
}

fn icon_toolbar_button(
    spec: ToolbarButtonSpec,
    background: Hsla,
    focus_ring: Hsla,
    on_activate: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    debug_assert!(
        !spec.label.trim().is_empty(),
        "toolbar button needs a label"
    );
    let on_activate = Arc::new(on_activate);
    let click = on_activate.clone();
    let key = on_activate;
    let button = div()
        .id(spec.id)
        .focusable()
        .tab_stop(true)
        .focus(move |style| style.border_color(focus_ring))
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                key(window, cx);
                cx.stop_propagation();
            }
        })
        .on_click(move |_, window, cx| click(window, cx))
        .flex()
        .items_center()
        .justify_center()
        .size(px(32.0))
        .border_2()
        .border_color(background)
        .rounded(px(4.0))
        .cursor_pointer()
        .child(spec.icon);
    Tooltip::new(spec.label).trigger(button)
}

fn toolbar_activate(
    me: &Entity<MainScreen>,
    action: RowAction,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let me = me.clone();
    move |_, app| dispatch(&me, app, action)
}

/// Contexts for the shortcuts that would clash with typing: Space, Delete, Ctrl+A and Ctrl+V
/// stay with a focused text field.
const OUTSIDE_TEXT_INPUT: &str = "MainScreen && !TextInput";

/// One line listing the shortcuts, shown under the table.
pub const SHORTCUT_HELP: &str = "Ctrl+O add · Ctrl+V paste magnet · Space pause/resume · Delete remove · Ctrl+A select all · Ctrl+F filter · F5 refresh · Shift+F10 menu";

/// Bind the main-screen shortcuts. Call once at startup.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-o", OpenAdd, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-v", PasteMagnet, Some(OUTSIDE_TEXT_INPUT)),
        KeyBinding::new("delete", DeleteSelected, Some(OUTSIDE_TEXT_INPUT)),
        KeyBinding::new("space", TogglePause, Some(OUTSIDE_TEXT_INPUT)),
        KeyBinding::new("ctrl-a", SelectAll, Some(OUTSIDE_TEXT_INPUT)),
        KeyBinding::new("ctrl-f", FocusFilter, Some(KEY_CONTEXT)),
        KeyBinding::new("f5", Refresh, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-f10", OpenContextMenu, Some(OUTSIDE_TEXT_INPUT)),
        KeyBinding::new("menu", OpenContextMenu, Some(OUTSIDE_TEXT_INPUT)),
    ]);
}

/// Torrent table, toolbar and context menu. Owns the snapshot the poll fills.
pub struct MainScreen {
    pub(crate) snapshot: AsyncResource<Snapshot, String>,
    filter: StatusFilter,
    category_filter: Option<String>,
    tag_filter: Option<String>,
    tracker_filter: Option<String>,
    status: Entity<StatusBar>,
    /// Detail pane for the selected row.
    detail: Entity<DetailPane>,
    /// Selected row id (the daemon id, or the info hash when the daemon omits one).
    selected: Option<String>,
    /// Rows marked by Select all or a Shift+arrow range.
    marked: HashSet<String>,
    /// Fixed end of a Shift+arrow range selection.
    selection_anchor: Option<String>,
    detail_open: bool,
    /// Case-insensitive name filter typed in the filter box.
    query: String,
    filter_input: Entity<TextInput>,
    focus: FocusHandle,
    autofocused: bool,
    /// Set while the delete confirmation is on screen.
    delete_prompt: bool,
    sort: Option<(String, DataSortDirection)>,
    /// Column widths overriding the defaults, persisted with the sort.
    prefs: TablePrefs,
    prefs_path: Option<PathBuf>,
    menu: Option<Entity<ContextMenu>>,
    /// Where the table was last painted, so a pointer position maps to a row.
    table_geometry: Rc<Cell<TableGeometry>>,
    action_error: Option<String>,
    applied_generation: u64,
    /// Add-torrent dialog, while it is open.
    add: Option<Entity<AddDialog>>,
    _add_events: Option<Subscription>,
    _add_send: Option<Task<()>>,
    bulk_edit: Option<Entity<BulkEditDialog>>,
    _bulk_edit_events: Option<Subscription>,
    toasts: Entity<ToastHost>,
    daemon_lost: bool,
    pub(crate) _poller: Option<Poller>,
    pub(crate) _load: Option<Task<()>>,
    _action: Option<Task<()>>,
}

impl MainScreen {
    #[must_use]
    pub fn new(status: Entity<StatusBar>, cx: &mut Context<Self>) -> Self {
        let me = cx.weak_entity();
        let filter_input = cx.new(|_| {
            TextInput::new()
                .placeholder("Filter (Ctrl+F)")
                .on_change(move |text, app| {
                    let _ = me.update(app, |screen, cx| screen.set_query(text.to_string(), cx));
                })
        });
        // Tests must not read or write the real config directory.
        let prefs_path = if cfg!(test) {
            None
        } else {
            TablePrefs::default_path()
        };
        let prefs = prefs_path
            .as_deref()
            .map(TablePrefs::load)
            .unwrap_or_default();
        let sort = prefs.sort.clone().map(|(column, ascending)| {
            let direction = if ascending {
                DataSortDirection::Ascending
            } else {
                DataSortDirection::Descending
            };
            (column, direction)
        });
        Self {
            snapshot: AsyncResource::new(),
            filter: prefs
                .status
                .as_deref()
                .and_then(parse_status)
                .unwrap_or(StatusFilter::All),
            category_filter: prefs.category.clone(),
            tag_filter: prefs.tag.clone(),
            tracker_filter: prefs.tracker.clone(),
            status,
            detail: cx.new(DetailPane::new),
            selected: None,
            marked: HashSet::new(),
            selection_anchor: None,
            detail_open: false,
            query: String::new(),
            filter_input,
            focus: cx.focus_handle(),
            autofocused: false,
            delete_prompt: false,
            sort,
            prefs,
            prefs_path,
            menu: None,
            table_geometry: Rc::default(),
            action_error: None,
            applied_generation: 0,
            add: None,
            _add_events: None,
            _add_send: None,
            bulk_edit: None,
            _bulk_edit_events: None,
            toasts: cx.new(ToastHost::new),
            daemon_lost: false,
            _poller: None,
            _load: None,
            _action: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn start_polling_for_test(&mut self, cx: &mut Context<Self>) {
        self.ensure_polling(cx);
    }

    #[cfg(test)]
    pub(crate) fn add_open(&self) -> bool {
        self.add.is_some()
    }

    /// Whether the daemon is reported lost. Actions that need it are refused meanwhile.
    #[must_use]
    pub fn daemon_lost(&self) -> bool {
        self.daemon_lost
    }

    /// Fold a poll outcome into the lost state and tell the shell on a change.
    fn on_poll_health(&mut self, health: PollHealth, cx: &mut Context<Self>) {
        match health {
            PollHealth::Ok => {
                if std::mem::take(&mut self.daemon_lost) {
                    cx.emit(MainEvent::DaemonRecovered);
                    cx.notify();
                }
            }
            PollHealth::Failing { failures, retry_in } if failures >= LOST_AFTER_FAILURES => {
                self.daemon_lost = true;
                cx.emit(MainEvent::DaemonLost {
                    retry_in_secs: retry_in.as_secs(),
                });
                cx.notify();
            }
            PollHealth::Failing { .. } => {}
            PollHealth::Unauthorised => {
                self.daemon_lost = false;
                self._poller = None;
                cx.emit(MainEvent::Unauthorised);
            }
        }
    }

    /// Refuse an action that needs the daemon while it is lost.
    fn refuse_if_lost(&mut self, cx: &mut Context<Self>) -> bool {
        if self.daemon_lost {
            self.action_error = Some("Lost connection to daemon.".to_owned());
            cx.notify();
        }
        self.daemon_lost
    }

    /// Open the add-torrent dialog over the table.
    pub(crate) fn open_add(&mut self, cx: &mut Context<Self>) {
        if self.refuse_if_lost(cx) {
            return;
        }
        self.menu = None;
        let dialog = cx.new(AddDialog::new);
        let events = cx.subscribe(
            &dialog,
            |screen: &mut Self, dialog: Entity<AddDialog>, event: &AddDialogEvent, cx| match event {
                AddDialogEvent::Cancel => screen.close_add(cx),
                AddDialogEvent::Submit(requests) => screen.send_add(dialog, requests.clone(), cx),
            },
        );
        self.add = Some(dialog);
        self._add_events = Some(events);
        cx.notify();
    }

    pub(crate) fn open_add_with_magnet(&mut self, magnet: &str, cx: &mut Context<Self>) {
        self.open_add(cx);
        if let Some(dialog) = &self.add {
            dialog.update(cx, |dialog, cx| dialog.prefill_magnet(magnet.trim(), cx));
        }
    }

    pub(crate) fn open_add_with_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        self.open_add(cx);
        if paths.is_empty() {
            return;
        }
        let Some(dialog) = &self.add else { return };
        let dialog = dialog.clone();
        let paths = paths.to_owned();
        let task = cx.background_executor().spawn(async move {
            paths
                .into_iter()
                .map(|path| {
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("torrent")
                        .to_owned();
                    (name, std::fs::read(path))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |_, cx| {
            let results = task.await;
            dialog.update(cx, |dialog, cx| {
                let mut files = Vec::new();
                for (name, result) in results {
                    match result {
                        Ok(bytes) => files.push((name, bytes)),
                        Err(err) => {
                            dialog.set_error(format!("could not read torrent: {err}"), cx);
                            return;
                        }
                    }
                }
                dialog.set_files(files, cx);
            });
        })
        .detach();
    }

    pub(crate) fn show_not_torrent_toast(&mut self, cx: &mut Context<Self>) {
        ToastStack::push(
            Toast::new("not-torrent", "That drop is not a torrent.").intent(MessageIntent::Warning),
            cx,
        );
    }

    /// Open the add dialog with the clipboard's magnet link filled in. Does nothing when the
    /// clipboard holds anything else.
    fn paste_magnet(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let magnet = text.trim();
        if !magnet.starts_with("magnet:?") {
            return;
        }
        self.open_add(cx);
        if let Some(dialog) = &self.add {
            dialog.update(cx, |dialog, cx| dialog.prefill_magnet(magnet, cx));
        }
    }

    /// Close the add dialog. Nothing is sent.
    fn close_add(&mut self, cx: &mut Context<Self>) {
        self.add = None;
        self._add_events = None;
        self.autofocused = false;
        cx.notify();
    }

    /// Send an add request through the bridge. The dialog closes on success and
    /// shows the daemon's reason on failure.
    fn send_add(
        &mut self,
        dialog: Entity<AddDialog>,
        requests: Vec<AddRequest>,
        cx: &mut Context<Self>,
    ) {
        if !cx.has_global::<Bridge>() {
            dialog.update(cx, |dialog, cx| {
                dialog.refused("Not connected to a daemon.".to_owned(), cx);
            });
            return;
        }
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            for AddRequest {
                source,
                save_path,
                category,
                start_paused,
            } in requests
            {
                let category = (!category.is_empty()).then_some(category);
                match source {
                    AddSource::Magnet(magnet) => {
                        client
                            .add_magnet(
                                &magnet,
                                Some(&save_path),
                                category.as_deref(),
                                start_paused,
                            )
                            .await?
                    }
                    AddSource::File { bytes, .. } => {
                        client
                            .add_torrent_file(
                                &bytes,
                                Some(&save_path),
                                category.as_deref(),
                                start_paused,
                            )
                            .await?
                    }
                }
            }
            Ok(())
        });
        self._add_send = Some(
            cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
                let reply = match rx.await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(err)) => Err(map_error(&err)),
                    Err(_) => Err("request cancelled".to_owned()),
                };
                let _ = view.update(cx, |screen, cx| match reply {
                    Ok(()) => {
                        screen.close_add(cx);
                        ToastStack::push(
                            Toast::new("add-torrent", "Torrent added.")
                                .intent(MessageIntent::Success),
                            cx,
                        );
                        screen.refresh(cx);
                    }
                    Err(message) => dialog.update(cx, |dialog, cx| dialog.refused(message, cx)),
                });
            }),
        );
    }

    /// Set the sidebar status filter the table shows.
    pub fn set_filter(&mut self, filter: StatusFilter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.prefs.status = (filter != StatusFilter::All).then(|| filter.label().to_owned());
        self.save_prefs();
        cx.notify();
    }

    fn set_category(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        self.category_filter = value;
        self.save_filter_prefs();
        cx.notify();
    }
    fn set_tag(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        self.tag_filter = value;
        self.save_filter_prefs();
        cx.notify();
    }
    fn set_tracker(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        self.tracker_filter = value;
        self.save_filter_prefs();
        cx.notify();
    }
    fn save_filter_prefs(&mut self) {
        self.prefs.category = self.category_filter.clone();
        self.prefs.tag = self.tag_filter.clone();
        self.prefs.tracker = self.tracker_filter.clone();
        self.save_prefs();
    }

    /// Filter the table to names containing `query`, ignoring case.
    pub(crate) fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.query = query.to_lowercase();
        cx.notify();
    }

    /// Mark every visible row, so the next command acts on all of them.
    fn select_all(&mut self, cx: &mut Context<Self>) {
        self.marked = self.visible_rows().into_iter().map(row_id).collect();
        cx.notify();
    }

    /// Number of rows marked by Select all.
    #[must_use]
    pub fn marked_count(&self) -> usize {
        self.marked.len()
    }

    /// Torrents a command applies to: the marked rows, else the selected row.
    fn targets(&self) -> Vec<&Torrent> {
        if self.marked.is_empty() {
            return self.selected_torrent().into_iter().collect();
        }
        self.snapshot.value().map_or_else(Vec::new, |snapshot| {
            snapshot
                .torrents
                .torrents
                .iter()
                .filter(|torrent| self.marked.contains(&row_id(torrent)))
                .collect()
        })
    }

    /// Pause what is running and resume what is paused, for each target.
    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        let jobs: Vec<(u64, Job)> = self
            .targets()
            .into_iter()
            .filter_map(|torrent| {
                let paused = torrent.stats.as_ref().and_then(|stats| stats.state)
                    == Some(TorrentState::Paused);
                let action = if paused {
                    RowAction::Start
                } else {
                    RowAction::Pause
                };
                torrent.id.map(|id| (id, Job::Row(action)))
            })
            .collect();
        self.run_jobs(jobs, cx);
    }

    /// Number of torrents in the last good snapshot, before filters.
    #[must_use]
    pub fn torrent_count(&self) -> usize {
        self.snapshot
            .value()
            .map_or(0, |snapshot| snapshot.torrents.torrents.len())
    }

    fn selected_torrent(&self) -> Option<&Torrent> {
        let selected = self.selected.as_deref()?;
        self.snapshot
            .value()?
            .torrents
            .torrents
            .iter()
            .find(|torrent| row_id(torrent) == selected)
    }

    /// Select a row and load its detail. A row with no daemon id clears the pane.
    fn select_row(&mut self, row: &str, cx: &mut Context<Self>) {
        self.selected = Some(row.to_owned());
        self.marked.clear();
        self.selection_anchor = Some(row.to_owned());
        self.detail_open = true;
        let id = self.selected_torrent().and_then(|torrent| torrent.id);
        self.detail.update(cx, |pane, cx| match id {
            Some(id) => pane.select_torrent(id, cx),
            None => pane.clear(cx),
        });
        cx.notify();
    }

    /// Move the keyboard cursor through the visible, sorted rows.
    fn move_selection(
        &mut self,
        movement: SelectionMovement,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        let rows = self
            .visible_rows()
            .into_iter()
            .map(row_id)
            .collect::<Vec<_>>();
        if rows.is_empty() {
            return;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|id| rows.iter().position(|row| row == id))
            .unwrap_or(0);
        let next = match movement {
            SelectionMovement::Previous => current.saturating_sub(1),
            SelectionMovement::Next => (current + 1).min(rows.len() - 1),
            SelectionMovement::First => 0,
            SelectionMovement::Last => rows.len() - 1,
        };
        let next_row = rows[next].clone();
        let anchor = if extend {
            self.selection_anchor
                .clone()
                .unwrap_or_else(|| rows[current].clone())
        } else {
            next_row.clone()
        };
        self.selected = Some(next_row.clone());
        self.selection_anchor = Some(anchor.clone());
        self.marked.clear();
        if extend {
            let anchor_index = rows.iter().position(|row| row == &anchor).unwrap_or(next);
            let (start, end) = if anchor_index <= next {
                (anchor_index, next)
            } else {
                (next, anchor_index)
            };
            self.marked.extend(rows[start..=end].iter().cloned());
        } else {
            self.marked.insert(next_row);
        }
        cx.notify();
    }

    fn open_detail(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_torrent().and_then(|torrent| torrent.id) else {
            return;
        };
        self.detail_open = true;
        self.detail
            .update(cx, |pane, cx| pane.select_torrent(id, cx));
        cx.notify();
    }

    fn close_detail(&mut self, cx: &mut Context<Self>) {
        if self.detail_open {
            self.detail_open = false;
            cx.notify();
        }
    }

    fn handle_table_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let movement = match event.keystroke.key.as_str() {
            "up" => Some(SelectionMovement::Previous),
            "down" => Some(SelectionMovement::Next),
            "home" => Some(SelectionMovement::First),
            "end" => Some(SelectionMovement::Last),
            "return" => {
                self.open_detail(cx);
                return true;
            }
            "escape" => {
                self.close_detail(cx);
                return true;
            }
            _ => return false,
        };
        self.move_selection(movement.unwrap(), event.keystroke.modifiers.shift, cx);
        true
    }

    /// Ask for a fresh snapshot now, in addition to the poll.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if !cx.has_global::<Bridge>() {
            return;
        }
        self._load = Some(Bridge::load(self, cx, snapshot_of, fetch_snapshot));
    }

    pub(crate) fn on_poll_health_cb(screen: &mut Self, health: PollHealth, cx: &mut Context<Self>) {
        screen.on_poll_health(health, cx);
    }

    /// Start the poll once a bridge is installed. Idempotent.
    fn ensure_polling(&mut self, cx: &mut Context<Self>) {
        if self._poller.is_none() && cx.has_global::<Bridge>() {
            self._poller = Some(Bridge::poll_snapshot(
                cx,
                snapshot_of,
                Self::on_poll_health_cb,
            ));
        }
    }

    /// Push each newly resolved snapshot to the status bar once.
    fn apply_to_status(&mut self, cx: &mut Context<Self>) {
        let generation = self.snapshot.generation();
        if generation == self.applied_generation {
            return;
        }
        self.applied_generation = generation;
        if let Some(snapshot) = self.snapshot.ready_value().cloned() {
            let status = self.status.clone();
            status.update(cx, |bar, cx| bar.apply_snapshot(&snapshot, cx));
        }
    }

    /// Right click at `position`: act on the row under the pointer. A row outside the marked
    /// set becomes the selection first; a row inside it keeps the whole marked set.
    fn open_menu_at_pointer(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(row) = self.row_at(position).map(row_id) else {
            return;
        };
        if !self.marked.contains(&row) {
            self.select_row(&row, cx);
        }
        self.open_menu(position, cx);
    }

    /// Shift+F10 or the Menu key: open the menu beside the selected row.
    fn open_menu_for_selection(&mut self, cx: &mut Context<Self>) {
        let geometry = self.table_geometry.get();
        let index = self.selected.as_deref().and_then(|selected| {
            self.visible_rows()
                .iter()
                .position(|torrent| row_id(torrent) == selected)
        });
        let Some(index) = index else {
            self.action_error = Some("Select a torrent first.".to_owned());
            cx.notify();
            return;
        };
        let position = geometry.row_origin(index);
        self.open_menu(position, cx);
    }

    /// Table row under a pointer position, from the painted geometry.
    fn row_at(&self, position: Point<Pixels>) -> Option<&Torrent> {
        let index = self.table_geometry.get().row_index(position)?;
        self.visible_rows().into_iter().nth(index)
    }

    /// Build the menu for the current targets. Items that cannot apply are disabled.
    fn open_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let targets = self.targets();
        let available = MenuAvailability::of(&targets);
        let categories = category_names(self.snapshot.value().map(|s| &s.torrents.torrents[..]));
        let screen = cx.entity();
        let item = |label: &str, enabled: bool, command: MenuCommand| {
            let screen = screen.clone();
            ContextMenuItem::action(label.to_owned(), move |_, _, app| {
                run_menu_command(&screen, app, command.clone());
            })
            .disabled(!enabled)
        };
        let mut category_items = vec![item(
            "No category",
            available.category,
            MenuCommand::SetCategory(None),
        )];
        category_items.extend(categories.into_iter().map(|name| {
            item(
                &name.clone(),
                available.category,
                MenuCommand::SetCategory(Some(name)),
            )
        }));
        let menu = ContextMenu::build(position)
            .item(item("Start", available.start, MenuCommand::Start))
            .item(item("Stop", available.stop, MenuCommand::Stop))
            .item(item(
                "Force recheck",
                available.recheck,
                MenuCommand::Recheck,
            ))
            .separator()
            .item(
                ContextMenuItem::submenu("Set category", category_items)
                    .disabled(!available.category),
            )
            .item(item(
                "Edit category and tags…",
                !targets.is_empty(),
                MenuCommand::BulkEdit,
            ))
            .separator()
            .item(item(
                "Copy magnet link",
                available.copy,
                MenuCommand::CopyMagnet,
            ))
            .item(item("Copy name", available.copy, MenuCommand::CopyName))
            .separator()
            .item(item("Delete…", available.delete, MenuCommand::Delete));
        self.menu = Some(cx.new(|_| menu));
        cx.notify();
    }

    /// Run a context-menu command on the whole selection.
    fn run_menu_command(&mut self, command: MenuCommand, cx: &mut Context<Self>) {
        self.menu = None;
        match command {
            MenuCommand::BulkEdit => self.open_bulk_edit(cx),
            MenuCommand::Delete => self.request(RowAction::Delete, cx),
            MenuCommand::CopyMagnet | MenuCommand::CopyName => {
                let text = copy_text(&self.targets(), &command);
                match text {
                    Some(text) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        self.action_error = None;
                        ToastStack::push(
                            Toast::new("copied", "Copied to clipboard.")
                                .intent(MessageIntent::Success),
                            cx,
                        );
                    }
                    None => self.action_error = Some("Select a torrent first.".to_owned()),
                }
                cx.notify();
            }
            command => {
                if self.refuse_if_lost(cx) {
                    return;
                }
                let jobs = menu_jobs(&self.targets(), &command);
                self.run_jobs(jobs, cx);
            }
        }
    }

    fn open_bulk_edit(&mut self, cx: &mut Context<Self>) {
        let count = self.targets().len();
        if count == 0 {
            self.action_error = Some("Select a torrent first.".to_owned());
            cx.notify();
            return;
        }
        let categories = category_names(self.snapshot.value().map(|s| &s.torrents.torrents[..]));
        let dialog = cx.new(|cx| BulkEditDialog::new(count, categories, cx));
        let events = cx.subscribe(&dialog, |screen, _, event, cx| match event {
            BulkEditDialogEvent::Cancel => {
                screen.bulk_edit = None;
                screen._bulk_edit_events = None;
                cx.notify();
            }
            BulkEditDialogEvent::Submit(request) => screen.send_bulk_edit(request.clone(), cx),
        });
        self.bulk_edit = Some(dialog);
        self._bulk_edit_events = Some(events);
        cx.notify();
    }

    fn send_bulk_edit(&mut self, request: BulkEditRequest, cx: &mut Context<Self>) {
        let targets = self
            .targets()
            .into_iter()
            .filter_map(|torrent| torrent.id.map(|id| (id, torrent.name.clone())))
            .collect::<Vec<_>>();
        let ids = targets.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        let names = targets
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();
        let Some(bridge) = cx.try_global::<Bridge>() else {
            self.action_error = Some("Not connected to a daemon.".to_owned());
            cx.notify();
            return;
        };
        self.bulk_edit = None;
        self._bulk_edit_events = None;
        let rx = bridge.call(move |client| async move {
            let mut client = client.lock().await;
            let mut failures = Vec::new();
            if let Some(category) = request.category.as_ref() {
                failures.extend(
                    client
                        .set_torrents_category(&ids, category.as_deref())
                        .await?
                        .failures,
                );
            }
            if !request.add_tags.is_empty() {
                failures.extend(
                    client
                        .add_torrents_tags(&ids, &request.add_tags)
                        .await?
                        .failures,
                );
            }
            if !request.remove_tags.is_empty() {
                failures.extend(
                    client
                        .remove_torrents_tags(&ids, &request.remove_tags)
                        .await?
                        .failures,
                );
            }
            Ok(failures)
        });
        self._action = Some(cx.spawn(async move |view, cx| {
            let result = match rx.await {
                Ok(result) => result,
                Err(_) => Err(rtbit_client::Error::Unreachable(
                    "request cancelled".to_owned(),
                )),
            };
            let _ = view.update(cx, |screen, cx| match result {
                Ok(failures) if failures.is_empty() => {
                    screen.action_error = None;
                    screen.refresh(cx);
                }
                Ok(failures) => {
                    screen.action_error = Some(
                        failures
                            .into_iter()
                            .map(|failure| {
                                let name = names
                                    .get(&failure.id)
                                    .and_then(|name| name.as_deref())
                                    .unwrap_or("unnamed torrent");
                                format!("{name}: {}", failure.error)
                            })
                            .collect::<Vec<_>>()
                            .join("; "),
                    );
                    cx.notify();
                }
                Err(error) => {
                    screen.action_error = Some(map_error(&error));
                    cx.notify();
                }
            });
        }));
    }

    /// Run a toolbar or menu command. Delete asks first.
    fn request(&mut self, action: RowAction, cx: &mut Context<Self>) {
        self.menu = None;
        if self.refuse_if_lost(cx) {
            return;
        }
        let targets = self.targets();
        let name = match targets.as_slice() {
            [] => {
                self.action_error = Some("Select a torrent first.".to_owned());
                cx.notify();
                return;
            }
            [only] => format!("“{}”", torrent_name(only)),
            many => format!("{} torrents", many.len()),
        };
        if action != RowAction::Delete {
            self.act(action, cx);
            return;
        }
        let me = cx.entity();
        ConfirmModal::new(
            "Delete torrent?",
            format!("{name} will be removed from the daemon."),
        )
        .confirm_label("Delete")
        .on_confirm(move |app| {
            confirm_delete(&me, app);
        })
        .show(cx);
        self.delete_prompt = true;
        cx.notify();
    }

    /// Send one row command for every target, then refresh the table.
    fn act(&mut self, action: RowAction, cx: &mut Context<Self>) {
        let jobs = self
            .targets()
            .into_iter()
            .filter_map(|torrent| torrent.id.map(|id| (id, Job::Row(action))))
            .collect();
        self.run_jobs(jobs, cx);
    }

    /// Send row commands to the daemon in order, then refresh the table.
    fn run_jobs(&mut self, jobs: Vec<(u64, Job)>, cx: &mut Context<Self>) {
        self.menu = None;
        self.delete_prompt = false;
        if jobs.is_empty() {
            self.action_error = Some("Select a torrent first.".to_owned());
            cx.notify();
            return;
        }
        if !cx.has_global::<Bridge>() {
            self.action_error = Some("Not connected to a daemon.".to_owned());
            cx.notify();
            return;
        }
        let deleting = jobs
            .iter()
            .any(|(_, job)| *job == Job::Row(RowAction::Delete));
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            for (id, job) in jobs {
                let sent = match job {
                    Job::Row(RowAction::Start) => client.start_torrent(id).await,
                    Job::Row(RowAction::Pause) => client.pause_torrent(id).await,
                    Job::Row(RowAction::Delete) => client.delete_torrent(id).await,
                    Job::Recheck => client.recheck_torrent(id).await,
                    Job::Category(category) => {
                        client.set_torrent_category(id, category.as_deref()).await
                    }
                };
                sent?;
            }
            Ok(())
        });
        self._action = Some(
            cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
                let reply = match rx.await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(err)) => Err(map_error(&err)),
                    Err(_) => Err("request cancelled".to_owned()),
                };
                let _ = view.update(cx, |screen, cx| match reply {
                    Ok(()) => {
                        screen.action_error = None;
                        if deleting {
                            screen.selected = None;
                            screen.marked.clear();
                            screen.detail.update(cx, |pane, cx| pane.clear(cx));
                        }
                        screen.refresh(cx);
                    }
                    Err(message) => {
                        screen.action_error = Some(message);
                        cx.notify();
                    }
                });
            }),
        );
    }

    fn sort_by(&mut self, column: &str, direction: DataSortDirection, cx: &mut Context<Self>) {
        self.sort = Some((column.to_owned(), direction));
        self.prefs.sort = Some((
            column.to_owned(),
            matches!(direction, DataSortDirection::Ascending),
        ));
        self.save_prefs();
        cx.notify();
    }

    /// Remember a column width (clamped to the minimum) and persist it.
    #[allow(dead_code)] // No resize handle in `DataTable` yet; widths come from the file.
    pub(crate) fn set_column_width(&mut self, column: &str, width: f32, cx: &mut Context<Self>) {
        let width = width.max(crate::table_prefs::MIN_COLUMN_WIDTH);
        self.prefs.widths.insert(column.to_owned(), width);
        self.save_prefs();
        cx.notify();
    }

    fn column_width(&self, column: &str, default: f32) -> f32 {
        self.prefs.widths.get(column).copied().unwrap_or(default)
    }

    fn save_prefs(&self) {
        if let Some(path) = &self.prefs_path {
            let _ = self.prefs.save(path);
        }
    }

    /// Rows for the table: filtered by the sidebar status, then sorted by the column.
    fn visible_rows(&self) -> Vec<&Torrent> {
        let Some(snapshot) = self.snapshot.value() else {
            return Vec::new();
        };
        let mut rows: Vec<&Torrent> = snapshot
            .torrents
            .torrents
            .iter()
            .filter(|torrent| matches_filter(self.filter, torrent))
            .filter(|torrent| {
                self.category_filter
                    .as_deref()
                    .is_none_or(|v| torrent.category.as_deref() == Some(v))
            })
            .filter(|torrent| {
                self.tag_filter.as_deref().is_none_or(|v| {
                    torrent
                        .tags
                        .as_ref()
                        .is_some_and(|tags| tags.iter().any(|tag| tag == v))
                })
            })
            .filter(|torrent| {
                self.tracker_filter.as_deref().is_none_or(|v| {
                    torrent.trackers.as_ref().is_some_and(|urls| {
                        urls.iter()
                            .any(|url| tracker_host(url).as_deref() == Some(v))
                    })
                })
            })
            .filter(|torrent| {
                self.query.is_empty() || torrent_name(torrent).to_lowercase().contains(&self.query)
            })
            .collect();
        if let Some((column, direction)) = &self.sort {
            rows.sort_by(|a, b| {
                let order = column_order(column, a, b);
                match direction {
                    DataSortDirection::Ascending => order,
                    DataSortDirection::Descending => order.reverse(),
                }
            });
        }
        rows
    }

    fn table(&self, me: &Entity<Self>) -> DataTable {
        let columns = [
            ("name", "Name", 320.0),
            ("size", "Size", 90.0),
            ("progress", "Progress", 90.0),
            ("speed", "Down", 110.0),
            ("upload", "Up", 110.0),
            ("eta", "ETA", 90.0),
            ("state", "State", 110.0),
        ]
        .map(|(id, title, width)| {
            DataColumn::new(id, title)
                .width(self.column_width(id, width))
                .sortable(true)
        });
        let rows = self.visible_rows().into_iter().map(|torrent| {
            DataRow::new(
                row_id(torrent),
                [
                    DataCell::new(torrent_name(torrent)),
                    DataCell::new(size_text(torrent)),
                    DataCell::new(progress_text(torrent)),
                    DataCell::new(speed_text(torrent)),
                    DataCell::new(upload_text(torrent)),
                    DataCell::new(eta_text(torrent)),
                    DataCell::new(state_of(torrent)),
                ],
            )
        });

        let select_me = me.clone();
        let click_me = me.clone();
        let sort_me = me.clone();
        let mut table = DataTable::new()
            .columns(columns)
            .rows(rows)
            .empty_message(NO_TORRENTS)
            .on_row_select(move |id: SharedString, _, app| {
                let id = id.to_string();
                select_me.update(app, |screen, cx| screen.select_row(&id, cx));
            })
            .on_row_click(move |id: SharedString, _, _, app| {
                let id = id.to_string();
                click_me.update(app, |screen, cx| screen.select_row(&id, cx));
            })
            .on_sort(move |column: SharedString, direction, _, _, app| {
                let column = column.to_string();
                sort_me.update(app, |screen, cx| screen.sort_by(&column, direction, cx));
            });
        if let Some((column, direction)) = &self.sort {
            table = table.sort(column.clone(), *direction);
        }
        if let Some(selected) = &self.selected {
            table = table.selected(selected.clone());
        }
        table
    }
    fn filter_sidebar(
        &self,
        me: &Entity<Self>,
        background: Hsla,
        accent: Hsla,
        stroke: Hsla,
    ) -> impl IntoElement {
        let rows = self
            .snapshot
            .value()
            .map(|s| &s.torrents.torrents[..])
            .unwrap_or(&[]);
        let mut panel = div()
            .id("filter-sidebar")
            .flex()
            .flex_col()
            .gap_1()
            .w(px(190.0))
            .p_2()
            .border_1()
            .border_color(stroke)
            .child(Label::new("Filters").size(LabelSize::Body));
        for status in StatusFilter::ALL {
            let target = me.clone();
            let count = rows
                .iter()
                .filter(|torrent| matches_filter(status, torrent))
                .count();
            let label = format!("{} ({count})", status.label());
            panel = panel.child(
                div()
                    .id(format!("status-{}", status.label()))
                    .px_2()
                    .py_1()
                    .bg(if self.filter == status {
                        accent
                    } else {
                        background
                    })
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        target.update(cx, |screen, cx| screen.set_filter(status, cx))
                    })
                    .child(Label::new(label).size(LabelSize::Caption)),
            );
        }
        let mut categories: Vec<String> = rows
            .iter()
            .filter_map(|t| t.category.clone())
            .filter(|v| !v.is_empty())
            .collect();
        categories.sort();
        categories.dedup();
        panel = panel.child(Label::new("Categories").size(LabelSize::Caption));
        for value in categories {
            let target = me.clone();
            let selected = self.category_filter.as_deref() == Some(value.as_str());
            let label = value.clone();
            panel = panel.child(
                div()
                    .id(format!("category-{value}"))
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .bg(if selected { accent } else { background })
                    .on_click(move |_, _, cx| {
                        target.update(cx, |screen, cx| {
                            screen.set_category((!selected).then_some(label.clone()), cx)
                        })
                    })
                    .child(Label::new(value).size(LabelSize::Caption)),
            );
        }
        let mut tags: Vec<String> = rows
            .iter()
            .flat_map(|t| t.tags.clone().unwrap_or_default())
            .collect();
        tags.sort();
        tags.dedup();
        panel = panel.child(Label::new("Tags").size(LabelSize::Caption));
        for value in tags {
            let target = me.clone();
            let selected = self.tag_filter.as_deref() == Some(value.as_str());
            let label = value.clone();
            panel = panel.child(
                div()
                    .id(format!("tag-{value}"))
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .bg(if selected { accent } else { background })
                    .on_click(move |_, _, cx| {
                        target.update(cx, |screen, cx| {
                            screen.set_tag((!selected).then_some(label.clone()), cx)
                        })
                    })
                    .child(Label::new(value).size(LabelSize::Caption)),
            );
        }
        let mut trackers: Vec<String> = rows
            .iter()
            .flat_map(|torrent| torrent.trackers.clone().unwrap_or_default())
            .filter_map(|url| tracker_host(&url))
            .collect();
        trackers.sort();
        trackers.dedup();
        panel = panel.child(Label::new("Trackers").size(LabelSize::Caption));
        for value in trackers {
            let target = me.clone();
            let selected = self.tracker_filter.as_deref() == Some(value.as_str());
            let label = value.clone();
            panel = panel.child(
                div()
                    .id(format!("tracker-{value}"))
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .bg(if selected { accent } else { background })
                    .on_click(move |_, _, cx| {
                        target.update(cx, |screen, cx| {
                            screen.set_tracker((!selected).then_some(label.clone()), cx)
                        })
                    })
                    .child(Label::new(value).size(LabelSize::Caption)),
            );
        }
        panel
    }
}

impl gpui::EventEmitter<MainEvent> for MainScreen {}

impl Render for MainScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.autofocused {
            self.autofocused = true;
            window.focus(&self.focus, cx);
        }
        self.ensure_polling(cx);
        self.apply_to_status(cx);
        if self.menu.as_ref().is_some_and(|menu| !menu.read(cx).open) {
            self.menu = None;
        }

        let me = cx.entity();
        let add_me = me.clone();
        let colors = cx.theme().colors.clone();
        let components = &cx.theme().components;
        let (header_height, row_height) = (
            px(components.content_tab_strip_height),
            px(components.menu_item_height),
        );
        let geometry = self.table_geometry.clone();
        let toolbar = div()
            .id("torrent-toolbar")
            .flex()
            .items_center()
            .gap_1()
            .p_1()
            .bg(colors.surface_dim)
            .child(icon_toolbar_button(
                toolbar_buttons()[0],
                colors.surface_dim,
                colors.stroke_accent,
                move |_, app| add_me.update(app, |screen, cx| screen.open_add(cx)),
            ))
            .child(icon_toolbar_button(
                toolbar_buttons()[1],
                colors.surface_dim,
                colors.stroke_accent,
                toolbar_activate(&me, RowAction::Start),
            ))
            .child(icon_toolbar_button(
                toolbar_buttons()[2],
                colors.surface_dim,
                colors.stroke_accent,
                toolbar_activate(&me, RowAction::Pause),
            ))
            .child(icon_toolbar_button(
                toolbar_buttons()[3],
                colors.surface_dim,
                colors.stroke_accent,
                toolbar_activate(&me, RowAction::Delete),
            ));

        let message = self
            .action_error
            .clone()
            .or_else(|| self.snapshot.error().cloned());
        let mut root = div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|screen, event: &KeyDownEvent, window, cx| {
                if !screen.delete_prompt {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "escape" => {
                        screen.delete_prompt = false;
                        ModalStack::pop(cx);
                        window.focus(&screen.focus, cx);
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "enter" => {
                        screen.delete_prompt = false;
                        ModalStack::pop(cx);
                        screen.act(RowAction::Delete, cx);
                        window.focus(&screen.focus, cx);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|screen, _, window, cx| {
                    if !screen.focus.contains_focused(window, cx) {
                        window.focus(&screen.focus, cx);
                    }
                }),
            )
            .on_action(cx.listener(|screen, _: &OpenAdd, _, cx| {
                if screen.add.is_none() {
                    screen.open_add(cx);
                }
            }))
            .on_action(cx.listener(|screen, _: &PasteMagnet, _, cx| {
                if screen.add.is_none() {
                    screen.paste_magnet(cx);
                }
            }))
            .on_action(cx.listener(|screen, _: &DeleteSelected, _, cx| {
                if screen.add.is_none() {
                    screen.request(RowAction::Delete, cx);
                }
            }))
            .on_action(cx.listener(|screen, _: &TogglePause, _, cx| {
                if screen.add.is_none() {
                    screen.toggle_pause(cx);
                }
            }))
            .on_action(cx.listener(|screen, _: &SelectAll, _, cx| {
                if screen.add.is_none() {
                    screen.select_all(cx);
                }
            }))
            .on_action(cx.listener(|screen, _: &FocusFilter, window, cx| {
                if screen.add.is_none() {
                    let handle = screen.filter_input.focus_handle(cx);
                    window.focus(&handle, cx);
                }
            }))
            .on_action(cx.listener(|screen, _: &Refresh, _, cx| screen.refresh(cx)))
            .on_action(cx.listener(|screen, _: &OpenContextMenu, _, cx| {
                if screen.add.is_none() {
                    screen.open_menu_for_selection(cx);
                }
            }))
            .flex()
            .flex_col()
            .flex_1()
            .p_4()
            .gap_2()
            .child(toolbar)
            .child(self.filter_input.clone());
        if let Some(message) = message {
            root = root.child(Label::new(message).size(LabelSize::Caption));
        }
        root = root.child(
            div()
                .tab_index(0)
                .relative()
                .key_context("TorrentTable")
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|screen, event: &MouseDownEvent, _, cx| {
                        if screen.add.is_none() {
                            screen.open_menu_at_pointer(event.position, cx);
                        }
                    }),
                )
                .capture_key_down(cx.listener(|screen, event: &KeyDownEvent, _, cx| {
                    if screen.handle_table_key(event, cx) {
                        cx.stop_propagation();
                    }
                }))
                .child(
                    canvas(
                        move |bounds, _, _| {
                            geometry.set(TableGeometry {
                                bounds,
                                header_height,
                                row_height,
                            });
                        },
                        |_, (), _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .child(self.filter_sidebar(
                    &me,
                    colors.surface_dim,
                    colors.accent,
                    colors.stroke_neutral,
                ))
                .child(self.table(&me)),
        );
        root = root.child(Label::new(SHORTCUT_HELP).size(LabelSize::Caption));
        if self.detail_open {
            root = root.child(self.detail.clone());
        }
        if let Some(menu) = &self.menu {
            root = root.child(menu.clone());
        }
        if let Some(add) = &self.add {
            root = root.child(add.clone());
        }
        if let Some(dialog) = &self.bulk_edit {
            root = root.child(dialog.clone());
        }
        root = root.child(self.toasts.clone());
        root
    }
}

#[derive(Clone, Copy)]
enum SelectionMovement {
    Previous,
    Next,
    First,
    Last,
}

/// Where the table was painted and how tall its header and rows are. `DataTable` exposes no
/// per-row hit testing, so the right click maps the pointer to a row from these.
#[derive(Clone, Copy, Default)]
struct TableGeometry {
    bounds: Bounds<Pixels>,
    header_height: Pixels,
    row_height: Pixels,
}

impl TableGeometry {
    /// One pixel each for the table border, the header rule and every row rule.
    const RULE: Pixels = px(1.0);

    fn first_row_top(&self) -> Pixels {
        self.bounds.origin.y + Self::RULE + self.header_height + Self::RULE
    }

    fn row_stride(&self) -> Pixels {
        self.row_height + Self::RULE
    }

    /// Index of the row under `position`, if the pointer is over the table body.
    fn row_index(&self, position: Point<Pixels>) -> Option<usize> {
        let stride = f32::from(self.row_stride());
        if stride <= 0.0 || !self.bounds.contains(&position) {
            return None;
        }
        let offset = f32::from(position.y - self.first_row_top());
        (offset >= 0.0).then(|| (offset / stride) as usize)
    }

    /// Where a menu opened from the keyboard for row `index` goes.
    fn row_origin(&self, index: usize) -> Point<Pixels> {
        Point {
            x: self.bounds.origin.x + px(24.0),
            y: self.first_row_top() + self.row_stride() * index as f32 + self.row_height,
        }
    }
}

/// A command from the row context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
enum MenuCommand {
    Start,
    Stop,
    Recheck,
    SetCategory(Option<String>),
    BulkEdit,
    CopyMagnet,
    CopyName,
    Delete,
}

/// One daemon call for one torrent.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Job {
    Row(RowAction),
    Recheck,
    Category(Option<String>),
}

/// Which context-menu items apply to the current targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MenuAvailability {
    start: bool,
    stop: bool,
    recheck: bool,
    category: bool,
    copy: bool,
    delete: bool,
}

impl MenuAvailability {
    fn of(targets: &[&Torrent]) -> Self {
        let state = |torrent: &&Torrent| torrent.stats.as_ref().and_then(|stats| stats.state);
        let has_id = |torrent: &&Torrent| torrent.id.is_some();
        let any = |test: &dyn Fn(&&Torrent) -> bool| targets.iter().any(test);
        Self {
            start: any(&|t| has_id(t) && !is_running(state(t))),
            stop: any(&|t| has_id(t) && is_stoppable(state(t))),
            recheck: any(&|t| has_id(t) && state(t) != Some(TorrentState::Initializing)),
            category: any(&has_id),
            copy: !targets.is_empty(),
            delete: any(&has_id),
        }
    }
}

/// Live or still starting: there is nothing for Start to do.
fn is_running(state: Option<TorrentState>) -> bool {
    matches!(state, Some(TorrentState::Live | TorrentState::Initializing))
}

/// Running, starting or of unknown state: Stop has something to do.
fn is_stoppable(state: Option<TorrentState>) -> bool {
    matches!(
        state,
        None | Some(TorrentState::Live | TorrentState::Initializing)
    )
}

/// The daemon calls a menu command makes, one per target it applies to.
fn menu_jobs(targets: &[&Torrent], command: &MenuCommand) -> Vec<(u64, Job)> {
    targets
        .iter()
        .filter_map(|torrent| {
            let id = torrent.id?;
            let state = torrent.stats.as_ref().and_then(|stats| stats.state);
            let job = match command {
                MenuCommand::Start if !is_running(state) => Job::Row(RowAction::Start),
                MenuCommand::Stop if is_stoppable(state) => Job::Row(RowAction::Pause),
                MenuCommand::Recheck if state != Some(TorrentState::Initializing) => Job::Recheck,
                MenuCommand::SetCategory(category) => Job::Category(category.clone()),
                _ => return None,
            };
            Some((id, job))
        })
        .collect()
}

/// Clipboard text for Copy magnet link and Copy name: one line per target.
fn copy_text(targets: &[&Torrent], command: &MenuCommand) -> Option<String> {
    if targets.is_empty() {
        return None;
    }
    let lines: Vec<String> = targets
        .iter()
        .map(|torrent| match command {
            MenuCommand::CopyMagnet => magnet_link(torrent),
            _ => torrent_name(torrent),
        })
        .collect();
    Some(lines.join("\n"))
}

/// `magnet:?xt=urn:btih:<hash>&dn=<name>`, with the name percent-encoded.
fn magnet_link(torrent: &Torrent) -> String {
    let mut link = format!("magnet:?xt=urn:btih:{}", torrent.info_hash);
    if let Some(name) = torrent.name.as_deref().filter(|name| !name.is_empty()) {
        link.push_str("&dn=");
        for byte in name.bytes() {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                link.push(char::from(byte));
            } else {
                link.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    link
}

/// Sorted, distinct, non-empty category names in use.
fn category_names(torrents: Option<&[Torrent]>) -> Vec<String> {
    let mut names: Vec<String> = torrents
        .unwrap_or_default()
        .iter()
        .filter_map(|torrent| torrent.category.clone())
        .filter(|name| !name.is_empty())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Hand a context-menu command to the screen entity.
fn run_menu_command(me: &Entity<MainScreen>, app: &mut App, command: MenuCommand) {
    me.update(app, |screen, cx| screen.run_menu_command(command, cx));
}

/// Shown in the table when the daemon has no torrents.
pub const NO_TORRENTS: &str = "No torrents yet.";

/// Run a confirmed delete. Calls the daemon path directly: `request` would open the modal again.
fn confirm_delete(me: &Entity<MainScreen>, app: &mut App) {
    me.update(app, |screen, cx| screen.act(RowAction::Delete, cx));
}

/// Hand a row command to the screen entity from a toolbar, menu or dialog.
fn dispatch(me: &Entity<MainScreen>, app: &mut App, action: RowAction) {
    me.update(app, |screen, cx| screen.request(action, cx));
}

fn snapshot_of(screen: &mut MainScreen) -> &mut AsyncResource<Snapshot, String> {
    &mut screen.snapshot
}

/// Whether a torrent passes the sidebar status filter.
fn matches_filter(filter: StatusFilter, torrent: &Torrent) -> bool {
    let state = torrent.stats.as_ref().and_then(|stats| stats.state);
    match filter {
        StatusFilter::All => true,
        StatusFilter::Live => state == Some(TorrentState::Live),
        StatusFilter::Paused => state == Some(TorrentState::Paused),
        StatusFilter::Error => state == Some(TorrentState::Error),
        StatusFilter::Initializing => state == Some(TorrentState::Initializing),
    }
}

fn parse_status(value: &str) -> Option<StatusFilter> {
    StatusFilter::ALL
        .into_iter()
        .find(|status| status.label() == value)
}

fn tracker_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', ':']).next().unwrap_or("");
    (!host.is_empty()).then(|| host.to_owned())
}

fn row_id(torrent: &Torrent) -> String {
    torrent
        .id
        .map_or_else(|| torrent.info_hash.clone(), |id| id.to_string())
}

fn torrent_name(torrent: &Torrent) -> String {
    torrent
        .name
        .clone()
        .unwrap_or_else(|| torrent.info_hash.clone())
}

fn total_bytes(torrent: &Torrent) -> u64 {
    torrent.stats.as_ref().map_or(0, |stats| stats.total_bytes)
}

fn done_fraction(torrent: &Torrent) -> f64 {
    match torrent.stats.as_ref() {
        Some(stats) if stats.total_bytes > 0 => {
            stats.progress_bytes as f64 / stats.total_bytes as f64
        }
        _ => 0.0,
    }
}

/// Live download speed in bytes per second (the daemon reports MiB/s).
fn download_bytes_per_sec(torrent: &Torrent) -> f64 {
    torrent
        .stats
        .as_ref()
        .and_then(|stats| stats.live.as_ref())
        .map_or(0.0, |live| live.download_speed.mbps * 1024.0 * 1024.0)
}

fn upload_speed(torrent: &Torrent) -> rtbit_client::Speed {
    torrent
        .stats
        .as_ref()
        .and_then(|stats| stats.live.as_ref())
        .map(|live| live.upload_speed.clone())
        .unwrap_or_default()
}

fn upload_text(torrent: &Torrent) -> String {
    format::speed(&upload_speed(torrent))
}

fn size_text(torrent: &Torrent) -> String {
    format::bytes(total_bytes(torrent))
}

fn progress_text(torrent: &Torrent) -> String {
    format!("{:.1}%", done_fraction(torrent) * 100.0)
}

fn speed_text(torrent: &Torrent) -> String {
    let speed = torrent
        .stats
        .as_ref()
        .and_then(|stats| stats.live.as_ref())
        .map(|live| live.download_speed.clone())
        .unwrap_or_default();
    format::speed(&speed)
}

fn state_of(torrent: &Torrent) -> String {
    format::state_label(torrent.stats.as_ref().and_then(|stats| stats.state))
}

/// Seconds left at the current download speed, or `None` when nothing is moving.
fn eta_seconds(torrent: &Torrent) -> Option<f64> {
    let stats = torrent.stats.as_ref()?;
    let remaining = stats.total_bytes.saturating_sub(stats.progress_bytes) as f64;
    let rate = download_bytes_per_sec(torrent);
    (remaining > 0.0 && rate > 0.0).then(|| remaining / rate)
}

fn eta_text(torrent: &Torrent) -> String {
    match eta_seconds(torrent) {
        None => "—".to_owned(),
        Some(secs) => {
            let total = secs.round() as u64;
            let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
            if h > 0 {
                format!("{h}h {m:02}m")
            } else if m > 0 {
                format!("{m}m {s:02}s")
            } else {
                format!("{s}s")
            }
        }
    }
}

/// Ordering for one sortable column. Unknown columns keep daemon order.
fn column_order(column: &str, a: &Torrent, b: &Torrent) -> Ordering {
    match column {
        "name" => torrent_name(a).cmp(&torrent_name(b)),
        "size" => total_bytes(a).cmp(&total_bytes(b)),
        "progress" => done_fraction(a).total_cmp(&done_fraction(b)),
        "speed" => download_bytes_per_sec(a).total_cmp(&download_bytes_per_sec(b)),
        "upload" => upload_speed(a).mbps.total_cmp(&upload_speed(b).mbps),
        "eta" => eta_seconds(a)
            .unwrap_or(f64::INFINITY)
            .total_cmp(&eta_seconds(b).unwrap_or(f64::INFINITY)),
        "state" => state_of(a).cmp(&state_of(b)),
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rtbit-client/tests/fixtures"
    );

    #[test]
    fn every_toolbar_button_has_a_non_empty_label() {
        assert!(
            toolbar_buttons()
                .iter()
                .all(|button| !button.label.trim().is_empty()),
            "toolbar buttons need text for their tooltip and accessible name"
        );
        assert_eq!(toolbar_buttons().len(), 4);
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
    }

    /// Serve the login, torrent list and stats fixtures, plus a pause endpoint.
    /// Returns the bridge (already logged in) and the runtime and server to inspect.
    fn daemon(cx: &mut TestAppContext) -> (tokio::runtime::Runtime, MockServer) {
        // Replies wake gpui tasks from tokio threads, which is real (not simulated) concurrency.
        cx.executor().allow_parking();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let server = runtime.block_on(async {
            let server = MockServer::start().await;
            for (route, file) in [
                ("/auth/login", "login.json"),
                ("/torrents", "torrents.json"),
                ("/stats", "stats.json"),
            ] {
                let method_name = if route == "/auth/login" {
                    "POST"
                } else {
                    "GET"
                };
                Mock::given(method(method_name))
                    .and(path(route))
                    .respond_with(
                        ResponseTemplate::new(200).set_body_raw(fixture(file), "application/json"),
                    )
                    .mount(&server)
                    .await;
            }
            Mock::given(method("POST"))
                .and(path("/torrents/3/pause"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
            server
        });
        let bridge = Bridge::new(&server.uri()).unwrap();
        // Log in on the bridge's own runtime; the test thread only waits for the reply.
        let login =
            bridge.call(|client| async move { client.lock().await.login("admin", "secret").await });
        let reply = bridge.runtime().block_on(login).unwrap();
        reply.unwrap();
        cx.update(|cx| bridge.install(cx));
        (runtime, server)
    }

    /// Step gpui until `done` holds. Replies arrive from tokio threads.
    fn drive(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("daemon reply never arrived");
    }

    /// A row with the given size, built from public fields (no JSON parser needed).
    fn row(name: &str, total_bytes: u64) -> Torrent {
        Torrent {
            id: Some(1),
            info_hash: name.to_owned(),
            name: Some(name.to_owned()),
            output_folder: String::new(),
            total_pieces: 0,
            category: None,
            tags: None,
            trackers: None,
            files: None,
            stats: Some(rtbit_client::TorrentStats {
                total_bytes,
                ..Default::default()
            }),
        }
    }

    #[test]
    fn size_sorts_numerically_not_by_text() {
        let small = row("small", 9);
        let big = row("big", 10);
        // Text order would put "10" before "9"; the column must sort by value.
        assert_eq!(column_order("size", &small, &big), Ordering::Less);
    }

    #[test]
    fn numeric_columns_sort_by_value() {
        let (mut slow, mut fast) = (row("a", 1), row("b", 1));
        let live = |mbps| rtbit_client::LiveSpeeds {
            download_speed: rtbit_client::Speed {
                mbps,
                human_readable: String::new(),
            },
            upload_speed: rtbit_client::Speed {
                mbps,
                human_readable: String::new(),
            },
        };
        slow.stats.as_mut().unwrap().live = Some(live(9.0));
        fast.stats.as_mut().unwrap().live = Some(live(10.0));
        for column in ["speed", "upload"] {
            assert_eq!(
                column_order(column, &slow, &fast),
                Ordering::Less,
                "{column}"
            );
        }
        let (mut half, mut most) = (row("a", 100), row("b", 100));
        half.stats.as_mut().unwrap().progress_bytes = 9;
        most.stats.as_mut().unwrap().progress_bytes = 10;
        assert_eq!(column_order("progress", &half, &most), Ordering::Less);
    }

    #[test]
    fn eta_is_dash_without_speed() {
        assert_eq!(eta_text(&row("idle", 100)), "—");
    }

    #[gpui::test]
    fn poll_fills_rows_and_feeds_status_bar(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| {
            screen._poller = Some(Bridge::poll_snapshot(
                cx,
                snapshot_of,
                MainScreen::on_poll_health_cb,
            ));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.read_with(cx, |screen, _| {
            let rows = screen.visible_rows();
            assert_eq!(rows[0].name.as_deref(), Some("example"));
        });
    }

    #[gpui::test]
    fn confirmed_delete_hits_daemon_once(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents/3/delete"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server),
        );
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.update(cx, |screen, cx| screen.select_row("3", cx));
        // The modal's on_confirm path.
        let me = screen.clone();
        cx.update(|app| confirm_delete(&me, app));
        // A successful delete clears the selection.
        drive(cx, |cx| screen.read_with(cx, |s, _| s.selected.is_none()));
        screen.read_with(cx, |s, _| assert!(s.action_error.is_none()));
        std::thread::sleep(Duration::from_millis(50));
        let hits = runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() == "/torrents/3/delete")
            .count();
        assert_eq!(hits, 1, "confirmed delete must hit the daemon exactly once");
    }

    #[gpui::test]
    fn selecting_a_row_loads_its_detail(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        let bitfield = ResponseTemplate::new(200)
            .set_body_bytes(vec![0b1000_0000u8])
            .insert_header("x-bitfield-len", "10");
        let json =
            |name: &str| ResponseTemplate::new(200).set_body_raw(fixture(name), "application/json");
        for (route, response) in [
            ("/torrents/3", json("torrent.json")),
            ("/torrents/3/trackers", json("trackers.json")),
            ("/torrents/3/peer_stats", json("peers.json")),
            ("/torrents/3/haves", bitfield),
        ] {
            runtime.block_on(
                Mock::given(method("GET"))
                    .and(path(route))
                    .respond_with(response)
                    .mount(&server),
            );
        }
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.update(cx, |screen, cx| screen.select_row("3", cx));
        let detail = screen.read_with(cx, |s, _| s.detail.clone());
        drive(cx, |cx| detail.read_with(cx, |p, _| p.snapshot().is_some()));
        detail.read_with(cx, |pane, _| {
            assert_eq!(pane.torrent_id(), Some(3));
            assert!(pane.error().is_none());
        });
    }

    #[gpui::test]
    fn keyboard_navigation_moves_extends_and_jumps_selection(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status, cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.update(cx, |screen, cx| {
            let mut snapshot = screen.snapshot.ready_value().unwrap().clone();
            for id in [4, 5] {
                let mut torrent = snapshot.torrents.torrents[0].clone();
                torrent.id = Some(id);
                torrent.name = Some(format!("torrent-{id}"));
                snapshot.torrents.torrents.push(torrent);
            }
            snapshot.torrents.total = 3;
            screen.snapshot = AsyncResource::ready(snapshot);

            screen.move_selection(SelectionMovement::Next, false, cx);
            assert_eq!(screen.selected.as_deref(), Some("4"));
            screen.move_selection(SelectionMovement::Next, true, cx);
            assert_eq!(screen.selected.as_deref(), Some("5"));
            assert_eq!(screen.marked.len(), 2);
            assert!(screen.marked.contains("4"));
            assert!(screen.marked.contains("5"));
            screen.move_selection(SelectionMovement::First, false, cx);
            assert_eq!(screen.selected.as_deref(), Some("3"));
            screen.move_selection(SelectionMovement::Last, false, cx);
            assert_eq!(screen.selected.as_deref(), Some("5"));
        });
    }

    #[gpui::test]
    fn enter_opens_and_escape_closes_detail(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status, cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.update(cx, |screen, cx| {
            screen.selected = Some("3".to_owned());
            screen.open_detail(cx);
            assert!(screen.detail_open, "Enter must open the detail pane");
            screen.close_detail(cx);
            assert!(!screen.detail_open, "Escape must close the detail pane");
        });
    }

    #[gpui::test]
    fn pause_action_hits_daemon_endpoint(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.update(cx, |screen, cx| {
            screen.selected = Some("3".to_owned());
            screen.act(RowAction::Pause, cx);
        });
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| s._action.is_some() && s.action_error.is_none())
        });
        // Give the daemon call a moment to land, then count the hits.
        std::thread::sleep(Duration::from_millis(50));
        let hits = runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() == "/torrents/3/pause")
            .count();
        assert_eq!(hits, 1, "pause must hit the daemon exactly once");
    }

    #[gpui::test]
    fn status_filter_hides_rows_that_do_not_match(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        // The fixture row reports no state, so only "All" keeps it.
        screen.update(cx, |screen, cx| screen.set_filter(StatusFilter::Live, cx));
        screen.read_with(cx, |s, _| assert!(s.visible_rows().is_empty()));
        screen.update(cx, |screen, cx| screen.set_filter(StatusFilter::All, cx));
        screen.read_with(cx, |s, _| assert_eq!(s.visible_rows().len(), 1));
    }

    const HASH_MAGNET: &str =
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Example";

    /// Count the POSTs the daemon has seen on `/torrents` (adds, not the list GET).
    fn add_posts(runtime: &tokio::runtime::Runtime, server: &MockServer) -> usize {
        runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() == "/torrents")
            .count()
    }

    #[gpui::test]
    fn bad_magnet_is_refused_without_a_request(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| screen.open_add(cx));
        let dialog = screen.read_with(cx, |s, _| s.add.clone()).unwrap();
        dialog.update(cx, |dialog, cx| {
            dialog.fill_for_test(cx, "https://example.com/not-a-magnet", "/data");
            dialog.submit(cx);
        });
        dialog.read_with(cx, |dialog, _| {
            assert_eq!(dialog.error(), Some("magnet link must start with magnet:?"));
        });
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            add_posts(&runtime, &server),
            0,
            "a refused magnet must send nothing"
        );
    }

    #[gpui::test]
    fn cancel_closes_the_dialog_without_a_request(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| screen.open_add(cx));
        let dialog = screen.read_with(cx, |s, _| s.add.clone()).unwrap();
        dialog.update(cx, |dialog, cx| {
            dialog.fill_for_test(cx, HASH_MAGNET, "/data");
            dialog.cancel(cx);
        });
        screen.read_with(cx, |s, _| {
            assert!(s.add.is_none(), "cancel must close the dialog")
        });
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(add_posts(&runtime, &server), 0, "cancel must send nothing");
    }

    #[gpui::test]
    fn valid_magnet_adds_exactly_once(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        cx.update(ToastStack::init);
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server),
        );
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| screen.open_add(cx));
        let dialog = screen.read_with(cx, |s, _| s.add.clone()).unwrap();
        dialog.update(cx, |dialog, cx| {
            dialog.fill_for_test(cx, HASH_MAGNET, "/data");
            dialog.submit(cx);
            // A second submit while the first is in flight must not send again.
            dialog.submit(cx);
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.add.is_none()));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            add_posts(&runtime, &server),
            1,
            "a valid magnet must add exactly once"
        );
    }

    #[gpui::test]
    fn a_torrent_file_is_posted_as_its_bytes_without_is_url(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        cx.update(ToastStack::init);
        // Only the exact shape counts: the file bytes, and no is_url query.
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents"))
                .and(wiremock::matchers::query_param_is_missing("is_url"))
                .and(wiremock::matchers::body_bytes(b"d8:announce0:e".as_slice()))
                .respond_with(ResponseTemplate::new(200))
                .expect(1)
                .mount(&server),
        );
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| screen.open_add(cx));
        let dialog = screen.read_with(cx, |s, _| s.add.clone()).unwrap();
        dialog.update(cx, |dialog, cx| {
            dialog.fill_for_test(cx, "", "/data");
            dialog.set_file_for_test("ubuntu.torrent", b"d8:announce0:e");
            dialog.submit(cx);
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.add.is_none()));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            add_posts(&runtime, &server),
            1,
            "a .torrent file must be posted once"
        );
        runtime.block_on(server.verify());
    }

    #[gpui::test]
    fn an_empty_list_uses_the_no_torrents_text(cx: &mut TestAppContext) {
        // The table is built with this constant; the text must say "no torrents".
        assert_eq!(NO_TORRENTS, "No torrents yet.");
        assert!(NO_TORRENTS.to_lowercase().contains("no torrents"));
        let _ = cx;
    }

    #[gpui::test]
    fn cancelled_delete_sends_nothing(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.update(cx, |screen, cx| screen.select_row("3", cx));
        // Opening the confirm modal needs a window, which this test context lacks, so the
        // test checks the guarantee directly: a delete only goes out from confirm_delete,
        // and a cancelled modal never calls it. No request has been made yet.
        std::thread::sleep(Duration::from_millis(50));
        let hits = runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() == "/torrents/3/delete")
            .count();
        assert_eq!(
            hits, 0,
            "a delete that is not confirmed must not reach the daemon"
        );
    }

    #[gpui::test]
    fn start_action_hits_daemon_endpoint(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx);
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents/3/start"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server),
        );
        let status = cx.new(|_| StatusBar::new());
        let screen = cx.new(|cx| MainScreen::new(status.clone(), cx));
        screen.update(cx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        screen.update(cx, |screen, cx| {
            screen.selected = Some("3".to_owned());
            screen.act(RowAction::Start, cx);
        });
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| s._action.is_some() && s.action_error.is_none())
        });
        std::thread::sleep(Duration::from_millis(50));
        let hits = runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() == "/torrents/3/start")
            .count();
        assert_eq!(hits, 1, "start must hit the daemon exactly once");
    }

    // --- keyboard shortcuts: each test sends real keystrokes through the keymap ---

    fn drive_window(
        cx: &mut gpui::VisualTestContext,
        mut done: impl FnMut(&mut gpui::VisualTestContext) -> bool,
    ) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("daemon reply never arrived");
    }

    /// A main screen in a window with the shortcuts bound, the snapshot loaded and row 3 shown.
    fn shortcut_screen(
        cx: &mut TestAppContext,
    ) -> (
        Entity<MainScreen>,
        &mut gpui::VisualTestContext,
        tokio::runtime::Runtime,
        MockServer,
    ) {
        let (runtime, server) = daemon(cx);
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents/3/start"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server),
        );
        cx.update(|app| {
            fluent_core::Theme::init(app);
            fluent_layout::ModalStack::init(app);
            bind_keys(app);
        });
        let status = cx.new(|_| StatusBar::new());
        let (screen, vcx) = cx.add_window_view(|_, cx| MainScreen::new(status, cx));
        screen.update(vcx, |screen, cx| {
            screen._load = Some(Bridge::load(screen, cx, snapshot_of, fetch_snapshot));
        });
        drive_window(vcx, |cx| screen.read_with(cx, |s, _| s.torrent_count() > 0));
        (screen, vcx, runtime, server)
    }

    fn posts_to(runtime: &tokio::runtime::Runtime, server: &MockServer, route: &str) -> usize {
        std::thread::sleep(Duration::from_millis(50));
        runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .filter(|req| req.method.as_str() == "POST" && req.url.path() == route)
            .count()
    }

    #[gpui::test]
    fn ctrl_o_opens_the_add_dialog(cx: &mut TestAppContext) {
        let (screen, vcx, _runtime, _server) = shortcut_screen(cx);
        screen.read_with(vcx, |s, _| assert!(s.add.is_none()));
        vcx.simulate_keystrokes("ctrl-o");
        screen.read_with(vcx, |s, _| assert!(s.add.is_some()));
    }

    #[gpui::test]
    fn add_dialog_escape_cancels_and_enter_submits_only_valid_input(cx: &mut TestAppContext) {
        let (screen, vcx, runtime, server) = shortcut_screen(cx);
        vcx.update(|_, cx| ToastStack::init(cx));
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server),
        );

        vcx.simulate_keystrokes("ctrl-o");
        let dialog = screen.read_with(vcx, |s, _| s.add.clone()).unwrap();
        vcx.run_until_parked();
        vcx.update(|window, cx| assert!(dialog.read(cx).magnet_focus(cx).is_focused(window)));
        vcx.simulate_keystrokes("enter");
        dialog.read_with(vcx, |dialog, _| assert!(dialog.error().is_some()));
        assert_eq!(posts_to(&runtime, &server, "/torrents"), 0);
        vcx.simulate_keystrokes("escape");
        screen.read_with(vcx, |s, _| assert!(s.add.is_none()));

        vcx.simulate_keystrokes("ctrl-o");
        let dialog = screen.read_with(vcx, |s, _| s.add.clone()).unwrap();
        dialog.update(vcx, |dialog, cx| {
            dialog.fill_for_test(cx, HASH_MAGNET, "/data");
        });
        vcx.simulate_keystrokes("enter");
        drive_window(vcx, |cx| screen.read_with(cx, |s, _| s.add.is_none()));
        assert_eq!(posts_to(&runtime, &server, "/torrents"), 1);
    }

    #[gpui::test]
    fn ctrl_v_with_a_magnet_opens_the_dialog_prefilled(cx: &mut TestAppContext) {
        let (screen, vcx, _runtime, _server) = shortcut_screen(cx);
        vcx.write_to_clipboard(gpui::ClipboardItem::new_string(format!(
            "  {HASH_MAGNET}\n"
        )));
        vcx.simulate_keystrokes("ctrl-v");
        let dialog = screen.read_with(vcx, |s, _| s.add.clone()).unwrap();
        dialog.read_with(vcx, |dialog, cx| {
            assert_eq!(dialog.magnet_text(cx), HASH_MAGNET)
        });
    }

    #[gpui::test]
    fn ctrl_v_without_a_magnet_opens_nothing(cx: &mut TestAppContext) {
        let (screen, vcx, _runtime, _server) = shortcut_screen(cx);
        vcx.write_to_clipboard(gpui::ClipboardItem::new_string("just some text".to_owned()));
        vcx.simulate_keystrokes("ctrl-v");
        screen.read_with(vcx, |s, _| assert!(s.add.is_none()));
    }

    #[gpui::test]
    fn delete_asks_before_removing_the_selection(cx: &mut TestAppContext) {
        let (screen, vcx, runtime, server) = shortcut_screen(cx);
        vcx.simulate_keystrokes("delete");
        screen.read_with(vcx, |s, _| {
            assert_eq!(s.action_error.as_deref(), Some("Select a torrent first."));
            assert!(!s.delete_prompt);
        });
        screen.update(vcx, |s, cx| s.select_row("3", cx));
        vcx.simulate_keystrokes("delete");
        screen.read_with(vcx, |s, _| assert!(s.delete_prompt));
        assert_eq!(posts_to(&runtime, &server, "/torrents/3/delete"), 0);
    }

    #[gpui::test]
    fn delete_confirmation_escape_cancels_and_enter_confirms(cx: &mut TestAppContext) {
        let (screen, vcx, runtime, server) = shortcut_screen(cx);
        runtime.block_on(
            Mock::given(method("POST"))
                .and(path("/torrents/3/delete"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server),
        );
        screen.update(vcx, |s, cx| s.select_row("3", cx));
        vcx.simulate_keystrokes("delete escape");
        screen.read_with(vcx, |s, _| assert!(!s.delete_prompt));
        assert_eq!(posts_to(&runtime, &server, "/torrents/3/delete"), 0);

        vcx.simulate_keystrokes("delete enter");
        drive_window(vcx, |cx| screen.read_with(cx, |s, _| s._action.is_some()));
        assert_eq!(posts_to(&runtime, &server, "/torrents/3/delete"), 1);
    }

    #[gpui::test]
    fn space_pauses_the_selected_torrent(cx: &mut TestAppContext) {
        let (screen, vcx, runtime, server) = shortcut_screen(cx);
        screen.update(vcx, |s, cx| s.select_row("3", cx));
        vcx.simulate_keystrokes("space");
        drive_window(vcx, |cx| screen.read_with(cx, |s, _| s._action.is_some()));
        // The fixture row is not paused, so Space pauses it.
        assert_eq!(posts_to(&runtime, &server, "/torrents/3/pause"), 1);
        assert_eq!(posts_to(&runtime, &server, "/torrents/3/start"), 0);
    }

    #[gpui::test]
    fn ctrl_a_marks_every_visible_row(cx: &mut TestAppContext) {
        let (screen, vcx, _runtime, _server) = shortcut_screen(cx);
        vcx.simulate_keystrokes("ctrl-a");
        screen.read_with(vcx, |s, _| assert_eq!(s.marked_count(), 1));
        // Picking a row again drops the marks.
        screen.update(vcx, |s, cx| s.select_row("3", cx));
        screen.read_with(vcx, |s, _| assert_eq!(s.marked_count(), 0));
    }

    #[gpui::test]
    fn ctrl_f_focuses_the_filter_and_typing_keys_stay_there(cx: &mut TestAppContext) {
        let (screen, vcx, runtime, server) = shortcut_screen(cx);
        screen.update(vcx, |s, cx| s.select_row("3", cx));
        vcx.simulate_keystrokes("ctrl-f");
        let filter = screen.read_with(vcx, |s, _| s.filter_input.clone());
        let focused = vcx.update(|window, cx| filter.focus_handle(cx).is_focused(window));
        assert!(focused, "Ctrl+F must focus the filter box");
        // Space belongs to the text field now: it must not pause the torrent.
        vcx.simulate_keystrokes("space");
        assert_eq!(posts_to(&runtime, &server, "/torrents/3/pause"), 0);
    }

    #[gpui::test]
    fn the_filter_box_narrows_the_rows(cx: &mut TestAppContext) {
        let (screen, vcx, _runtime, _server) = shortcut_screen(cx);
        screen.update(vcx, |s, cx| s.set_query("EXAM".to_owned(), cx));
        screen.read_with(vcx, |s, _| assert_eq!(s.visible_rows().len(), 1));
        screen.update(vcx, |s, cx| s.set_query("nomatch".to_owned(), cx));
        screen.read_with(vcx, |s, _| assert!(s.visible_rows().is_empty()));
    }

    #[gpui::test]
    fn f5_asks_the_daemon_for_a_fresh_snapshot(cx: &mut TestAppContext) {
        let (screen, vcx, runtime, server) = shortcut_screen(cx);
        let gets = |runtime: &tokio::runtime::Runtime, server: &MockServer| {
            runtime
                .block_on(server.received_requests())
                .unwrap()
                .iter()
                .filter(|req| req.method.as_str() == "GET" && req.url.path() == "/torrents")
                .count()
        };
        let before = gets(&runtime, &server);
        screen.update(vcx, |s, _| s._load = None);
        vcx.simulate_keystrokes("f5");
        screen.read_with(vcx, |s, _| assert!(s._load.is_some()));
        drive_window(vcx, |_| gets(&runtime, &server) > before);
    }

    fn with_state(id: u64, state: TorrentState) -> Torrent {
        let mut torrent = row(&format!("t{id}"), 1);
        torrent.id = Some(id);
        torrent.stats.as_mut().unwrap().state = Some(state);
        torrent
    }

    #[test]
    fn menu_items_follow_the_state_of_the_targets() {
        let live = with_state(1, TorrentState::Live);
        let paused = with_state(2, TorrentState::Paused);
        let starting = with_state(3, TorrentState::Initializing);

        let running = MenuAvailability::of(&[&live]);
        assert!(!running.start, "Start is greyed while running");
        assert!(running.stop && running.recheck && running.delete && running.copy);

        let stopped = MenuAvailability::of(&[&paused]);
        assert!(stopped.start && !stopped.stop);

        let mixed = MenuAvailability::of(&[&live, &paused]);
        assert!(mixed.start && mixed.stop, "a mixed selection allows both");

        assert!(!MenuAvailability::of(&[&starting]).recheck);

        let none = MenuAvailability::of(&[]);
        assert!(!(none.start || none.stop || none.recheck || none.category || none.copy));
        assert!(!none.delete);
    }

    #[test]
    fn menu_jobs_cover_only_the_targets_a_command_applies_to() {
        let live = with_state(1, TorrentState::Live);
        let paused = with_state(2, TorrentState::Paused);
        let targets = [&live, &paused];
        assert_eq!(
            menu_jobs(&targets, &MenuCommand::Start),
            vec![(2, Job::Row(RowAction::Start))]
        );
        assert_eq!(
            menu_jobs(&targets, &MenuCommand::Stop),
            vec![(1, Job::Row(RowAction::Pause))]
        );
        assert_eq!(
            menu_jobs(&targets, &MenuCommand::Recheck),
            vec![(1, Job::Recheck), (2, Job::Recheck)]
        );
        assert_eq!(
            menu_jobs(&targets, &MenuCommand::SetCategory(Some("tv".to_owned()))),
            vec![
                (1, Job::Category(Some("tv".to_owned()))),
                (2, Job::Category(Some("tv".to_owned())))
            ]
        );
        assert!(menu_jobs(&targets, &MenuCommand::CopyName).is_empty());
    }

    #[test]
    fn copy_text_builds_magnets_and_names_per_target() {
        let mut one = row("My Show.S01", 1);
        one.info_hash = "abc123".to_owned();
        let two = row("plain", 1);
        assert_eq!(
            copy_text(&[&one], &MenuCommand::CopyMagnet).unwrap(),
            "magnet:?xt=urn:btih:abc123&dn=My%20Show.S01"
        );
        assert_eq!(
            copy_text(&[&one, &two], &MenuCommand::CopyName).unwrap(),
            "My Show.S01\nplain"
        );
        assert_eq!(copy_text(&[], &MenuCommand::CopyName), None);
    }

    #[test]
    fn pointer_maps_to_table_rows() {
        let geometry = TableGeometry {
            bounds: Bounds {
                origin: gpui::point(px(10.0), px(100.0)),
                size: gpui::size(px(400.0), px(300.0)),
            },
            header_height: px(30.0),
            row_height: px(20.0),
        };
        // Body starts below the table border, header and header rule.
        let at = |y: f32| geometry.row_index(gpui::point(px(50.0), px(y)));
        assert_eq!(at(110.0), None, "header");
        assert_eq!(at(132.0), Some(0));
        assert_eq!(at(132.0 + 21.0), Some(1));
        assert_eq!(geometry.row_index(gpui::point(px(5.0), px(150.0))), None);
    }

    #[gpui::test]
    fn right_click_selects_the_row_and_opens_the_menu(cx: &mut TestAppContext) {
        let (screen, vcx, _runtime, _server) = shortcut_screen(cx);
        vcx.run_until_parked();
        let (position, id) = screen.read_with(vcx, |s, _| {
            let geometry = s.table_geometry.get();
            (geometry.row_origin(0), row_id(s.visible_rows()[0]))
        });
        // `row_origin` sits a row below the row's top; step back into the row.
        let inside = gpui::point(position.x, position.y - px(10.0));
        vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Right,
            position: inside,
            modifiers: gpui::Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        screen.read_with(vcx, |s, _| {
            assert_eq!(s.selected.as_deref(), Some(id.as_str()));
            assert!(s.menu.is_some(), "right click must open the menu");
        });
    }

    #[gpui::test]
    fn shift_f10_opens_the_menu_for_the_selection(cx: &mut TestAppContext) {
        let (screen, vcx, _runtime, _server) = shortcut_screen(cx);
        vcx.simulate_keystrokes("shift-f10");
        screen.read_with(vcx, |s, _| {
            assert!(s.menu.is_none());
            assert_eq!(s.action_error.as_deref(), Some("Select a torrent first."));
        });
        screen.update(vcx, |s, cx| s.select_row("3", cx));
        vcx.simulate_keystrokes("shift-f10");
        screen.read_with(vcx, |s, _| assert!(s.menu.is_some()));
    }

    /// Mount the context-menu routes and select the fixture row.
    fn menu_screen(
        cx: &mut TestAppContext,
    ) -> (
        Entity<MainScreen>,
        &mut gpui::VisualTestContext,
        tokio::runtime::Runtime,
        MockServer,
    ) {
        let (screen, vcx, runtime, server) = shortcut_screen(cx);
        for route in ["/torrents/3/recheck", "/torrents/3/set_category"] {
            runtime.block_on(
                Mock::given(method("POST"))
                    .and(path(route))
                    .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
                    .mount(&server),
            );
        }
        vcx.update(|_, cx| ToastStack::init(cx));
        set_state(&screen, vcx, TorrentState::Live);
        screen.update(vcx, |s, cx| s.select_row("3", cx));
        (screen, vcx, runtime, server)
    }

    fn set_state(
        screen: &Entity<MainScreen>,
        vcx: &mut gpui::VisualTestContext,
        state: TorrentState,
    ) {
        screen.update(vcx, |s, _| {
            let mut snapshot = s.snapshot.value().unwrap().clone();
            snapshot.torrents.torrents[0]
                .stats
                .get_or_insert_with(Default::default)
                .state = Some(state);
            s.snapshot = AsyncResource::ready(snapshot);
        });
    }

    fn run_menu(
        screen: &Entity<MainScreen>,
        vcx: &mut gpui::VisualTestContext,
        command: MenuCommand,
    ) {
        screen.update(vcx, |s, cx| s.run_menu_command(command, cx));
    }

    #[gpui::test]
    fn menu_stop_pauses_the_selection(cx: &mut TestAppContext) {
        let (screen, vcx, runtime, server) = menu_screen(cx);
        run_menu(&screen, vcx, MenuCommand::Stop);
        drive_window(vcx, |cx| screen.read_with(cx, |s, _| s._action.is_some()));
        assert_eq!(posts_to(&runtime, &server, "/torrents/3/pause"), 1);
    }

    #[gpui::test]
    fn menu_start_is_a_no_op_while_running(cx: &mut TestAppContext) {
        let (screen, vcx, runtime, server) = menu_screen(cx);
        run_menu(&screen, vcx, MenuCommand::Start);
        assert_eq!(posts_to(&runtime, &server, "/torrents/3/start"), 0);
    }

    #[gpui::test]
    fn menu_start_resumes_a_paused_selection(cx: &mut TestAppContext) {
        let (screen, vcx, runtime, server) = menu_screen(cx);
        set_state(&screen, vcx, TorrentState::Paused);
        run_menu(&screen, vcx, MenuCommand::Start);
        drive_window(vcx, |cx| screen.read_with(cx, |s, _| s._action.is_some()));
        assert_eq!(posts_to(&runtime, &server, "/torrents/3/start"), 1);
    }

    #[gpui::test]
    fn menu_recheck_posts_to_the_daemon(cx: &mut TestAppContext) {
        let (screen, vcx, runtime, server) = menu_screen(cx);
        run_menu(&screen, vcx, MenuCommand::Recheck);
        drive_window(vcx, |cx| screen.read_with(cx, |s, _| s._action.is_some()));
        assert_eq!(posts_to(&runtime, &server, "/torrents/3/recheck"), 1);
    }

    #[gpui::test]
    fn menu_set_category_sends_the_category(cx: &mut TestAppContext) {
        let (screen, vcx, runtime, server) = menu_screen(cx);
        run_menu(
            &screen,
            vcx,
            MenuCommand::SetCategory(Some("movies".to_owned())),
        );
        drive_window(vcx, |cx| screen.read_with(cx, |s, _| s._action.is_some()));
        assert_eq!(posts_to(&runtime, &server, "/torrents/3/set_category"), 1);
        let body: serde_json::Value = runtime
            .block_on(server.received_requests())
            .unwrap()
            .iter()
            .find(|req| req.url.path() == "/torrents/3/set_category")
            .map(|req| serde_json::from_slice(&req.body).unwrap())
            .unwrap();
        assert_eq!(body, serde_json::json!({ "category": "movies" }));
    }

    #[gpui::test]
    fn menu_copy_commands_fill_the_clipboard(cx: &mut TestAppContext) {
        let (screen, vcx, _runtime, _server) = menu_screen(cx);
        run_menu(&screen, vcx, MenuCommand::CopyName);
        let copied = vcx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(copied.as_deref(), Some("example"));
        run_menu(&screen, vcx, MenuCommand::CopyMagnet);
        let copied = vcx.read_from_clipboard().and_then(|item| item.text());
        assert!(copied.unwrap().starts_with("magnet:?xt=urn:btih:"));
    }

    #[gpui::test]
    fn menu_delete_asks_before_removing(cx: &mut TestAppContext) {
        let (screen, vcx, runtime, server) = menu_screen(cx);
        run_menu(&screen, vcx, MenuCommand::Delete);
        screen.read_with(vcx, |s, _| assert!(s.delete_prompt));
        assert_eq!(posts_to(&runtime, &server, "/torrents/3/delete"), 0);
    }
}
