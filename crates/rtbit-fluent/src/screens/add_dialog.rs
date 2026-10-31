//! Add-torrent dialog: a magnet link or a `.torrent` file, a save path, an
//! optional category and a start-paused switch.
//!
//! The dialog checks its input with the same rules the add-torrent flow applies, then
//! emits an [`AddDialogEvent::Submit`] carrying an [`AddRequest`]. The owner
//! (the main screen) sends the request through the bridge. The dialog never
//! talks to the daemon itself, so a cancel can never reach it.

use fluent_core::ThemeProvider as _;
use fluent_layout::{MessageBar, MessageIntent};
use fluent_primitives::{Button, ButtonAppearance, Label, Switch, TextInput};
use gpui::{
    AsyncApp, Context, Entity, EventEmitter, IntoElement, PathPromptOptions, Render, Task,
    WeakEntity, Window, div, prelude::*, px,
};
use rtbit_ui_core::{AddRequest, AddSource};

/// What the dialog tells its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddDialogEvent {
    /// The input passed the checks. The owner sends the request and closes the dialog.
    Submit(AddRequest),
    /// The user cancelled. Nothing is sent.
    Cancel,
}

/// The add-torrent form. Owned by the main screen while it is open.
pub struct AddDialog {
    magnet: Entity<TextInput>,
    save_path: Entity<TextInput>,
    category: Entity<TextInput>,
    start_paused: bool,
    /// Name and bytes of the `.torrent` file the user picked.
    file: Option<(String, Vec<u8>)>,
    error: Option<String>,
    /// Set once a request has been emitted, so a second submit sends nothing.
    submitted: bool,
    _pick: Option<Task<()>>,
}

impl EventEmitter<AddDialogEvent> for AddDialog {}

impl AddDialog {
    #[must_use]
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            magnet: cx.new(|_| TextInput::new().placeholder("magnet:?xt=urn:btih:")),
            save_path: cx.new(|_| TextInput::new().placeholder("Save path")),
            category: cx.new(|_| TextInput::new().placeholder("Category (optional)")),
            start_paused: false,
            file: None,
            error: None,
            submitted: false,
            _pick: None,
        }
    }

    /// The message shown when the input was refused, if any.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Check the input and emit a request, or show why it was refused.
    pub(crate) fn submit(&mut self, cx: &mut Context<Self>) {
        if self.submitted {
            return;
        }
        let magnet = self.magnet.read(cx).text().to_owned();
        let save_path = self.save_path.read(cx).text().to_owned();
        let category = self.category.read(cx).text().to_owned();
        let file = self
            .file
            .as_ref()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()));
        match validate_add(&magnet, file, &save_path, &category, self.start_paused) {
            Ok(request) => {
                self.submitted = true;
                self.error = None;
                cx.emit(AddDialogEvent::Submit(request));
            }
            Err(message) => {
                self.error = Some(message);
                cx.notify();
            }
        }
    }

    /// Ask the owner to close the dialog without sending anything.
    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(AddDialogEvent::Cancel);
    }

    /// The daemon refused the request: show why and allow a retry.
    pub(crate) fn refused(&mut self, message: String, cx: &mut Context<Self>) {
        self.submitted = false;
        self.error = Some(message);
        cx.notify();
    }

    /// Open the native file picker for one `.torrent` file and read it off the UI thread.
    fn pick_file(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        self._pick = Some(
            cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
                let Ok(Ok(Some(paths))) = picked.await else {
                    return;
                };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("torrent")
                    .to_owned();
                let read = cx
                    .background_executor()
                    .spawn(async move { std::fs::read(&path) })
                    .await;
                let _ = view.update(cx, |dialog, cx| {
                    match read {
                        Ok(bytes) => {
                            dialog.file = Some((name, bytes));
                            dialog.error = None;
                        }
                        Err(err) => dialog.error = Some(format!("could not read {name}: {err}")),
                    }
                    cx.notify();
                });
            }),
        );
    }

    /// Stand in for the native picker: set the chosen `.torrent` file, for tests.
    #[cfg(test)]
    pub(crate) fn set_file_for_test(&mut self, name: &str, bytes: &[u8]) {
        self.file = Some((name.to_owned(), bytes.to_vec()));
    }

    /// Type a magnet and a save path, for tests outside this module.
    #[cfg(test)]
    pub(crate) fn fill_for_test(&self, cx: &mut gpui::App, magnet: &str, save_path: &str) {
        self.magnet
            .update(cx, |field, cx| field.set_value(magnet, cx));
        self.save_path
            .update(cx, |field, cx| field.set_value(save_path, cx));
    }
}

fn field(label: &'static str, input: Entity<TextInput>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(Label::new(label))
        .child(input)
}

impl Render for AddDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors.clone();
        let file_label = self
            .file
            .as_ref()
            .map_or_else(|| "No file chosen".to_owned(), |(name, _)| name.clone());
        let pick_me = cx.entity();
        let paused_me = cx.entity();
        let cancel_me = cx.entity();
        let add_me = cx.entity();

        let mut card = div()
            .flex()
            .flex_col()
            .gap_3()
            .w(px(480.))
            .p_4()
            .rounded_md()
            .bg(colors.surface)
            .border_1()
            .border_color(colors.stroke_neutral)
            .child(Label::new("Add torrent"))
            .child(field("Magnet link", self.magnet.clone()))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("add-pick-file")
                            .label("Choose .torrent")
                            .on_click(move |_, _, app| {
                                pick_me.update(app, |dialog, cx| dialog.pick_file(cx));
                            }),
                    )
                    .child(Label::new(file_label)),
            )
            .child(field("Save path", self.save_path.clone()))
            .child(field("Category", self.category.clone()))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Switch::new("add-start-paused")
                            .on(self.start_paused)
                            .on_click(move |next, _, _, app| {
                                paused_me.update(app, |dialog, cx| {
                                    dialog.start_paused = next;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(Label::new("Start paused")),
            );
        if let Some(message) = &self.error {
            card = card.child(MessageBar::new(message.clone()).intent(MessageIntent::Error));
        }
        card = card.child(
            div()
                .flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("add-cancel")
                        .label("Cancel")
                        .on_click(move |_, _, app| {
                            cancel_me.update(app, |dialog, cx| dialog.cancel(cx));
                        }),
                )
                .child(
                    Button::new("add-submit")
                        .label("Add torrent")
                        .appearance(ButtonAppearance::Accent)
                        .on_click(move |_, _, app| {
                            add_me.update(app, |dialog, cx| dialog.submit(cx));
                        }),
                ),
        );

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .bg(gpui::black().opacity(0.4))
            .child(card)
    }
}

/// Client-side checks, the same rules the add-torrent flow applies. The daemon
/// still rejects a bad magnet or a bad path. A magnet wins over a file.
pub fn validate_add(
    magnet: &str,
    file: Option<(&str, &[u8])>,
    save_path: &str,
    category: &str,
    start_paused: bool,
) -> Result<AddRequest, String> {
    let save_path = save_path.trim().to_owned();
    if let Some(message) = check_save_path(&save_path) {
        return Err(message.to_owned());
    }
    let magnet = magnet.trim();
    let source = if !magnet.is_empty() {
        if let Some(message) = check_magnet(magnet) {
            return Err(message.to_owned());
        }
        AddSource::Magnet(magnet.to_owned())
    } else if let Some((name, bytes)) = file.filter(|(_, bytes)| !bytes.is_empty()) {
        AddSource::File {
            name: name.to_owned(),
            bytes: bytes.to_vec(),
        }
    } else {
        return Err("enter a magnet link or choose a .torrent file".to_owned());
    };
    Ok(AddRequest {
        source,
        save_path,
        category: category.trim().to_owned(),
        start_paused,
    })
}

fn check_magnet(magnet: &str) -> Option<&'static str> {
    let Some(rest) = magnet.strip_prefix("magnet:?") else {
        return Some("magnet link must start with magnet:?");
    };
    let ok = rest.split('&').any(|part| {
        let mut key_value = part.splitn(2, '=');
        key_value.next() == Some("xt")
            && key_value
                .next()
                .is_some_and(|value| value.to_ascii_lowercase().starts_with("urn:btih:"))
    });
    if ok {
        None
    } else {
        Some("magnet link needs xt=urn:btih:")
    }
}

fn check_save_path(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("save path is required");
    }
    if path.contains('\0') {
        return Some("save path contains a null");
    }
    if path.contains('\n') || path.contains('\r') {
        return Some("save path must be a single line");
    }
    None
}

#[cfg(test)]
mod validation {
    use super::*;

    const HASH_MAGNET: &str =
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Example";

    #[test]
    fn a_magnet_without_a_hash_is_refused() {
        let err = validate_add("magnet:?dn=nohash", None, "/data", "", false).unwrap_err();
        assert_eq!(err, "magnet link needs xt=urn:btih:");
    }

    #[test]
    fn a_link_that_is_not_a_magnet_is_refused() {
        let err = validate_add("https://example.com/x", None, "/data", "", false).unwrap_err();
        assert_eq!(err, "magnet link must start with magnet:?");
    }

    #[test]
    fn a_blank_path_is_refused() {
        let err = validate_add(HASH_MAGNET, None, "  ", "", false).unwrap_err();
        assert_eq!(err, "save path is required");
    }

    #[test]
    fn nothing_to_add_is_refused() {
        let err = validate_add("", None, "/data", "", false).unwrap_err();
        assert_eq!(err, "enter a magnet link or choose a .torrent file");
    }

    #[test]
    fn a_good_magnet_and_path_is_accepted() {
        let request =
            validate_add(HASH_MAGNET, None, " /data/incoming ", " movies ", true).expect("valid");
        assert!(matches!(request.source, AddSource::Magnet(_)));
        assert_eq!(request.save_path, "/data/incoming");
        assert_eq!(request.category, "movies");
        assert!(request.start_paused);
    }

    #[test]
    fn a_file_is_accepted_when_no_magnet_is_given() {
        let bytes = b"d4:infod4:name1:xee";
        let request =
            validate_add("", Some(("x.torrent", bytes)), "/data", "", false).expect("valid");
        match request.source {
            AddSource::File { name, bytes: sent } => {
                assert_eq!(name, "x.torrent");
                assert_eq!(sent, bytes);
            }
            other => panic!("expected a file source, got {other:?}"),
        }
    }

    #[test]
    fn a_magnet_wins_over_a_file() {
        let request = validate_add(HASH_MAGNET, Some(("x.torrent", b"d")), "/data", "", false)
            .expect("valid");
        assert!(matches!(request.source, AddSource::Magnet(_)));
    }
}
