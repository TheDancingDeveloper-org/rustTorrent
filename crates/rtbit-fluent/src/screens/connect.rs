//! Connect screen: daemon URL, username and masked password, one sign-in call
//! on the bridge, and an error bar when the daemon refuses.
//!
//! The sign-in runs on the bridge's tokio runtime. The UI thread only sets the
//! pending flag and awaits the reply in a gpui task. The shell reacts to the
//! [`ConnectEvent`]s to update the connection badge and the window title.
//!
//! Setup (first-run account) and refresh-token resume from the keychain are not
//! in this slice.

use fluent_layout::{MessageBar, MessageIntent};
use fluent_primitives::{Button, ButtonAppearance, Label, TextInput};
use gpui::{
    AsyncApp, ClickEvent, Context, Entity, EventEmitter, IntoElement, Render, WeakEntity, Window,
    div, prelude::*, px,
};
use rtbit_client::Error;
use rtbit_ui_core::map_error;

use crate::bridge::Bridge;

/// Address the egui shell starts with.
pub const DEFAULT_URL: &str = "http://127.0.0.1:3030";

/// What the shell needs to know about a sign-in attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectEvent {
    /// A sign-in request is in flight.
    Connecting,
    /// The daemon accepted the credentials. The shell opens the main screen.
    Connected,
    /// The attempt failed. The message is shown on this screen.
    Failed,
}

/// The connect form. Fields are read when the user submits; nothing is logged.
pub struct ConnectScreen {
    url: Entity<TextInput>,
    username: Entity<TextInput>,
    password: Entity<TextInput>,
    error: Option<String>,
    pending: bool,
}

impl EventEmitter<ConnectEvent> for ConnectScreen {}

impl ConnectScreen {
    #[must_use]
    pub fn new(cx: &mut Context<Self>) -> Self {
        let url = cx.new(|_| TextInput::new().value(DEFAULT_URL));
        let username = cx.new(|_| TextInput::new().placeholder("Username"));
        let password = cx.new(|_| TextInput::new().placeholder("Password").masked(true));
        Self {
            url,
            username,
            password,
            error: None,
            pending: false,
        }
    }

    /// The message shown in the error bar, if the last attempt failed.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// `true` while a sign-in request is in flight.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Validate the form and start one sign-in on the bridge. A second submit
    /// while one is pending does nothing.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let url = self.url.read(cx).text().trim().to_owned();
        let username = self.username.read(cx).text().trim().to_owned();
        let password = self.password.read(cx).text().to_owned();
        if url.is_empty() || username.is_empty() || password.is_empty() {
            self.fail("Daemon URL, username and password are required.", cx);
            return;
        }
        let bridge = match Bridge::new(&url) {
            Ok(bridge) => bridge,
            Err(err) => {
                self.fail(&map_error(&err), cx);
                return;
            }
        };
        bridge.install(cx);

        self.pending = true;
        self.error = None;
        cx.emit(ConnectEvent::Connecting);
        cx.notify();

        let rx = cx.global::<Bridge>().call(move |client| async move {
            let mut client = client.lock().await;
            client.login(&username, &password).await?;
            let refresh = client.refresh_token().map(str::to_owned);
            Ok::<Option<String>, Error>(refresh)
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let reply = match rx.await {
                Ok(reply) => reply,
                Err(_) => Err(Error::Unreachable("request cancelled".to_owned())),
            };
            let _ = this.update(cx, |this, cx| this.finish(reply, cx));
        })
        .detach();
    }

    fn finish(&mut self, reply: Result<Option<String>, Error>, cx: &mut Context<Self>) {
        self.pending = false;
        match reply {
            Ok(_refresh) => {
                // The refresh token is not persisted yet; keychain resume is a later slice.
                self.password
                    .update(cx, |field, cx| field.set_value("", cx));
                cx.emit(ConnectEvent::Connected);
                cx.notify();
            }
            Err(err) => self.fail(&map_error(&err), cx),
        }
    }

    fn fail(&mut self, message: &str, cx: &mut Context<Self>) {
        self.pending = false;
        self.error = Some(message.to_owned());
        cx.emit(ConnectEvent::Failed);
        cx.notify();
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

impl Render for ConnectScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let submit = cx.listener(|this, _: &ClickEvent, _window, cx| this.submit(cx));
        let label = if self.pending {
            "Connecting…"
        } else {
            "Sign in"
        };
        let mut form = div()
            .flex()
            .flex_col()
            .gap_3()
            .w(px(360.0))
            .child(Label::new("Connect to a daemon"))
            .child(field("Daemon URL", self.url.clone()))
            .child(field("Username", self.username.clone()))
            .child(field("Password", self.password.clone()));
        if let Some(message) = &self.error {
            form = form.child(MessageBar::new(message.clone()).intent(MessageIntent::Error));
        }
        form.child(
            Button::new("connect-submit")
                .label(label)
                .appearance(ButtonAppearance::Accent)
                .disabled(self.pending)
                .on_click(submit),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use gpui::{App, Subscription, TestAppContext};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rtbit-client/tests/fixtures"
    );

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
    }

    /// A wiremock daemon whose `/auth/login` answers `status`.
    fn login_server(runtime: &tokio::runtime::Runtime, status: u16) -> MockServer {
        runtime.block_on(async {
            let server = MockServer::start().await;
            let response = if status == 200 {
                ResponseTemplate::new(200).set_body_raw(fixture("login.json"), "application/json")
            } else {
                ResponseTemplate::new(status)
            };
            Mock::given(method("POST"))
                .and(path("/auth/login"))
                .respond_with(response)
                .mount(&server)
                .await;
            server
        })
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    /// Put text into the three fields the way a user would type it.
    fn fill(
        screen: &Entity<ConnectScreen>,
        cx: &mut TestAppContext,
        url: &str,
        user: &str,
        pass: &str,
    ) {
        let (url_field, user_field, pass_field) = screen.read_with(cx, |s, _| {
            (s.url.clone(), s.username.clone(), s.password.clone())
        });
        url_field.update(cx, |field, cx| field.set_value(url, cx));
        user_field.update(cx, |field, cx| field.set_value(user, cx));
        pass_field.update(cx, |field, cx| field.set_value(pass, cx));
    }

    /// Drive gpui until `done` holds. Replies arrive from tokio threads.
    fn drive(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("sign-in reply never arrived");
    }

    #[gpui::test]
    fn empty_fields_show_an_error_without_a_request(cx: &mut TestAppContext) {
        let screen = cx.new(ConnectScreen::new);
        screen.update(cx, |screen, cx| screen.submit(cx));
        screen.read_with(cx, |screen, _| {
            assert!(!screen.is_pending());
            assert_eq!(
                screen.error(),
                Some("Daemon URL, username and password are required.")
            );
        });
    }

    #[gpui::test]
    fn bad_credentials_show_the_error_bar(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        let server = login_server(&runtime, 401);
        let screen = cx.new(ConnectScreen::new);
        fill(&screen, cx, &server.uri(), "admin", "wrong");
        screen.update(cx, |screen, cx| screen.submit(cx));
        assert!(screen.read_with(cx, |s, _| s.is_pending()));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_pending()));
        screen.read_with(cx, |screen, _| {
            assert_eq!(screen.error(), Some("Sign-in was refused."));
        });
    }

    #[gpui::test]
    fn good_credentials_emit_connected_and_clear_the_password(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let runtime = runtime();
        let server = login_server(&runtime, 200);
        let screen = cx.new(ConnectScreen::new);
        let events: Rc<RefCell<Vec<ConnectEvent>>> = Rc::default();
        let sink = events.clone();
        let _subscription: Subscription = cx.update(|cx: &mut App| {
            cx.subscribe(&screen, move |_, event: &ConnectEvent, _| {
                sink.borrow_mut().push(*event);
            })
        });
        fill(&screen, cx, &server.uri(), "admin", "secret");
        screen.update(cx, |screen, cx| screen.submit(cx));
        drive(cx, |cx| screen.read_with(cx, |s, _| !s.is_pending()));
        assert_eq!(
            *events.borrow(),
            vec![ConnectEvent::Connecting, ConnectEvent::Connected]
        );
        screen.read_with(cx, |screen, _| assert!(screen.error().is_none()));
        let pass = screen.read_with(cx, |s, _| s.password.clone());
        assert_eq!(pass.read_with(cx, |field, _| field.text().to_owned()), "");
    }
}
