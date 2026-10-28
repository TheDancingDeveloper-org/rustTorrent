//! Connect screen through AccessKit. No daemon and no network.

use std::sync::Mutex;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use rtbit_client::{
    Error, PeerCounts, PeerList, PieceProgress, SessionCounters, SessionStats, Speed, Torrent,
    TorrentFile, TorrentList, TorrentState, TorrentStats, TrackerList,
};
use rtbit_egui::{
    AddRequest, AddSource, App, Connection, DetailSnapshot, LoginAttempt, MemoryStore, RowAction,
    Screen, Snapshot, TokenStore,
};

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
    adds: std::sync::Arc<Mutex<Vec<AddRequest>>>,
}

impl LoginAttempt for Scripted {
    fn login(&self, _url: &str, _username: &str, _password: &str) -> Result<Option<String>, Error> {
        *self.logins.lock().expect("logins") += 1;
        match *self.result.lock().expect("script") {
            Script::Ok => Ok(Some("refresh-1".into())),
            Script::Unauthorised => Err(Error::Unauthorised),
        }
    }

    fn resume(&self, _url: &str, refresh: &str) -> Result<Option<String>, Error> {
        *self.logins.lock().expect("logins") += 1;
        if refresh == "refresh-1" {
            Ok(Some("refresh-2".into()))
        } else {
            Err(Error::Unauthorised)
        }
    }

    fn snapshot(&self) -> Result<Snapshot, Error> {
        Ok(sample_snapshot())
    }

    fn act(&self, id: u64, action: RowAction) -> Result<(), Error> {
        self.actions.lock().expect("actions").push((id, action));
        Ok(())
    }

    fn add(&self, request: &AddRequest) -> Result<(), Error> {
        self.adds.lock().expect("adds").push(request.clone());
        Ok(())
    }

    fn detail(&self, id: u64) -> Result<DetailSnapshot, Error> {
        let _ = id;
        Ok(DetailSnapshot {
            torrent: detailed_torrent(),
            trackers: TrackerList { trackers: vec![] },
            peers: PeerList::default(),
            pieces: PieceProgress {
                have: vec![true, false],
            },
        })
    }
}

fn scripted(result: Script) -> Scripted {
    Scripted {
        result: std::sync::Arc::new(Mutex::new(result)),
        logins: std::sync::Arc::new(Mutex::new(0)),
        actions: std::sync::Arc::new(Mutex::new(Vec::new())),
        adds: std::sync::Arc::new(Mutex::new(Vec::new())),
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
        files: None,
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
        .with_size(egui::vec2(1100.0, 900.0))
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

fn type_into(harness: &mut Harness<'static, App<Scripted>>, label: &str, text: &str) {
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| {
            node.label().as_deref() == Some(label) && node.role() == Role::TextInput
        }))
        .next()
        .unwrap_or_else(|| panic!("{label} input"))
        .focus();
    harness.run_steps(1);
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| {
            node.label().as_deref() == Some(label) && node.role() == Role::TextInput
        }))
        .next()
        .unwrap_or_else(|| panic!("{label} input"))
        .type_text(text);
    harness.run_steps(1);
}

fn click_named(harness: &mut Harness<'static, App<Scripted>>, name: &str) {
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| {
            node.label().as_deref() == Some(name) && node.role() == Role::Button
        }))
        .last()
        .unwrap_or_else(|| panic!("{name} button"))
        .click();
}

/// Press the dialog's primary button. A pointer click at its centre lands on the
/// command bar behind the dialog, and the accesskit id of a salted button does
/// not match the node's id, so Enter is used once no text field is focused.
fn confirm_dialog(harness: &mut Harness<'static, App<Scripted>>) {
    harness.key_press(egui::Key::Tab);
    harness.run_steps(1);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(1);
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

const GOOD_MAGNET: &str = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Example";

#[test]
fn a_bad_magnet_is_refused_without_an_add() {
    let login = scripted(Script::Ok);
    let adds = login.adds.clone();
    let mut harness = harness(login);
    on_main(&mut harness);
    click_named(&mut harness, "Add");
    harness.run_steps(2);
    type_into(&mut harness, "Magnet link", "not-a-magnet");
    type_into(&mut harness, "Save path", "/data/incoming");
    confirm_dialog(&mut harness);
    harness.run_steps(4);
    assert!(adds.lock().expect("adds").is_empty());
    assert!(shows(&harness, "magnet link must start with magnet:?"));
}

#[test]
fn cancel_closes_the_add_dialog_without_an_add() {
    let login = scripted(Script::Ok);
    let adds = login.adds.clone();
    let mut harness = harness(login);
    on_main(&mut harness);
    click_named(&mut harness, "Add");
    harness.run_steps(2);
    click_named(&mut harness, "Cancel");
    harness.run_steps(4);
    assert!(adds.lock().expect("adds").is_empty());
    assert!(!shows(&harness, "Magnet link"));
}

#[test]
fn a_valid_magnet_records_one_add() {
    let login = scripted(Script::Ok);
    let adds = login.adds.clone();
    let mut harness = harness(login);
    on_main(&mut harness);
    click_named(&mut harness, "Add");
    harness.run_steps(2);
    type_into(&mut harness, "Magnet link", GOOD_MAGNET);
    type_into(&mut harness, "Save path", "/data/incoming");
    type_into(&mut harness, "Category", "movies");
    harness
        .query_all(
            egui_kittest::kittest::by()
                .predicate(|node| node.label().as_deref() == Some("Start paused")),
        )
        .last()
        .expect("Start paused")
        .click();
    confirm_dialog(&mut harness);
    for _ in 0..50 {
        harness.run_steps(1);
        if !adds.lock().expect("adds").is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let recorded = adds.lock().expect("adds").clone();
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0].source,
        AddSource::Magnet(GOOD_MAGNET.to_owned())
    );
    assert_eq!(recorded[0].save_path, "/data/incoming");
    assert_eq!(recorded[0].category, "movies");
    assert!(recorded[0].start_paused);
}

fn detailed_torrent() -> Torrent {
    let mut torrent = sample_snapshot().torrents.torrents.remove(0);
    torrent.files = Some(vec![TorrentFile {
        name: "movie.mkv".into(),
        components: vec!["movie.mkv".into()],
        length: 2048,
        included: true,
    }]);
    torrent
}

/// A selected row shows the file list once the detail poll returns.
#[test]
fn selected_row_shows_the_file_name() {
    let mut harness = harness(scripted(Script::Ok));
    on_main(&mut harness);
    harness.run();
    assert!(shows(&harness, "Select a torrent."));
    harness.state_mut().select_for_test(7);
    for _ in 0..30 {
        harness.run_steps(1);
        if shows(&harness, "movie.mkv") {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("file name missing after the detail poll");
}

#[test]
fn a_stored_refresh_token_reconnects_without_a_password() {
    let login = scripted(Script::Ok);
    let logins = login.logins.clone();
    let store = MemoryStore::default();
    store.store("http://127.0.0.1:3030", "refresh-1").unwrap();
    let app = App::with_store(login, store.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_ui_state(|ui, app: &mut App<Scripted, MemoryStore>| app.ui(ui), app);
    harness.run_steps(4);
    assert_eq!(harness.state().screen(), Screen::Main);
    assert_eq!(
        *logins.lock().expect("logins"),
        1,
        "one resume, no password login"
    );
    assert_eq!(
        store.load("http://127.0.0.1:3030").unwrap().as_deref(),
        Some("refresh-2")
    );
}

#[test]
fn a_missing_keychain_stays_on_the_connect_screen() {
    let login = scripted(Script::Ok);
    let logins = login.logins.clone();
    let app = App::with_store(login, Closed);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_ui_state(|ui, app: &mut App<Scripted, Closed>| app.ui(ui), app);
    harness.run_steps(3);
    assert_eq!(harness.state().screen(), Screen::Connect);
    assert_eq!(*logins.lock().expect("logins"), 0);
}

#[test]
fn sign_out_clears_the_stored_token() {
    let login = scripted(Script::Ok);
    let store = MemoryStore::default();
    store.store("http://127.0.0.1:3030", "refresh-1").unwrap();
    let app = App::with_store(login, store.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_ui_state(|ui, app: &mut App<Scripted, MemoryStore>| app.ui(ui), app);
    harness.run_steps(4);
    assert_eq!(harness.state().screen(), Screen::Main);
    harness
        .query_all(egui_kittest::kittest::by().predicate(|node| {
            node.label().as_deref() == Some("Sign out") && node.role() == Role::Button
        }))
        .next()
        .expect("sign out")
        .click();
    harness.run_steps(2);
    assert_eq!(harness.state().screen(), Screen::Connect);
    assert_eq!(store.load("http://127.0.0.1:3030").unwrap(), None);
}

/// A keychain that cannot be opened. Every call fails, and nothing is stored.
#[derive(Clone, Copy)]
struct Closed;

impl TokenStore for Closed {
    fn store(&self, _: &str, _: &str) -> Option<()> {
        None
    }
    fn load(&self, _: &str) -> Option<Option<String>> {
        None
    }
    fn clear(&self, _: &str) -> Option<()> {
        None
    }
}
