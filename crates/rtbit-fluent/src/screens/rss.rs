//! RSS screen: the feed item list, a Download action for the selected item,
//! an empty state, and an error bar when a call fails.
//!
//! Loads and downloads run on the bridge's tokio runtime. The UI thread only
//! marks the request pending and awaits the reply in a gpui task.

use fluent_core::AsyncResource;
use fluent_layout::{DataColumn, DataRow, DataTable, MessageBar, MessageIntent};
use fluent_primitives::{Button, ButtonAppearance, Label};
use fluent_primitives::{Checkbox, TextInput};
use gpui::{
    AsyncApp, ClickEvent, Context, Entity, IntoElement, Render, SharedString, Task, WeakEntity,
    Window, div, prelude::*,
};
use rtbit_client::{Error, RssItem, RssRule, RssRuleBody, RssRuleEpisode, RssSnapshot};
use rtbit_ui_core::map_error;

use crate::bridge::Bridge;

/// The rule editor's fields. `id` is `None` for a new rule.
#[derive(Clone, Debug, PartialEq)]
pub struct RuleDraft {
    pub id: Option<String>,
    pub name: String,
    pub match_regex: String,
    pub feeds: Vec<String>,
    pub category: String,
    pub priority: i32,
    pub enabled: bool,
    pub track_episodes: bool,
}

impl Default for RuleDraft {
    fn default() -> Self {
        Self {
            id: None,
            name: String::new(),
            match_regex: String::new(),
            feeds: Vec::new(),
            category: String::new(),
            priority: 1,
            enabled: true,
            track_episodes: true,
        }
    }
}

impl RuleDraft {
    /// A draft that starts from an existing rule, keeping every field.
    #[must_use]
    pub fn edit(rule: &RssRule) -> Self {
        Self {
            id: Some(rule.id.clone()),
            name: rule.name.clone(),
            match_regex: rule.match_regex.clone(),
            feeds: rule.feed_names.clone(),
            category: rule.category.clone().unwrap_or_default(),
            priority: rule.priority,
            enabled: rule.enabled,
            track_episodes: rule.track_episodes,
        }
    }
}

/// Client-side checks, the same as the web UI's. The daemon still compiles
/// the regex and refuses a bad pattern with a 400 that `map_error` shows.
pub fn validate_rule(draft: &RuleDraft) -> Result<RssRuleBody, &'static str> {
    let name = draft.name.trim();
    let match_regex = draft.match_regex.trim();
    if name.is_empty() || match_regex.is_empty() {
        return Err("Name and match regex are required.");
    }
    if draft.feeds.is_empty() {
        return Err("Select at least one feed.");
    }
    let category = draft.category.trim();
    Ok(RssRuleBody {
        name: name.to_owned(),
        feed_names: draft.feeds.clone(),
        category: (!category.is_empty()).then(|| category.to_owned()),
        priority: draft.priority,
        match_regex: match_regex.to_owned(),
        enabled: draft.enabled,
        track_episodes: draft.track_episodes,
    })
}

/// The RSS screen. The shell builds one when the RSS nav entry opens.
pub struct RssScreen {
    snapshot: AsyncResource<RssSnapshot, String>,
    /// The item the user picked in the table, if it is still listed.
    selected: Option<String>,
    /// Item id whose download is in flight, so one item is not queued twice.
    downloading: Option<String>,
    error: Option<String>,
    /// The rule being added or edited, when the editor is open.
    editor: Option<RuleDraft>,
    /// Editor text fields. Read into the draft at save time.
    name_input: Entity<TextInput>,
    regex_input: Entity<TextInput>,
    category_input: Entity<TextInput>,
    /// A rule save or delete in flight.
    rule_busy: bool,
    /// Matched episodes of the rule open in the editor, and their load state.
    episodes: Vec<RssRuleEpisode>,
    episodes_loading: bool,
    episodes_error: Option<String>,
    _load: Option<Task<()>>,
    _download: Option<Task<()>>,
    _rule: Option<Task<()>>,
    _episodes: Option<Task<()>>,
}

impl RssScreen {
    #[must_use]
    pub fn new(cx: &mut Context<Self>) -> Self {
        let name_input = cx.new(|_| TextInput::new().placeholder("Rule name"));
        let regex_input = cx.new(|_| TextInput::new().placeholder("Match regex"));
        let category_input = cx.new(|_| TextInput::new().placeholder("Category (optional)"));
        Self {
            snapshot: AsyncResource::default(),
            selected: None,
            downloading: None,
            error: None,
            editor: None,
            name_input,
            regex_input,
            category_input,
            rule_busy: false,
            episodes: Vec::new(),
            episodes_loading: false,
            episodes_error: None,
            _load: None,
            _download: None,
            _rule: None,
            _episodes: None,
        }
    }

    /// The feeds, items and rules once a load has landed.
    #[must_use]
    pub fn snapshot(&self) -> Option<&RssSnapshot> {
        self.snapshot.ready_value()
    }

    /// The error bar text, if the last call failed.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The item id whose download is in flight, if any.
    #[must_use]
    pub fn downloading(&self) -> Option<&str> {
        self.downloading.as_deref()
    }

    /// The rule editor's draft, while the editor is open.
    #[must_use]
    pub fn editor(&self) -> Option<&RuleDraft> {
        self.editor.as_ref()
    }

    /// Open the editor for a new rule with the defaults.
    pub fn open_new_rule(&mut self, cx: &mut Context<Self>) {
        if self.rule_busy {
            return;
        }
        self.show_editor(RuleDraft::default(), cx);
    }

    /// Put a draft into the editor fields and show the editor.
    fn show_editor(&mut self, draft: RuleDraft, cx: &mut Context<Self>) {
        let name = draft.name.clone();
        let regex = draft.match_regex.clone();
        let category = draft.category.clone();
        self.name_input
            .update(cx, |field, cx| field.set_value(name, cx));
        self.regex_input
            .update(cx, |field, cx| field.set_value(regex, cx));
        self.category_input
            .update(cx, |field, cx| field.set_value(category, cx));
        self.editor = Some(draft);
        self.error = None;
        self.episodes.clear();
        self.episodes_loading = false;
        self.episodes_error = None;
        cx.notify();
    }

    /// Open the editor on an existing rule. Unknown ids are ignored.
    pub fn open_edit_rule(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.rule_busy {
            return;
        }
        let draft = self
            .snapshot()
            .and_then(|snapshot| snapshot.rules.iter().find(|rule| rule.id == id))
            .map(RuleDraft::edit);
        if let Some(draft) = draft {
            let id = draft.id.clone();
            self.show_editor(draft, cx);
            if let Some(id) = id {
                self.load_episodes(&id, cx);
            }
        }
    }

    /// Close the editor without saving.
    pub fn cancel_rule(&mut self, cx: &mut Context<Self>) {
        if self.rule_busy {
            return;
        }
        self.editor = None;
        self.error = None;
        cx.notify();
    }

    /// Read the matched episodes of a rule from the daemon into the editor.
    fn load_episodes(&mut self, id: &str, cx: &mut Context<Self>) {
        self.episodes_loading = true;
        cx.notify();
        let call_id = id.to_owned();
        let reply_id = id.to_owned();
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            client.rss_rule_episodes(&call_id).await
        });
        self._episodes = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let reply = match rx.await {
                    Ok(reply) => reply,
                    Err(_) => Err(Error::Unreachable("request cancelled".to_owned())),
                };
                let _ = this.update(cx, |this, cx| this.finish_episodes(&reply_id, reply, cx));
            }),
        );
    }

    fn finish_episodes(
        &mut self,
        id: &str,
        reply: Result<Vec<RssRuleEpisode>, Error>,
        cx: &mut Context<Self>,
    ) {
        // A reply for a rule the user has since closed or swapped is stale.
        if self.editor.as_ref().and_then(|draft| draft.id.as_deref()) != Some(id) {
            return;
        }
        self.episodes_loading = false;
        match reply {
            Ok(list) => {
                self.episodes = list;
                self.episodes_error = None;
            }
            Err(err) => {
                self.episodes.clear();
                self.episodes_error = Some(map_error(&err));
            }
        }
        cx.notify();
    }

    /// Tick or untick one feed in the open draft. Ignored with no editor.
    pub fn toggle_feed(&mut self, feed: &str, cx: &mut Context<Self>) {
        if let Some(draft) = self.editor.as_mut() {
            toggle_in(&mut draft.feeds, feed);
            cx.notify();
        }
    }

    /// Flip the enabled or track-episodes flag in the open draft.
    pub fn toggle_flag(&mut self, flag: DraftFlag, cx: &mut Context<Self>) {
        if let Some(draft) = self.editor.as_mut() {
            match flag {
                DraftFlag::Enabled => draft.enabled = !draft.enabled,
                DraftFlag::TrackEpisodes => draft.track_episodes = !draft.track_episodes,
            }
            cx.notify();
        }
    }
}

/// A boolean field of the rule draft that the editor shows as a checkbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftFlag {
    Enabled,
    TrackEpisodes,
}

/// Add `item` to `list` when absent, remove it when present.
fn toggle_in(list: &mut Vec<String>, item: &str) {
    if let Some(index) = list.iter().position(|one| one == item) {
        list.remove(index);
    } else {
        list.push(item.to_owned());
    }
}

impl RssScreen {
    /// Save the open draft: create it, or replace the rule it was opened on.
    /// A client-side refusal stays in the editor and makes no request.
    pub fn save_rule(&mut self, cx: &mut Context<Self>) {
        if self.rule_busy {
            return;
        }
        let Some(mut draft) = self.editor.clone() else {
            return;
        };
        draft.name = self.name_input.read(cx).text().to_owned();
        draft.match_regex = self.regex_input.read(cx).text().to_owned();
        draft.category = self.category_input.read(cx).text().to_owned();
        let body = match validate_rule(&draft) {
            Ok(body) => body,
            Err(message) => {
                self.error = Some(message.to_owned());
                cx.notify();
                return;
            }
        };
        self.rule_busy = true;
        self.error = None;
        cx.notify();
        let id = draft.id.clone();
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            match id {
                Some(id) => client.update_rss_rule(&id, &body).await,
                None => client.add_rss_rule(&body).await,
            }
        });
        self._rule = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let reply = match rx.await {
                    Ok(reply) => reply,
                    Err(_) => Err(Error::Unreachable("request cancelled".to_owned())),
                };
                let _ = this.update(cx, |this, cx| this.finish_save(reply, cx));
            }),
        );
    }

    fn finish_save(&mut self, reply: Result<(), Error>, cx: &mut Context<Self>) {
        self.rule_busy = false;
        match reply {
            Ok(()) => {
                // Close the editor and reload so the list shows the saved rule.
                self.editor = None;
                self._load = Some(Self::start_load(self, cx));
            }
            // Keep the editor open so the user can correct the regex.
            Err(err) => self.error = Some(map_error(&err)),
        }
        cx.notify();
    }

    /// Delete one rule by id, then reload. Items already downloaded stay.
    pub fn delete_rule(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.rule_busy {
            return;
        }
        self.rule_busy = true;
        self.error = None;
        cx.notify();
        let id = id.to_owned();
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            client.delete_rss_rule(&id).await
        });
        self._rule = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let reply = match rx.await {
                    Ok(reply) => reply,
                    Err(_) => Err(Error::Unreachable("request cancelled".to_owned())),
                };
                let _ = this.update(cx, |this, cx| this.finish_delete(reply, cx));
            }),
        );
    }

    fn finish_delete(&mut self, reply: Result<(), Error>, cx: &mut Context<Self>) {
        self.rule_busy = false;
        match reply {
            Ok(()) => self._load = Some(Self::start_load(self, cx)),
            Err(err) => self.error = Some(map_error(&err)),
        }
        cx.notify();
    }

    /// The item id the user selected, if it is still in the list.
    #[must_use]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Pick one item in the table. Ids not in the list are ignored.
    pub fn select(&mut self, id: &str, cx: &mut Context<Self>) {
        let listed = self
            .snapshot()
            .is_some_and(|snapshot| snapshot.items.iter().any(|item| item.id == id));
        if listed {
            self.selected = Some(id.to_owned());
            cx.notify();
        }
    }

    /// Fetch feeds, items and rules from the daemon.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self._load = Some(Self::start_load(self, cx));
    }

    fn start_load(this: &mut Self, cx: &mut Context<Self>) -> Task<()> {
        Bridge::load(
            this,
            cx,
            |screen| &mut screen.snapshot,
            |client| async move {
                let mut client = client.lock().await;
                client.rss().await
            },
        )
    }

    /// Ask the daemon to add the selected item as a torrent. Ignored while a
    /// download is in flight, when nothing is selected, or when the item
    /// cannot be downloaded.
    pub fn download_selected(&mut self, cx: &mut Context<Self>) {
        if self.downloading.is_some() {
            return;
        }
        let Some(id) = self.selected.clone() else {
            return;
        };
        let downloadable = self
            .snapshot()
            .and_then(|snapshot| snapshot.items.iter().find(|item| item.id == id))
            .is_some_and(can_download);
        if !downloadable {
            return;
        }
        self.downloading = Some(id.clone());
        self.error = None;
        cx.notify();
        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            client.download_rss_item(&id).await
        });
        self._download = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let reply = match rx.await {
                    Ok(reply) => reply,
                    Err(_) => Err(Error::Unreachable("request cancelled".to_owned())),
                };
                let _ = this.update(cx, |this, cx| this.finish_download(reply, cx));
            }),
        );
    }

    fn finish_download(&mut self, reply: Result<(), Error>, cx: &mut Context<Self>) {
        self.downloading = None;
        match reply {
            // The daemon marks the item downloaded; reload to show it.
            Ok(()) => self._load = Some(Self::start_load(self, cx)),
            Err(err) => self.error = Some(map_error(&err)),
        }
        cx.notify();
    }
}

/// Whether an item can be downloaded now: it has a link and is not already
/// downloaded.
#[must_use]
pub fn can_download(item: &RssItem) -> bool {
    !item.downloaded && item.url.is_some()
}

/// The priority label shown for a daemon priority value.
#[must_use]
pub fn priority_label(priority: i32) -> &'static str {
    match priority {
        0 => "Low",
        2 => "High",
        3 => "Force",
        _ => "Normal",
    }
}

/// One rule as a single line in the rule list.
#[must_use]
pub fn rule_line(rule: &RssRule) -> String {
    let state = if rule.enabled { "On" } else { "Off" };
    format!(
        "{} · {} · {} · {} · {state}",
        rule.name,
        rule.feed_names.join(", "),
        rule.match_regex,
        priority_label(rule.priority),
    )
}

/// One matched episode: `S01E02 · Show S01E02 · 2026-10-31 · torrent 4`.
#[must_use]
pub fn episode_line(entry: &RssRuleEpisode) -> String {
    let date = entry.matched_at.get(..10).unwrap_or(&entry.matched_at);
    let mut line = format!(
        "S{:02}E{:02} · {} · {date}",
        entry.season, entry.episode, entry.title
    );
    if let Some(id) = entry.torrent_id {
        line.push_str(&format!(" · torrent {id}"));
    }
    line
}

/// The status column text for an item.
#[must_use]
pub fn item_status(item: &RssItem) -> &'static str {
    if item.downloaded {
        "Downloaded"
    } else if item.url.is_none() {
        "No link"
    } else {
        "Ready"
    }
}

impl Render for RssScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let me: Entity<Self> = cx.entity();
        let mut root = div().flex().flex_col().gap_3().child(Label::new("RSS"));

        if let Some(message) = self.error.clone() {
            root = root.child(MessageBar::new(message).intent(MessageIntent::Error));
        }

        let table = match self.snapshot.ready_value() {
            None if self.snapshot.is_loading() => DataTable::new().loading("Loading RSS…"),
            None => DataTable::new().error_message(
                self.snapshot
                    .error()
                    .cloned()
                    .unwrap_or_else(|| "RSS is not loaded.".to_owned()),
            ),
            Some(snapshot) if snapshot.items.is_empty() => {
                DataTable::new().empty_message("No RSS items yet.")
            }
            Some(snapshot) => {
                let rows = snapshot.items.iter().map(|item| {
                    DataRow::new(
                        item.id.clone(),
                        [
                            item.title.clone(),
                            item.feed_name.clone(),
                            item_status(item).to_owned(),
                        ],
                    )
                });
                let table = DataTable::new()
                    .columns([
                        DataColumn::new("title", "Title").width(360.0),
                        DataColumn::new("feed", "Feed").width(160.0),
                        DataColumn::new("status", "Status").width(120.0),
                    ])
                    .rows(rows)
                    .on_row_select(move |id: SharedString, _window, cx| {
                        me.update(cx, |screen, cx| screen.select(&id, cx));
                    });
                match self.selected.as_deref() {
                    Some(id) => table.selected(id.to_owned()),
                    None => table,
                }
            }
        };
        root = root.child(table);

        root = root.child(self.render_rules(cx));

        let can_act = self.downloading.is_none()
            && self
                .selected
                .as_deref()
                .and_then(|id| self.snapshot()?.items.iter().find(|item| item.id == id))
                .is_some_and(can_download);
        let label = if self.downloading.is_some() {
            "Downloading…"
        } else {
            "Download"
        };
        let click = cx.listener(|this, _: &ClickEvent, _window, cx| this.download_selected(cx));
        root.child(
            Button::new("rss-download")
                .label(label)
                .appearance(ButtonAppearance::Accent)
                .disabled(!can_act)
                .on_click(click),
        )
    }
}

impl RssScreen {
    /// The rules list, the Add rule button, and the rule editor when it is open.
    fn render_rules(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let add = cx.listener(|this, _: &ClickEvent, _window, cx| this.open_new_rule(cx));
        let mut section = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(Label::new("Rules"))
            .child(
                Button::new("rss-add-rule")
                    .label("Add rule")
                    .appearance(ButtonAppearance::Neutral)
                    .disabled(self.rule_busy)
                    .on_click(add),
            );
        if let Some(snapshot) = self.snapshot() {
            for rule in &snapshot.rules {
                let id = rule.id.clone();
                let edit = cx.listener({
                    let id = id.clone();
                    move |this, _: &ClickEvent, _window, cx| this.open_edit_rule(&id, cx)
                });
                let delete = cx.listener({
                    let id = id.clone();
                    move |this, _: &ClickEvent, _window, cx| this.delete_rule(&id, cx)
                });
                section = section.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(Label::new(rule_line(rule)))
                        .child(
                            Button::new(SharedString::from(format!("rss-edit-{id}")))
                                .label("Edit")
                                .appearance(ButtonAppearance::Subtle)
                                .disabled(self.rule_busy)
                                .on_click(edit),
                        )
                        .child(
                            Button::new(SharedString::from(format!("rss-delete-{id}")))
                                .label("Delete")
                                .appearance(ButtonAppearance::Subtle)
                                .disabled(self.rule_busy)
                                .on_click(delete),
                        ),
                );
            }
        }
        if let Some(draft) = &self.editor {
            let title = if draft.id.is_some() {
                "Edit rule"
            } else {
                "New rule"
            };
            let cancel = cx.listener(|this, _: &ClickEvent, _window, cx| this.cancel_rule(cx));
            let save = cx.listener(|this, _: &ClickEvent, _window, cx| this.save_rule(cx));
            let me = cx.entity();
            let mut feeds = div().flex().flex_col().gap_1().child(Label::new("Feeds"));
            if let Some(snapshot) = self.snapshot() {
                for (index, feed) in snapshot.feeds.iter().enumerate() {
                    let name = feed.name.clone();
                    let checked = draft.feeds.contains(&name);
                    let me = me.clone();
                    let toggle = name.clone();
                    let checkbox = Checkbox::new(("rss-feed", index))
                        .checked(checked)
                        .on_click(move |_, _, _, cx| {
                            me.update(cx, |this, cx| this.toggle_feed(&toggle, cx));
                        });
                    feeds = feeds.child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(checkbox)
                            .child(Label::new(name)),
                    );
                }
            }
            let enabled_me = me.clone();
            let enabled = Checkbox::new("rss-rule-enabled")
                .checked(draft.enabled)
                .on_click(move |_, _, _, cx| {
                    enabled_me.update(cx, |this, cx| this.toggle_flag(DraftFlag::Enabled, cx));
                });
            let track_me = me.clone();
            let track = Checkbox::new("rss-rule-track")
                .checked(draft.track_episodes)
                .on_click(move |_, _, _, cx| {
                    track_me.update(cx, |this, cx| {
                        this.toggle_flag(DraftFlag::TrackEpisodes, cx);
                    });
                });
            section = section
                .child(Label::new(title))
                .child(Label::new("Name"))
                .child(self.name_input.clone())
                .child(Label::new("Match regex"))
                .child(self.regex_input.clone())
                .child(feeds)
                .child(Label::new("Category"))
                .child(self.category_input.clone())
                .child(Label::new(format!(
                    "Priority: {}",
                    priority_label(draft.priority)
                )))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(enabled)
                        .child(Label::new("Enabled")),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(track)
                        .child(Label::new("Track episodes")),
                );
            if draft.id.is_some() {
                section = section.child(self.render_episodes());
            }
            section = section.child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        Button::new("rss-rule-save")
                            .label("Save")
                            .appearance(ButtonAppearance::Accent)
                            .disabled(self.rule_busy)
                            .on_click(save),
                    )
                    .child(
                        Button::new("rss-rule-cancel")
                            .label("Cancel")
                            .appearance(ButtonAppearance::Subtle)
                            .disabled(self.rule_busy)
                            .on_click(cancel),
                    ),
            );
        }
        section
    }
}

impl RssScreen {
    /// The matched-episode history of the rule open in the editor.
    fn render_episodes(&self) -> impl IntoElement {
        let mut section = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(Label::new("Matched episodes"));
        if self.episodes_loading {
            section = section.child(Label::new("Loading…"));
        } else if let Some(message) = &self.episodes_error {
            section = section.child(Label::new(message.clone()));
        } else if self.episodes.is_empty() {
            section = section.child(Label::new("No episodes matched yet."));
        } else {
            for entry in &self.episodes {
                section = section.child(Label::new(episode_line(entry)));
            }
        }
        section
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rtbit-client/tests/fixtures"
    );

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
    }

    const ITEMS: &str = r#"[
        {"id":"a","feed_name":"Show","title":"Ready one","url":"magnet:?xt=urn:btih:aa","downloaded":false},
        {"id":"b","feed_name":"Show","title":"Done one","url":"magnet:?xt=urn:btih:bb","downloaded":true},
        {"id":"c","feed_name":"Show","title":"No link","url":null,"downloaded":false}
    ]"#;

    /// Start a wiremock daemon on the bridge runtime and install the bridge
    /// with an authenticated client. `items` is the `/rss/items` body.
    fn daemon(
        cx: &mut TestAppContext,
        items: &str,
    ) -> (std::sync::Arc<tokio::runtime::Runtime>, MockServer) {
        cx.executor().allow_parking();
        let scratch = Bridge::new("http://127.0.0.1:1").unwrap();
        let runtime = scratch.runtime().clone();
        let items = items.to_owned();
        let server = runtime.block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/auth/login"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(fixture("login.json"), "application/json"),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/rss/feeds"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(fixture("rss-feeds.json"), "application/json"),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/rss/items"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(items, "application/json"))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/rss/rules"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(fixture("rss-rules.json"), "application/json"),
                )
                .mount(&server)
                .await;
            server
        });
        let client = runtime.block_on(async {
            let mut client = rtbit_client::Client::new(server.uri()).unwrap();
            client.login("admin", "secret").await.unwrap();
            client
        });
        let bridge = Bridge::from_parts(runtime.clone(), client);
        cx.update(|cx| bridge.install(cx));
        (runtime, server)
    }

    /// Drive gpui until `done` holds. Replies arrive from tokio threads.
    fn drive(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("rss reply never arrived");
    }

    fn loaded(cx: &mut TestAppContext) -> gpui::Entity<RssScreen> {
        let screen = cx.new(RssScreen::new);
        screen.update(cx, |screen, cx| screen.load(cx));
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| !s.snapshot.is_loading())
        });
        screen
    }

    #[gpui::test]
    fn load_lists_feeds_items_and_rules(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx, ITEMS);
        let screen = loaded(cx);
        screen.read_with(cx, |screen, _| {
            let snapshot = screen.snapshot().expect("snapshot ready");
            assert_eq!(snapshot.items.len(), 3);
            assert_eq!(snapshot.feeds.len(), 1);
            assert_eq!(snapshot.rules.len(), 1);
        });
    }

    #[gpui::test]
    fn empty_items_are_a_ready_empty_list(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx, "[]");
        let screen = loaded(cx);
        screen.read_with(cx, |screen, _| {
            assert!(screen.snapshot().expect("ready").items.is_empty());
            assert!(screen.error().is_none());
        });
    }

    #[gpui::test]
    fn only_listed_items_are_selectable(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx, ITEMS);
        let screen = loaded(cx);
        screen.update(cx, |screen, cx| {
            screen.select("missing", cx);
            assert!(screen.selected().is_none());
            screen.select("a", cx);
        });
        screen.read_with(cx, |screen, _| assert_eq!(screen.selected(), Some("a")));
    }

    #[gpui::test]
    fn downloaded_and_linkless_items_cannot_download(cx: &mut TestAppContext) {
        let (_runtime, _server) = daemon(cx, ITEMS);
        let screen = loaded(cx);
        screen.read_with(cx, |screen, _| {
            let items = &screen.snapshot().unwrap().items;
            assert!(can_download(&items[0]));
            assert!(!can_download(&items[1]), "already downloaded");
            assert!(!can_download(&items[2]), "no link");
            assert_eq!(item_status(&items[1]), "Downloaded");
            assert_eq!(item_status(&items[2]), "No link");
        });
    }

    #[gpui::test]
    fn download_posts_the_item_and_reloads(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx, ITEMS);
        runtime.block_on(async {
            Mock::given(method("POST"))
                .and(path("/rss/items/a/download"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
        });
        let screen = loaded(cx);
        screen.update(cx, |screen, cx| {
            screen.select("a", cx);
            screen.download_selected(cx);
            assert_eq!(screen.downloading(), Some("a"));
        });
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| s.downloading().is_none())
        });
        screen.read_with(cx, |screen, _| {
            assert!(screen.error().is_none(), "download should succeed");
        });
    }

    #[gpui::test]
    fn refused_download_shows_an_error(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx, ITEMS);
        runtime.block_on(async {
            Mock::given(method("POST"))
                .and(path("/rss/items/a/download"))
                .respond_with(ResponseTemplate::new(500))
                .mount(&server)
                .await;
        });
        let screen = loaded(cx);
        screen.update(cx, |screen, cx| {
            screen.select("a", cx);
            screen.download_selected(cx);
        });
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| s.downloading().is_none())
        });
        screen.read_with(cx, |screen, _| {
            assert!(screen.error().is_some(), "error bar must show the refusal");
        });
    }

    const EPISODES: &str = r#"[
        {"show":"Show","season":1,"episode":2,"title":"Show S01E02","matched_at":"2026-10-31T10:00:00Z","torrent_id":4}
    ]"#;

    fn draft_with(name: &str, regex: &str, feeds: &[&str]) -> RuleDraft {
        RuleDraft {
            name: name.to_owned(),
            match_regex: regex.to_owned(),
            feeds: feeds.iter().map(|f| (*f).to_owned()).collect(),
            ..RuleDraft::default()
        }
    }

    #[test]
    fn validate_rule_requires_name_regex_and_a_feed() {
        assert_eq!(
            validate_rule(&draft_with("  ", "x", &["Show"])).unwrap_err(),
            "Name and match regex are required."
        );
        assert_eq!(
            validate_rule(&draft_with("n", "  ", &["Show"])).unwrap_err(),
            "Name and match regex are required."
        );
        assert_eq!(
            validate_rule(&draft_with("n", "x", &[])).unwrap_err(),
            "Select at least one feed."
        );
    }

    #[test]
    fn validate_rule_trims_and_drops_an_empty_category() {
        let mut draft = draft_with(" Show ", " Show.*1080p ", &["Show"]);
        draft.category = "  ".to_owned();
        let body = validate_rule(&draft).expect("valid");
        assert_eq!(body.name, "Show");
        assert_eq!(body.match_regex, "Show.*1080p");
        assert_eq!(body.feed_names, vec!["Show".to_owned()]);
        assert_eq!(body.category, None);
        assert!(body.enabled && body.track_episodes);
    }

    #[test]
    fn rule_line_shows_name_feeds_regex_priority_and_state() {
        let rule: RssRule = serde_json::from_str(&fixture("rss-rules.json"))
            .map(|rules: Vec<RssRule>| rules.into_iter().next().unwrap())
            .unwrap();
        assert_eq!(
            rule_line(&rule),
            "Show 1080p · Show · Show.*1080p · High · On"
        );
    }

    #[test]
    fn toggle_in_adds_then_removes() {
        let mut list = vec!["A".to_owned()];
        toggle_in(&mut list, "B");
        toggle_in(&mut list, "A");
        assert_eq!(list, vec!["B".to_owned()]);
    }

    #[gpui::test]
    fn edit_loads_the_matched_episodes(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx, ITEMS);
        runtime.block_on(async {
            Mock::given(method("GET"))
                .and(path("/rss/rules/r-1/episodes"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(EPISODES, "application/json"))
                .mount(&server)
                .await;
        });
        let screen = loaded(cx);
        screen.update(cx, |screen, cx| screen.open_edit_rule("r-1", cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.episodes_loading));
        screen.read_with(cx, |screen, _| {
            assert!(screen.editor().is_some(), "editor stays open");
            assert_eq!(screen.episodes.len(), 1);
            assert_eq!(
                episode_line(&screen.episodes[0]),
                "S01E02 · Show S01E02 · 2026-10-31 · torrent 4"
            );
        });
    }

    #[gpui::test]
    fn save_creates_a_rule_and_reloads(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx, ITEMS);
        runtime.block_on(async {
            Mock::given(method("POST"))
                .and(path("/rss/rules"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
        });
        let screen = loaded(cx);
        screen.update(cx, |screen, cx| {
            screen.open_new_rule(cx);
            screen.name_input.update(cx, |f, cx| f.set_value("Two", cx));
            screen
                .regex_input
                .update(cx, |f, cx| f.set_value("Two.*", cx));
            screen.editor.as_mut().unwrap().feeds = vec!["Show".to_owned()];
            screen.save_rule(cx);
        });
        drive(cx, |cx| {
            screen.read_with(cx, |s, _| s.editor().is_none() && !s.rule_busy)
        });
        screen.read_with(cx, |screen, _| {
            assert!(screen.error().is_none(), "save should succeed");
        });
    }

    #[gpui::test]
    fn a_refused_regex_keeps_the_editor_open_with_the_error(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx, ITEMS);
        runtime.block_on(async {
            Mock::given(method("POST"))
                .and(path("/rss/rules"))
                .respond_with(ResponseTemplate::new(400).set_body_string("invalid regex"))
                .mount(&server)
                .await;
        });
        let screen = loaded(cx);
        screen.update(cx, |screen, cx| {
            screen.open_new_rule(cx);
            screen.name_input.update(cx, |f, cx| f.set_value("Bad", cx));
            screen.regex_input.update(cx, |f, cx| f.set_value("(", cx));
            screen.editor.as_mut().unwrap().feeds = vec!["Show".to_owned()];
            screen.save_rule(cx);
        });
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.rule_busy));
        screen.read_with(cx, |screen, _| {
            assert!(screen.editor().is_some(), "editor must stay open");
            assert!(screen.error().is_some(), "refusal must show an error");
        });
    }

    #[gpui::test]
    fn delete_removes_the_rule_and_reloads(cx: &mut TestAppContext) {
        let (runtime, server) = daemon(cx, ITEMS);
        runtime.block_on(async {
            Mock::given(method("DELETE"))
                .and(path("/rss/rules/r-1"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
        });
        let screen = loaded(cx);
        screen.update(cx, |screen, cx| screen.delete_rule("r-1", cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.rule_busy));
        screen.read_with(cx, |screen, _| {
            assert!(screen.error().is_none(), "delete should succeed");
        });
    }
}
