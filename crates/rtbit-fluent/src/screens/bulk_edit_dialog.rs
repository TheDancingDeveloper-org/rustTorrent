use fluent_core::ThemeProvider as _;
use fluent_layout::{MessageBar, MessageIntent};
use fluent_primitives::{Button, ButtonAppearance, Label, TextInput};
use gpui::{Context, Entity, EventEmitter, IntoElement, Render, Window, div, prelude::*, px};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BulkEditRequest {
    pub category: Option<Option<String>>,
    pub add_tags: Vec<String>,
    pub remove_tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BulkEditDialogEvent {
    Submit(BulkEditRequest),
    Cancel,
}

pub struct BulkEditDialog {
    count: usize,
    categories: Vec<String>,
    selected_category: Option<Option<String>>,
    add_tags: Entity<TextInput>,
    remove_tags: Entity<TextInput>,
    error: Option<String>,
}

impl EventEmitter<BulkEditDialogEvent> for BulkEditDialog {}

impl BulkEditDialog {
    pub fn new(count: usize, categories: Vec<String>, cx: &mut Context<Self>) -> Self {
        Self {
            count,
            categories,
            selected_category: None,
            add_tags: cx.new(|_| TextInput::new().placeholder("comma-separated tags")),
            remove_tags: cx.new(|_| TextInput::new().placeholder("comma-separated tags")),
            error: None,
        }
    }

    fn tags(input: &Entity<TextInput>, cx: &gpui::App) -> Vec<String> {
        input
            .read(cx)
            .text()
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .collect()
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let request = BulkEditRequest {
            category: self.selected_category.clone(),
            add_tags: Self::tags(&self.add_tags, cx),
            remove_tags: Self::tags(&self.remove_tags, cx),
        };
        if request.category.is_none()
            && request.add_tags.is_empty()
            && request.remove_tags.is_empty()
        {
            self.error = Some("Choose a category or enter tags first.".to_owned());
            cx.notify();
            return;
        }
        cx.emit(BulkEditDialogEvent::Submit(request));
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

impl Render for BulkEditDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors.clone();
        let clear = cx.entity();
        let mut categories = div().flex().flex_wrap().gap_1().child(
            Button::new("bulk-category-none")
                .label("No category")
                .appearance(if self.selected_category == Some(None) {
                    ButtonAppearance::Accent
                } else {
                    ButtonAppearance::Neutral
                })
                .on_click(move |_, _, app| {
                    clear.update(app, |dialog, cx| {
                        dialog.selected_category = Some(None);
                        cx.notify();
                    });
                }),
        );
        for (index, category) in self.categories.iter().cloned().enumerate() {
            let me = cx.entity();
            let selected = self.selected_category.as_ref() == Some(&Some(category.clone()));
            let value = category.clone();
            categories = categories.child(
                Button::new(("bulk-category", index))
                    .label(category)
                    .appearance(if selected {
                        ButtonAppearance::Accent
                    } else {
                        ButtonAppearance::Neutral
                    })
                    .on_click(move |_, _, app| {
                        me.update(app, |dialog, cx| {
                            dialog.selected_category = Some(Some(value.clone()));
                            cx.notify();
                        });
                    }),
            );
        }
        let cancel = cx.entity();
        let submit = cx.entity();
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
            .child(Label::new(format!("Edit {} selected torrents", self.count)))
            .child(Label::new("Set category"))
            .child(categories)
            .child(field("Add tags", self.add_tags.clone()))
            .child(field("Remove tags", self.remove_tags.clone()));
        if let Some(error) = &self.error {
            card = card.child(MessageBar::new(error.clone()).intent(MessageIntent::Error));
        }
        card.child(
            div()
                .flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("bulk-cancel")
                        .label("Cancel")
                        .on_click(move |_, _, app| {
                            cancel.update(app, |_, cx| cx.emit(BulkEditDialogEvent::Cancel));
                        }),
                )
                .child(
                    Button::new("bulk-apply")
                        .label("Apply")
                        .appearance(ButtonAppearance::Accent)
                        .on_click(move |_, _, app| {
                            submit.update(app, |dialog, cx| dialog.submit(cx));
                        }),
                ),
        )
    }
}
