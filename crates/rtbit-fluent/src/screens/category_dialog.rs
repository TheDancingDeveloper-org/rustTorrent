use fluent_core::ThemeProvider as _;
use fluent_layout::{MessageBar, MessageIntent};
use fluent_primitives::{Button, ButtonAppearance, Label, TextInput};
use gpui::{Context, Entity, EventEmitter, IntoElement, Render, Window, div, prelude::*, px};
use rtbit_client::{Category, CategoryBody, CategoryLimits};

#[derive(Clone, Debug, PartialEq)]
pub enum CategoryDialogEvent {
    Submit(CategoryBody),
    Cancel,
}

/// Edits one category's three share limits. Blank fields use the global limit.
pub struct CategoryDialog {
    name: String,
    save_path: Option<String>,
    ratio: Entity<TextInput>,
    seeding: Entity<TextInput>,
    inactive: Entity<TextInput>,
    error: Option<String>,
    submitted: bool,
}

impl EventEmitter<CategoryDialogEvent> for CategoryDialog {}

/// Validate the three text fields and build the save request.
pub fn build_body(
    name: &str,
    save_path: Option<String>,
    ratio: &str,
    seeding: &str,
    inactive: &str,
) -> Result<CategoryBody, String> {
    Ok(CategoryLimits::parse(ratio, seeding, inactive)?.into_body(name, save_path))
}

impl CategoryDialog {
    pub fn new(name: String, current: Option<&Category>, cx: &mut Context<Self>) -> Self {
        let ratio = current
            .and_then(|c| c.ratio_limit)
            .map(|v| v.to_string())
            .unwrap_or_default();
        let minutes = |secs: Option<u64>| secs.map(|s| (s / 60).to_string()).unwrap_or_default();
        let seeding = minutes(current.and_then(|c| c.seeding_time_limit));
        let inactive = minutes(current.and_then(|c| c.inactive_seeding_time_limit));
        Self {
            name,
            save_path: current.and_then(|c| c.save_path.clone()),
            ratio: cx.new(|_| TextInput::new().placeholder("use global").value(ratio)),
            seeding: cx.new(|_| TextInput::new().placeholder("use global").value(seeding)),
            inactive: cx.new(|_| TextInput::new().placeholder("use global").value(inactive)),
            error: None,
            submitted: false,
        }
    }

    pub fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        self.submitted = false;
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.submitted {
            return;
        }
        match build_body(
            &self.name,
            self.save_path.clone(),
            self.ratio.read(cx).text(),
            self.seeding.read(cx).text(),
            self.inactive.read(cx).text(),
        ) {
            Ok(body) => {
                self.submitted = true;
                self.error = None;
                cx.emit(CategoryDialogEvent::Submit(body));
            }
            Err(error) => self.set_error(error, cx),
        }
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

impl Render for CategoryDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors.clone();
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
            .child(Label::new(format!("Share limits for “{}”", self.name)))
            .child(field("Ratio limit (blank = global)", self.ratio.clone()))
            .child(field(
                "Seeding time limit, minutes (blank = global)",
                self.seeding.clone(),
            ))
            .child(field(
                "Inactive seeding time limit, minutes (blank = global)",
                self.inactive.clone(),
            ));
        if let Some(error) = &self.error {
            card = card.child(MessageBar::new(error.clone()).intent(MessageIntent::Error));
        }
        card.child(
            div()
                .flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("category-cancel")
                        .label("Cancel")
                        .on_click(move |_, _, app| {
                            cancel.update(app, |_, cx| cx.emit(CategoryDialogEvent::Cancel));
                        }),
                )
                .child(
                    Button::new("category-save")
                        .label("Save")
                        .appearance(ButtonAppearance::Accent)
                        .on_click(move |_, _, app| {
                            submit.update(app, |dialog, cx| dialog.submit(cx));
                        }),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtbit_client::Client;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn blank_fields_leave_every_limit_unset() {
        let body = build_body("tv", None, "", " ", "").unwrap();
        assert_eq!(body.ratio_limit, None);
        assert_eq!(body.seeding_time_limit, None);
        assert_eq!(body.inactive_seeding_time_limit, None);
    }

    #[test]
    fn invalid_fields_are_rejected() {
        assert!(build_body("tv", None, "-1", "", "").is_err());
        assert!(build_body("tv", None, "x", "", "").is_err());
        assert!(build_body("tv", None, "", "1.5", "").is_err());
        assert!(build_body("tv", None, "", "", "-2").is_err());
    }

    #[tokio::test]
    async fn the_built_body_is_posted_to_the_categories_route() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/login"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "a", "refresh_token": "r", "token_type": "Bearer", "expires_in": 900, "role": "admin"
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/torrents/categories"))
            .and(body_json(serde_json::json!({
                "name": "tv", "save_path": "/data/tv",
                "ratio_limit": 2.0, "seeding_time_limit": 3600
            })))
            .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
            .expect(1)
            .mount(&server)
            .await;
        let mut client = Client::new(server.uri()).unwrap();
        client.login("ada", "secret").await.unwrap();
        let body = build_body("tv", Some("/data/tv".into()), "2", "60", "").unwrap();
        client.save_category(&body).await.unwrap();
    }
}
