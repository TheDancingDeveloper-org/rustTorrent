//! RSS: feeds, items and auto-download rules.
//!
//! Items: a downloaded item is shown but its button is off. Rules are listed
//! under each feed they apply to (a rule on two feeds shows under both) and
//! can be added, edited and deleted. The rule dialog matches the web UI's: name,
//! match regex, feeds, category, priority and enabled. The daemon compiles the
//! regex; its reason for a refusal is shown inside the dialog.
//!
//! Episode tracking is not a setting: the daemon always skips an episode it
//! already downloaded for a series, and it has no route that lists those
//! episodes, so there is no toggle and no history here.

use fluent2_egui::controls::{Appearance, Button, Checkbox, Dropdown, Input};
use fluent2_egui::surfaces::{CloseReason, Dialog, DialogResult};
use rtbit_client::{RssFeed, RssItem, RssRule, RssRuleBody, RssSnapshot};

/// Priority labels, indexed by the daemon's priority value.
const PRIORITIES: [&str; 4] = ["Low", "Normal", "High", "Force"];

/// What the RSS screen is showing, from one read of the daemon.
#[derive(Clone, Debug, Default)]
pub struct RssView {
    pub feeds: Vec<RssFeed>,
    pub items: Vec<RssItem>,
    pub rules: Vec<RssRule>,
}

impl RssView {
    pub fn from_snapshot(snap: &RssSnapshot) -> Self {
        Self {
            feeds: snap.feeds.clone(),
            items: snap.items.clone(),
            rules: snap.rules.clone(),
        }
    }
}

/// The rule dialog's fields. `id` is `None` for a new rule.
#[derive(Clone, Debug, PartialEq)]
pub struct RuleDraft {
    pub id: Option<String>,
    pub name: String,
    pub match_regex: String,
    pub feeds: Vec<String>,
    pub category: String,
    pub priority: Option<usize>,
    pub enabled: bool,
    /// A client check or the daemon's refusal, shown inside the dialog.
    pub error: String,
}

impl Default for RuleDraft {
    fn default() -> Self {
        Self {
            id: None,
            name: String::new(),
            match_regex: String::new(),
            feeds: Vec::new(),
            category: String::new(),
            priority: Some(1),
            enabled: true,
            error: String::new(),
        }
    }
}

impl RuleDraft {
    fn edit(rule: &RssRule) -> Self {
        Self {
            id: Some(rule.id.clone()),
            name: rule.name.clone(),
            match_regex: rule.match_regex.clone(),
            feeds: rule.feed_names.clone(),
            category: rule.category.clone().unwrap_or_default(),
            priority: usize::try_from(rule.priority)
                .ok()
                .filter(|p| *p < PRIORITIES.len()),
            enabled: rule.enabled,
            error: String::new(),
        }
    }
}

/// Dialog state the app keeps between frames.
#[derive(Clone, Debug, Default)]
pub struct RuleEditor {
    pub open: bool,
    pub draft: RuleDraft,
    /// The rule the delete confirmation is about.
    pub delete: Option<RssRule>,
    pub delete_open: bool,
}

impl RuleEditor {
    /// Close the dialog after the daemon accepted the rule.
    pub fn saved(&mut self) {
        self.open = false;
        self.draft = RuleDraft::default();
    }

    /// Keep the dialog open and show why the daemon refused the rule.
    pub fn refused(&mut self, message: String) {
        self.open = true;
        self.draft.error = message;
    }
}

/// What the user asked for this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum RssAction {
    Download(String),
    /// `id` is `None` to add a rule, or the rule to replace.
    SaveRule {
        id: Option<String>,
        body: RssRuleBody,
    },
    DeleteRule(String),
}

/// Draw feeds, rules and items, and the rule dialogs.
pub fn draw_rss(
    ui: &mut egui::Ui,
    view: &RssView,
    editor: &mut RuleEditor,
    busy: bool,
    error: &str,
) -> Option<RssAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label("Feeds");
        if ui
            .add_enabled(
                !busy && !view.feeds.is_empty(),
                Button::new("Add rule").appearance(Appearance::Secondary),
            )
            .clicked()
        {
            editor.draft = RuleDraft::default();
            editor.open = true;
        }
    });
    if view.feeds.is_empty() {
        ui.label("No feeds.");
    }
    for feed in &view.feeds {
        let state = if feed.enabled { "on" } else { "off" };
        ui.label(format!("{} ({state})", feed.name));
        let rules: Vec<&RssRule> = view
            .rules
            .iter()
            .filter(|rule| rule.feed_names.contains(&feed.name))
            .collect();
        ui.indent(("rss_feed_rules", &feed.name), |ui| {
            if rules.is_empty() {
                ui.label("No rules.");
            }
            for rule in rules {
                rule_row(ui, rule, editor, busy);
            }
        });
    }
    ui.add_space(8.0);
    ui.label("Items");
    if view.items.is_empty() {
        ui.label("No items.");
    }
    for item in &view.items {
        ui.horizontal(|ui| {
            ui.label(item.title.as_str());
            if item.downloaded {
                ui.label("Downloaded");
            } else if item.url.is_none() {
                ui.label("No link");
            } else if ui
                .add_enabled(
                    !busy,
                    Button::new("Download").appearance(Appearance::Primary),
                )
                .clicked()
            {
                action = Some(RssAction::Download(item.id.clone()));
            }
        });
    }
    if !error.is_empty() {
        ui.add_space(8.0);
        ui.colored_label(egui::Color32::from_rgb(196, 43, 28), error);
    }

    let feed_names: Vec<String> = view.feeds.iter().map(|feed| feed.name.clone()).collect();
    if let Some(saved) = rule_dialog(ui.ctx(), editor, &feed_names, busy) {
        action = Some(saved);
    }
    if let Some(id) = delete_dialog(ui.ctx(), editor) {
        action = Some(RssAction::DeleteRule(id));
    }
    action
}

fn rule_row(ui: &mut egui::Ui, rule: &RssRule, editor: &mut RuleEditor, busy: bool) {
    ui.horizontal(|ui| {
        let state = if rule.enabled { "" } else { ", off" };
        ui.label(format!("{}: /{}/{state}", rule.name, rule.match_regex));
        if ui
            .add_enabled(!busy, Button::new("Edit").appearance(Appearance::Subtle))
            .clicked()
        {
            editor.draft = RuleDraft::edit(rule);
            editor.open = true;
        }
        if ui
            .add_enabled(!busy, Button::new("Delete").appearance(Appearance::Subtle))
            .clicked()
        {
            editor.delete = Some(rule.clone());
            editor.delete_open = true;
        }
    });
}

/// The add/edit dialog. Returns a save only when Save is pressed and the
/// client checks pass. The dialog stays open until the app reports the result.
fn rule_dialog(
    ctx: &egui::Context,
    editor: &mut RuleEditor,
    feed_names: &[String],
    busy: bool,
) -> Option<RssAction> {
    if !editor.open {
        return None;
    }
    let confirm_id = egui::Id::unique("rtbit_rule_confirm");
    let mut action = None;
    // As in the add dialog: read the fields one frame after Save, so text typed
    // in the same frame as the key press is included.
    if ctx
        .data(|data| data.get_temp::<bool>(confirm_id))
        .unwrap_or(false)
    {
        ctx.data_mut(|data| data.remove::<bool>(confirm_id));
        match validate_rule(&editor.draft) {
            Ok(body) => {
                editor.draft.error.clear();
                action = Some(RssAction::SaveRule {
                    id: editor.draft.id.clone(),
                    body,
                });
            }
            Err(message) => editor.draft.error = message.to_owned(),
        }
    }
    let title = if editor.draft.id.is_some() {
        "Edit rule"
    } else {
        "Add rule"
    };
    let draft = &mut editor.draft;
    let result = Dialog::new("rtbit_rule")
        .title(title)
        .actions(|actions| {
            actions.primary("save", "Save rule");
            actions.secondary("cancel", "Cancel");
        })
        .show(ctx, &mut editor.open, |ui| {
            ui.add(Input::new(&mut draft.name).label("Rule name"));
            ui.add_space(8.0);
            ui.add(
                Input::new(&mut draft.match_regex)
                    .label("Match regex")
                    .placeholder("e.g. 1080p.*HEVC"),
            );
            ui.add_space(8.0);
            ui.add(
                Input::new(&mut draft.category)
                    .label("Category")
                    .placeholder("Optional"),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Priority");
                ui.add(Dropdown::new(
                    "rtbit_rule_priority",
                    &mut draft.priority,
                    &PRIORITIES,
                ));
            });
            ui.add_space(8.0);
            ui.label("Feeds");
            if feed_names.is_empty() {
                ui.label("No feeds configured");
            }
            for feed in feed_names {
                let was = draft.feeds.contains(feed);
                let mut on = was;
                // Compare values: the checkbox does not mark its response changed.
                ui.add(Checkbox::bool(&mut on, feed.as_str()));
                if on && !was {
                    draft.feeds.push(feed.clone());
                } else if was && !on {
                    draft.feeds.retain(|name| name != feed);
                }
            }
            ui.add_space(8.0);
            ui.add(Checkbox::bool(&mut draft.enabled, "Enabled"));
            if busy {
                ui.add_space(8.0);
                ui.label("Saving…");
            }
            if !draft.error.is_empty() {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::from_rgb(196, 43, 28), &draft.error);
            }
        });
    match result {
        DialogResult::Closed(CloseReason::Button(id)) if id == "save" => {
            if !busy {
                ctx.data_mut(|data| data.insert_temp(confirm_id, true));
            }
            editor.open = true;
        }
        DialogResult::Closed(_) => editor.draft = RuleDraft::default(),
        DialogResult::Open(_) => {}
    }
    action
}

fn delete_dialog(ctx: &egui::Context, editor: &mut RuleEditor) -> Option<String> {
    if !editor.delete_open {
        return None;
    }
    let name = editor
        .delete
        .as_ref()
        .map(|rule| rule.name.clone())
        .unwrap_or_default();
    let result = Dialog::new("rtbit_rule_delete")
        .title("Delete rule")
        .actions(|actions| {
            actions.primary("delete", "Delete rule");
            actions.secondary("cancel", "Cancel");
        })
        .show(ctx, &mut editor.delete_open, |ui| {
            ui.label(format!(
                "Delete the rule \"{name}\"? Items already downloaded stay."
            ));
        });
    match result {
        DialogResult::Closed(CloseReason::Button(id)) if id == "delete" => {
            editor.delete.take().map(|rule| rule.id)
        }
        DialogResult::Closed(_) => {
            editor.delete = None;
            None
        }
        DialogResult::Open(_) => None,
    }
}

/// Client checks, the same as the web UI's. The daemon still compiles the regex.
pub fn validate_rule(draft: &RuleDraft) -> Result<RssRuleBody, &'static str> {
    let name = draft.name.trim();
    let match_regex = draft.match_regex.trim();
    if name.is_empty() || match_regex.is_empty() {
        return Err("Name and match regex are required");
    }
    if draft.feeds.is_empty() {
        return Err("Select at least one feed");
    }
    let category = draft.category.trim();
    Ok(RssRuleBody {
        name: name.to_owned(),
        feed_names: draft.feeds.clone(),
        category: (!category.is_empty()).then(|| category.to_owned()),
        priority: draft
            .priority
            .and_then(|p| i32::try_from(p).ok())
            .unwrap_or(1),
        match_regex: match_regex.to_owned(),
        enabled: draft.enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_needs_a_name_a_regex_and_a_feed() {
        let mut draft = RuleDraft::default();
        assert_eq!(
            validate_rule(&draft),
            Err("Name and match regex are required")
        );
        draft.name = " Show ".into();
        draft.match_regex = "S01".into();
        assert_eq!(validate_rule(&draft), Err("Select at least one feed"));
        draft.feeds.push("Show".into());
        let body = validate_rule(&draft).unwrap();
        assert_eq!(body.name, "Show");
        assert_eq!(body.category, None);
        assert_eq!(body.priority, 1);
        assert!(body.enabled);
    }

    #[test]
    fn editing_keeps_every_field() {
        let rule = RssRule {
            id: "r-1".into(),
            name: "Show".into(),
            feed_names: vec!["A".into(), "B".into()],
            category: Some("tv".into()),
            priority: 3,
            match_regex: "S01".into(),
            enabled: false,
        };
        let draft = RuleDraft::edit(&rule);
        assert_eq!(draft.priority, Some(3));
        let body = validate_rule(&draft).unwrap();
        assert_eq!(body.feed_names, ["A", "B"]);
        assert_eq!(body.category.as_deref(), Some("tv"));
        assert_eq!(body.priority, 3);
        assert!(!body.enabled);
    }
}
