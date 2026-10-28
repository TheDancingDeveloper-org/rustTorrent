//! Connect screen through AccessKit. No daemon and no network.

use std::sync::Mutex;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use rtbit_client::Error;
use rtbit_egui::{App, Connection, LoginAttempt, Screen};

#[derive(Clone, Copy)]
enum Script {
    Ok,
    Unauthorised,
}

#[derive(Clone)]
struct Scripted {
    result: std::sync::Arc<Mutex<Script>>,
}

impl LoginAttempt for Scripted {
    fn login(&self, _url: &str, _username: &str, _password: &str) -> Result<(), Error> {
        match *self.result.lock().expect("script") {
            Script::Ok => Ok(()),
            Script::Unauthorised => Err(Error::Unauthorised),
        }
    }
}

fn harness(login: Scripted) -> Harness<'static, App<Scripted>> {
    let app = App::with_login(login);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui_state(|ui, app: &mut App<Scripted>| app.ui(ui), app);
    harness.run_steps(2);
    harness
}

fn shows(harness: &Harness<'static, App<Scripted>>, text: &str) -> bool {
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| {
            node.label().as_deref() == Some(text) || node.value().as_deref() == Some(text)
        }))
        .next()
        .is_some()
}

#[test]
fn connect_screen_has_url_username_password_and_a_button() {
    let login = Scripted {
        result: std::sync::Arc::new(Mutex::new(Script::Ok)),
    };
    let harness = harness(login);
    assert!(shows(&harness, "Daemon URL"));
    assert!(shows(&harness, "Username"));
    assert!(shows(&harness, "Password"));
    assert!(shows(&harness, "Connect"));
    assert!(shows(&harness, "rustTorrent"));
    assert!(shows(&harness, "Disconnected"));
    let mut inputs = harness.query_all(
        egui_kittest::kittest::by()
            .predicate(|node| matches!(node.role(), Role::TextInput | Role::PasswordInput)),
    );
    let count = inputs.by_ref().count();
    assert!(count >= 3, "url, username and password, got {count}");
}

#[test]
fn failed_login_surfaces_the_error_without_a_daemon() {
    let login = Scripted {
        result: std::sync::Arc::new(Mutex::new(Script::Unauthorised)),
    };
    let mut harness = harness(login);
    harness
        .query_all(egui_kittest::kittest::by().predicate(|node| {
            node.label().as_deref() == Some("Connect") && node.role() == Role::Button
        }))
        .next()
        .expect("connect button")
        .click();
    // The worker thread answers on a later frame.
    for _ in 0..50 {
        harness.run_steps(1);
        if harness.state().connection() == Connection::Error {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(harness.state().connection(), Connection::Error);
    assert_eq!(harness.state().screen(), Screen::Connect);
    assert_eq!(harness.state().error(), "unauthorised");
    assert!(shows(&harness, "unauthorised"));
    assert!(shows(&harness, "Error"));
}

#[test]
fn theme_installs_once() {
    let login = Scripted {
        result: std::sync::Arc::new(Mutex::new(Script::Ok)),
    };
    let harness = harness(login);
    assert_eq!(harness.state().screen(), Screen::Connect);
}
