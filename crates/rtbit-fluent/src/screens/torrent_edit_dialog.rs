//! Small text dialogs for changing one torrent's location or display name.

use fluent_core::ThemeProvider as _;
use fluent_layout::{MessageBar, MessageIntent};
use fluent_primitives::{Button, ButtonAppearance, Label, TextInput};
use gpui::{
    AsyncApp, Context, Entity, EventEmitter, Focusable as _, IntoElement, KeyDownEvent,
    PathPromptOptions, Render, Task, WeakEntity, Window, div, prelude::*, px,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorrentEditKind {
    Location,
    Name,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TorrentEditDialogEvent {
    Submit(String),
    Cancel,
}

pub struct TorrentEditDialog {
    kind: TorrentEditKind,
    input: Entity<TextInput>,
    pub(crate) error: Option<String>,
    submitted: bool,
    autofocused: bool,
    _pick: Option<Task<()>>,
}

impl EventEmitter<TorrentEditDialogEvent> for TorrentEditDialog {}

impl TorrentEditDialog {
    pub fn new(kind: TorrentEditKind, initial: String, cx: &mut Context<Self>) -> Self {
        let placeholder = match kind {
            TorrentEditKind::Location => "/downloads",
            TorrentEditKind::Name => "Torrent name",
        };
        Self {
            kind,
            input: cx.new(|_| TextInput::new().placeholder(placeholder).value(initial)),
            error: None,
            submitted: false,
            autofocused: false,
            _pick: None,
        }
    }

    pub fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        self.submitted = false;
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).text().trim().to_owned();
        if value.is_empty() {
            self.error = Some(
                match self.kind {
                    TorrentEditKind::Location => "Choose or enter a location first.",
                    TorrentEditKind::Name => "Enter a name first.",
                }
                .to_owned(),
            );
            cx.notify();
            return;
        }
        if !self.submitted {
            self.submitted = true;
            cx.emit(TorrentEditDialogEvent::Submit(value));
        }
    }

    fn pick_directory(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
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
                let _ = view.update(cx, |dialog, cx| {
                    dialog
                        .input
                        .update(cx, |input, cx| input.set_value(path.to_string_lossy(), cx));
                    dialog.error = None;
                    cx.notify();
                });
            }),
        );
    }
}

impl Render for TorrentEditDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.autofocused {
            window.focus(&self.input.focus_handle(cx), cx);
            self.autofocused = true;
        }
        let title = match self.kind {
            TorrentEditKind::Location => "Set location",
            TorrentEditKind::Name => "Rename torrent",
        };
        let submit_label = match self.kind {
            TorrentEditKind::Location => "Move",
            TorrentEditKind::Name => "Rename",
        };
        let cancel = cx.entity();
        let submit = cx.entity();
        let pick = cx.entity();
        let mut card = div()
            .w(px(480.0))
            .p_4()
            .gap_3()
            .flex()
            .flex_col()
            .rounded(px(6.0))
            .bg(cx.theme().colors.surface)
            .child(Label::new(title))
            .child(self.input.clone());
        if self.kind == TorrentEditKind::Location {
            card = card.child(
                Button::new("choose-location")
                    .label("Choose folder…")
                    .on_click(move |_, _, app| {
                        pick.update(app, |dialog, cx| dialog.pick_directory(cx))
                    }),
            );
        }
        if let Some(error) = &self.error {
            card = card.child(MessageBar::new(error.clone()).intent(MessageIntent::Error));
        }
        card = card.child(
            div()
                .flex()
                .gap_2()
                .justify_end()
                .child(Button::new("torrent-edit-cancel").label("Cancel").on_click(
                    move |_, _, app| {
                        cancel.update(app, |_, cx| cx.emit(TorrentEditDialogEvent::Cancel));
                    },
                ))
                .child(
                    Button::new("torrent-edit-submit")
                        .label(submit_label)
                        .appearance(ButtonAppearance::Accent)
                        .on_click(move |_, _, app| {
                            submit.update(app, |dialog, cx| dialog.submit(cx));
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
            .capture_key_down(cx.listener(|dialog, event: &KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        cx.emit(TorrentEditDialogEvent::Cancel);
                        cx.stop_propagation();
                    }
                    "enter" => {
                        dialog.submit(cx);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .child(card)
    }
}
