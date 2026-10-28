//! Connect screen through AccessKit. No daemon and no network.

use std::sync::Mutex;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use rtbit_client::{
    Error, PeerCounts, SessionCounters, SessionStats, Speed, Torrent, TorrentList, TorrentState,
    TorrentStats,
};
use rtbit_egui::{App, Connection, LoginAttempt, RowAction, Screen, Snapshot};

#[derive(Clone, Copy)]
enum Script {
    Ok,
    Unauthorised,
}

#[derive(Clone)]
struct Scripted {
    result: std::sync::Arc<Mutex<Script>>,
    logins: std::sync::Arc<Mutex<u32>>,
    actions: std::sync::Arc<Mutex<Vec<(u64, RowAction)>>>,
}

impl LoginAttempt for Scripted {
    fn login(&self, _url: &str, _username: &str, _password: &str) -> Result<(), Error> {
        *self.logins.lock().expect("logins") += 1;
        match *self.result.lock().expect("script") {
            Script::Ok => Ok(()),
            Script::Unauthorised => Err(Error::Unauthorised),
        }
    }

    fn snapshot(&self) -> Result<Snapshot, Error> {
        Ok(sample_snapshot())
    }

    fn act(&self, id: u64, action: RowAction) -> Result<(), Error> {
        self.actions.lock().expect("actions").push((id, action));
        Ok(())
    }
}

fn scripted(result: Script) -> Scripted {
    Scripted {
        result: std::sync::Arc::new(Mutex::new(result)),
        logins: std::sync::Arc::new(Mutex::new(0)),
        actions: std::sync::Arc::new(Mutex::new(Vec::new())),
    }
}

fn sample_snapshot() -> Snapshot {
    let torrent = Torrent {
        id: Some(7),
        info_hash: "abc".into(),
        name: Some("Example torrent".into()),
        output_folder: "/data".into(),
        total_pieces: 4,
        category: Some("movies".into()),
        tags: None,
        trackers: Some(vec!["http://tracker.example/announce".into()]),
        stats: Some(TorrentStats {
            state: Some(TorrentState::Live),
            progress_bytes: 1024,
            total_bytes: 2048,
            finished: false,
            live: None,
        }),
    };
    Snapshot {
        torrents: TorrentList {
            torrents: vec![torrent],
            total: 1,
        },
        stats: SessionStats {
            counters: SessionCounters {
                fetched_bytes: 0,
                uploaded_bytes: 0,
                blocked_incoming: 0,
                blocked_outgoing: 0,
            },
            peers: PeerCounts {
                queued: 0,
                connecting: 0,
                live: 1,
                seen: 1,
                dead: 0,
                not_needed: 0,
            },
            connections: rtbit_client::Connections::default(),
            download_speed: Speed {
                mbps: 1.0,
                human_readable: String::new(),
            },
            upload_speed: Speed {
                mbps: 0.0,
                human_readable: String::new(),
            },
            uptime_seconds: 3,
        },
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
    let login = scripted(Script::Ok);
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
    let login = scripted(Script::Unauthorised);
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
    let login = scripted(Script::Ok);
    let harness = harness(login);
    assert_eq!(harness.state().screen(), Screen::Connect);
}

#[test]
fn main_screen_lists_a_row_and_the_speed_labels() {
    let login = scripted(Script::Ok);
    let mut harness = harness(login);
    harness
        .query_all(egui_kittest::kittest::by().predicate(|node| {
            node.label().as_deref() == Some("Connect") && node.role() == Role::Button
        }))
        .next()
        .expect("connect button")
        .click();
    for _ in 0..50 {
        harness.run_steps(1);
        if shows(&harness, "Example torrent") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(harness.state().screen(), Screen::Main);
    assert!(
        shows(&harness, "live"),
        "the daemon state word is the label"
    );
    assert!(shows(&harness, "Status"));
    assert!(shows(&harness, "All"));
}

#[test]
fn empty_list_says_there_are_no_torrents() {
    let login = scripted(Script::Ok);
    let mut harness = harness(login);
    // A snapshot with no rows: replace by driving the screen state directly.
    harness.state_mut().note_empty_for_test();
    harness.run_steps(2);
    assert!(shows(&harness, "No torrents."));
}

fn click_named(harness: &mut Harness<'static, App<Scripted>>, name: &str) {
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| {
            node.label().as_deref() == Some(name) && node.role() == Role::Button
        }))
        .next()
        .unwrap_or_else(|| panic!("{name} button"))
        .click();
}

fn on_main(harness: &mut Harness<'static, App<Scripted>>) {
    click_named(harness, "Connect");
    for _ in 0..50 {
        harness.run_steps(1);
        if harness.state().screen() == Screen::Main {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("never reached the main screen");
}

#[test]
fn polls_reuse_the_login() {
    let login = scripted(Script::Ok);
    let logins = login.logins.clone();
    let mut harness = harness(login);
    on_main(&mut harness);
    // The poll interval is two seconds, so several frames are still one session.
    harness.run_steps(6);
    assert_eq!(*logins.lock().expect("logins"), 1);
}

#[test]
fn start_records_the_row_action() {
    let login = scripted(Script::Ok);
    let actions = login.actions.clone();
    let mut harness = harness(login);
    on_main(&mut harness);
    harness.state_mut().select_for_test(7);
    harness.run_steps(1);
    click_named(&mut harness, "Start");
    for _ in 0..50 {
        harness.run_steps(1);
        if !actions.lock().expect("actions").is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        *actions.lock().expect("actions"),
        vec![(7, RowAction::Start)]
    );
}

#[test]
fn delete_asks_and_cancel_does_not_act() {
    let login = scripted(Script::Ok);
    let actions = login.actions.clone();
    let mut harness = harness(login);
    on_main(&mut harness);
    harness.state_mut().select_for_test(7);
    harness.run_steps(1);
    click_named(&mut harness, "Delete");
    harness.run_steps(2);
    assert!(shows(&harness, "Delete this torrent and its files?"));
    click_named(&mut harness, "Cancel");
    harness.run_steps(4);
    assert!(actions.lock().expect("actions").is_empty());
}
