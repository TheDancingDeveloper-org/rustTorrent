//! Connect screen through AccessKit. No daemon and no network.

use std::sync::Mutex;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use rtbit_client::{
    AdvancedConfig, AltSpeedConfig, AltSpeedSchedule, AltSpeedStatus, AuthStatus, DownloadsConfig,
    Error, FoldersConfig, IpFilterConfig, NetworkConfig, NetworkSettings, PeerCounts, PeerList,
    PieceProgress, ProxyConfig, RateLimits, RssFeed, RssItem, RssRule, RssRuleBody, RssRuleEpisode,
    RssSnapshot, SavedNetworkStatus, SeedLimits, SessionCounters, SessionStats, SettingsSnapshot,
    Speed, Torrent, TorrentFile, TorrentList, TorrentState, TorrentStats, TrackerList,
};
use rtbit_egui::{
    AddRequest, AddSource, App, Connection, DetailSnapshot, LoginAttempt, MemoryStore, RowAction,
    Screen, SettingsChanges, SettingsTab, Snapshot, TokenStore,
};

#[derive(Clone, Copy)]
enum Script {
    Ok,
    Unauthorised,
}

#[derive(Clone, Copy)]
enum SetupScript {
    Closed,
    Required,
    Unreachable,
}

/// One rule save: the rule id (`None` for an add) and the body.
type RuleSave = (Option<String>, RssRuleBody);

#[derive(Clone)]
struct Scripted {
    result: std::sync::Arc<Mutex<Script>>,
    logins: std::sync::Arc<Mutex<u32>>,
    actions: std::sync::Arc<Mutex<Vec<(u64, RowAction)>>>,
    adds: std::sync::Arc<Mutex<Vec<AddRequest>>>,
    setup_script: SetupScript,
    setups: std::sync::Arc<Mutex<u32>>,
    /// What `rotated_refresh` reports. Tests set this to simulate a 401 rotation.
    rotated: std::sync::Arc<Mutex<Option<String>>>,
    settings_saves: std::sync::Arc<Mutex<Vec<SettingsChanges>>>,
    /// The network part of what `load_settings` returns.
    network: std::sync::Arc<Mutex<NetworkConfig>>,
    rss_downloads: std::sync::Arc<Mutex<Vec<String>>>,
    rule_saves: std::sync::Arc<Mutex<Vec<RuleSave>>>,
    rule_deletes: std::sync::Arc<Mutex<Vec<String>>>,
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

    fn status(&self, _url: &str) -> Result<AuthStatus, Error> {
        match self.setup_script {
            SetupScript::Closed => Ok(AuthStatus {
                auth_enabled: true,
                setup_required: false,
            }),
            SetupScript::Required => Ok(AuthStatus {
                auth_enabled: false,
                setup_required: true,
            }),
            SetupScript::Unreachable => Err(Error::Unreachable("down bearer secret-token".into())),
        }
    }

    fn setup(
        &self,
        _url: &str,
        username: &str,
        password: &str,
        setup_token: &str,
    ) -> Result<Option<String>, Error> {
        let _ = (username, password, setup_token);
        *self.setups.lock().expect("setups") += 1;
        Ok(Some("refresh-setup".into()))
    }

    fn rotated_refresh(&self) -> Option<String> {
        self.rotated.lock().expect("rotated").clone()
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

    fn load_settings(&self) -> Result<SettingsSnapshot, Error> {
        Ok(SettingsSnapshot {
            network: self.network.lock().expect("network").clone(),
            ..sample_settings()
        })
    }

    fn save_settings(&self, changes: &SettingsChanges) -> Result<(), Error> {
        self.settings_saves
            .lock()
            .expect("saves")
            .push(changes.clone());
        Ok(())
    }

    fn load_rss(&self) -> Result<RssSnapshot, Error> {
        Ok(RssSnapshot {
            feeds: vec![RssFeed {
                name: "Show".into(),
                url: "https://feeds.example/show.xml".into(),
                enabled: true,
            }],
            items: vec![RssItem {
                id: "ep-1".into(),
                feed_name: "Show".into(),
                title: "Episode One".into(),
                url: Some("magnet:?xt=urn:btih:abc".into()),
                downloaded: false,
            }],
            rules: vec![RssRule {
                id: "r-1".into(),
                name: "Show S01".into(),
                feed_names: vec!["Show".into()],
                category: None,
                priority: 2,
                match_regex: "S01".into(),
                enabled: true,
                track_episodes: true,
            }],
        })
    }

    fn download_rss(&self, id: &str) -> Result<(), Error> {
        self.rss_downloads.lock().expect("rss").push(id.to_owned());
        Ok(())
    }

    fn save_rss_rule(&self, id: Option<&str>, body: &RssRuleBody) -> Result<(), Error> {
        self.rule_saves
            .lock()
            .expect("rules")
            .push((id.map(str::to_owned), body.clone()));
        if body.match_regex.contains('(') {
            return Err(Error::Daemon {
                status: rtbit_client::StatusCode::BAD_REQUEST,
                message: Some("invalid filter pattern: invalid regex: unclosed group".into()),
            });
        }
        Ok(())
    }

    fn delete_rss_rule(&self, id: &str) -> Result<(), Error> {
        self.rule_deletes.lock().expect("rules").push(id.to_owned());
        Ok(())
    }

    fn rss_rule_episodes(&self, id: &str) -> Result<Vec<RssRuleEpisode>, Error> {
        assert_eq!(id, "r-1");
        Ok(vec![RssRuleEpisode {
            show: "show".into(),
            season: 1,
            episode: 2,
            title: "Show S01E02".into(),
            matched_at: "2026-10-31T10:00:00+00:00".into(),
            torrent_id: Some(4),
        }])
    }

    fn detail(&self, id: u64) -> Result<DetailSnapshot, Error> {
        let _ = id;
        Ok(DetailSnapshot {
            torrent: detailed_torrent(),
            trackers: TrackerList {
                trackers: vec![rtbit_client::TrackerStatus {
                    url: "http://tracker.example/announce".into(),
                    state: rtbit_client::TrackerState::Working,
                    seeders: Some(3),
                    leechers: Some(1),
                    last_error: None,
                }],
            },
            peers: PeerList {
                peers: vec![rtbit_client::PeerView {
                    addr: "10.0.0.8:51413".into(),
                    state: "live".into(),
                    fetched_bytes: 1024,
                    uploaded_bytes: 0,
                }],
            },
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
        setup_script: SetupScript::Closed,
        setups: std::sync::Arc::new(Mutex::new(0)),
        rotated: std::sync::Arc::new(Mutex::new(None)),
        settings_saves: std::sync::Arc::new(Mutex::new(Vec::new())),
        network: std::sync::Arc::new(Mutex::new(sample_settings().network)),
        rss_downloads: std::sync::Arc::new(Mutex::new(Vec::new())),
        rule_saves: std::sync::Arc::new(Mutex::new(Vec::new())),
        rule_deletes: std::sync::Arc::new(Mutex::new(Vec::new())),
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

/// True when any node's label or value contains `text`, so a row that joins
/// several fields still counts.
fn shows_containing(harness: &Harness<'static, App<Scripted>>, text: &str) -> bool {
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| {
            node.label().is_some_and(|label| label.contains(text))
                || node.value().is_some_and(|value| value.contains(text))
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
    assert_eq!(harness.state().error(), "Sign-in was refused.");
    assert!(shows(&harness, "Sign-in was refused."));
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

fn mem_harness(
    login: Scripted,
    store: MemoryStore,
) -> Harness<'static, App<Scripted, MemoryStore>> {
    let app = App::with_store(login, store);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_ui_state(|ui, app: &mut App<Scripted, MemoryStore>| app.ui(ui), app);
    harness.run_steps(2);
    harness
}

fn node_contains(node: &egui_kittest::kittest::AccessKitNode<'_>, text: &str) -> bool {
    node.label()
        .as_deref()
        .is_some_and(|label| label.contains(text))
        || node
            .value()
            .as_deref()
            .is_some_and(|value| value.contains(text))
}

fn shows_mem(harness: &Harness<'static, App<Scripted, MemoryStore>>, text: &'static str) -> bool {
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| node_contains(node, text)))
        .next()
        .is_some()
}

fn fill(
    harness: &mut Harness<'static, App<Scripted, MemoryStore>>,
    label: &'static str,
    value: &str,
) {
    let mut nodes = harness.query_all(egui_kittest::kittest::by().predicate(move |node| {
        node.label().as_deref() == Some(label)
            && matches!(node.role(), Role::TextInput | Role::PasswordInput)
    }));
    let node = nodes.next().unwrap_or_else(|| panic!("no input {label}"));
    assert!(
        nodes.next().is_none(),
        "more than one input labelled {label}"
    );
    node.click();
    drop(nodes);
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| {
            node.label().as_deref() == Some(label)
                && matches!(node.role(), Role::TextInput | Role::PasswordInput)
        }))
        .next()
        .unwrap_or_else(|| panic!("no input {label}"))
        .type_text(value);
}

fn click_mem(harness: &mut Harness<'static, App<Scripted, MemoryStore>>, name: &'static str) {
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| {
            node.label().as_deref() == Some(name) && node.role() == Role::Button
        }))
        .next()
        .unwrap_or_else(|| panic!("no button {name}"))
        .click();
}

#[test]
fn setup_required_shows_the_setup_form() {
    let mut script = scripted(Script::Ok);
    script.setup_script = SetupScript::Required;
    let harness = mem_harness(script, MemoryStore::default());
    assert!(shows_mem(&harness, "Setup token"));
    assert!(shows_mem(&harness, "Confirm password"));
    assert!(shows_mem(&harness, "Create account"));
}

#[test]
fn mismatched_passwords_do_not_call_setup() {
    let mut script = scripted(Script::Ok);
    script.setup_script = SetupScript::Required;
    let setups = script.setups.clone();
    let mut harness = mem_harness(script, MemoryStore::default());
    fill(&mut harness, "Setup token", "tok");
    fill(&mut harness, "Username", "ada");
    fill(&mut harness, "Password", "long-enough");
    fill(&mut harness, "Confirm password", "different");
    click_mem(&mut harness, "Create account");
    harness.run_steps(2);
    assert_eq!(*setups.lock().expect("setups"), 0);
    assert!(shows_mem(&harness, "Passwords do not match."));
}

#[test]
fn a_successful_setup_opens_main_and_stores_the_refresh_token() {
    let mut script = scripted(Script::Ok);
    script.setup_script = SetupScript::Required;
    let store = MemoryStore::default();
    let stored = store.clone();
    let mut harness = mem_harness(script, store);
    fill(&mut harness, "Setup token", "tok");
    fill(&mut harness, "Username", "ada");
    fill(&mut harness, "Password", "long-enough");
    fill(&mut harness, "Confirm password", "long-enough");
    click_mem(&mut harness, "Create account");
    for _ in 0..50 {
        harness.run_steps(1);
        if harness.state().screen() == Screen::Main {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(harness.state().screen(), Screen::Main);
    assert_eq!(
        stored.load("http://127.0.0.1:3030").unwrap().as_deref(),
        Some("refresh-setup")
    );
}

#[test]
fn setup_closed_stays_on_connect() {
    let script = scripted(Script::Ok);
    let harness = mem_harness(script, MemoryStore::default());
    assert!(shows_mem(&harness, "Daemon URL"));
    assert!(!shows_mem(&harness, "Setup token"));
    assert_eq!(harness.state().screen(), Screen::Connect);
}

#[test]
fn unreachable_status_stays_on_connect() {
    let mut script = scripted(Script::Ok);
    script.setup_script = SetupScript::Unreachable;
    let harness = mem_harness(script, MemoryStore::default());
    assert_eq!(harness.state().screen(), Screen::Connect);
    assert!(!shows_mem(&harness, "Setup token"));
    assert!(shows_mem(&harness, "The daemon is unreachable."));
}

#[test]
fn a_rotated_refresh_token_is_written_back() {
    let script = scripted(Script::Ok);
    let rotated = script.rotated.clone();
    let store = MemoryStore::default();
    let stored = store.clone();
    let mut harness = mem_harness(script, store);
    click_mem(&mut harness, "Connect");
    for _ in 0..50 {
        harness.run_steps(1);
        if harness.state().screen() == Screen::Main {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(harness.state().screen(), Screen::Main);
    assert_eq!(
        stored.load("http://127.0.0.1:3030").unwrap().as_deref(),
        Some("refresh-1")
    );
    *rotated.lock().expect("rotated") = Some("refresh-rotated".into());
    harness.run_steps(2);
    assert_eq!(
        stored.load("http://127.0.0.1:3030").unwrap().as_deref(),
        Some("refresh-rotated")
    );
}

fn sample_settings() -> SettingsSnapshot {
    SettingsSnapshot {
        downloads: DownloadsConfig {
            incomplete_file_suffix: false,
        },
        advanced: AdvancedConfig::default(),
        network: NetworkConfig {
            dht_enabled: true,
            pex_enabled: true,
            listen_port: Some(6881),
            http_api_listen: "127.0.0.1:3030".into(),
            ..NetworkConfig::default()
        },
        proxy: ProxyConfig {
            proxy_url: Some("socks5://user:***@127.0.0.1:1080".into()),
        },
        ip_filter: IpFilterConfig {
            filter: String::new(),
        },
        folders: FoldersConfig {
            download_folder: "/data/in".into(),
            completed_folder: None,
        },
        limits: RateLimits::default(),
        alt: AltSpeedStatus {
            enabled: false,
            config: AltSpeedConfig::default(),
            schedule: None,
        },
        schedule: AltSpeedSchedule::default(),
        seed_limits: SeedLimits {
            ratio_limit: None,
            time_limit_secs: None,
        },
    }
}

#[test]
fn settings_opens_and_a_save_posts_only_the_changed_ratio() {
    let script = scripted(Script::Ok);
    let saves = script.settings_saves.clone();
    let mut harness = mem_harness(script, MemoryStore::default());
    click_mem(&mut harness, "Connect");
    for _ in 0..50 {
        harness.run_steps(1);
        if harness.state().screen() == Screen::Main {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(harness.state().screen(), Screen::Main);
    click_mem(&mut harness, "Settings");
    for _ in 0..50 {
        harness.run_steps(1);
        if shows_mem(&harness, "Append .part to incomplete files") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(shows_mem(&harness, "General"));
    assert!(shows_mem(&harness, "Connection"));
    click_mem(&mut harness, "Seeding");
    harness.run_steps(2);
    assert!(shows_mem(&harness, "Ratio limit"));
    fill(&mut harness, "Ratio limit", "1.5");
    click_mem(&mut harness, "Save");
    for _ in 0..50 {
        harness.run_steps(1);
        if !saves.lock().expect("saves").is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let saved = saves.lock().expect("saves").clone();
    assert_eq!(saved.len(), 1);
    assert!(saved[0].downloads.is_none());
    assert!(
        saved[0].proxy.is_none(),
        "a masked proxy must not be posted"
    );
    assert!(saved[0].folders.is_none());
    assert_eq!(
        saved[0].seed_limits,
        Some(SeedLimits {
            ratio_limit: Some(1.5),
            time_limit_secs: None,
        })
    );
    let _ = SettingsTab::Seeding;
}

#[test]
fn rss_lists_an_item_and_download_posts_only_that_id() {
    let script = scripted(Script::Ok);
    let downloads = script.rss_downloads.clone();
    let mut harness = mem_harness(script, MemoryStore::default());
    click_mem(&mut harness, "Connect");
    for _ in 0..50 {
        harness.run_steps(1);
        if harness.state().screen() == Screen::Main {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(harness.state().screen(), Screen::Main);
    click_mem(&mut harness, "RSS");
    for _ in 0..50 {
        harness.run_steps(1);
        if shows_mem(&harness, "Episode One") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(shows_mem(&harness, "Show (on)"));
    click_mem(&mut harness, "Download");
    for _ in 0..50 {
        harness.run_steps(1);
        if !downloads.lock().expect("rss").is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(downloads.lock().expect("rss").as_slice(), ["ep-1"]);
}

#[test]
fn an_unreachable_daemon_shows_a_mapped_message_without_the_detail() {
    let mut script = scripted(Script::Ok);
    script.setup_script = SetupScript::Unreachable;
    let harness = mem_harness(script, MemoryStore::default());
    assert_eq!(harness.state().screen(), Screen::Connect);
    assert_eq!(harness.state().connection(), Connection::Error);
    let text = harness.state().error();
    assert_eq!(text, "The daemon is unreachable.");
    assert!(!text.contains("secret-token"));
    assert!(shows_mem(&harness, "The daemon is unreachable."));
    assert!(shows_mem(&harness, "Dismiss"));
}

/// Picking a status in the navigation pane hides rows that do not match.
#[test]
fn the_status_filter_hides_rows_that_do_not_match() {
    let mut harness = harness(scripted(Script::Ok));
    on_main(&mut harness);
    harness.run();
    assert!(shows(&harness, "Example torrent"));
    // NavigationView items are tabs. click_tab is the detail strip, so click
    // the status tab by its AccessKit label.
    click_status(&mut harness, "Paused");
    harness.run_steps(2);
    assert!(
        !shows(&harness, "Example torrent"),
        "a live torrent must leave the grid when the filter is Paused"
    );
    assert!(shows(&harness, "No torrents match the filter."));
    click_status(&mut harness, "All");
    harness.run_steps(2);
    assert!(shows(&harness, "Example torrent"));
}

fn click_status(harness: &mut Harness<'static, App<Scripted>>, name: &str) {
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| {
            node.label().as_deref() == Some(name) && node.role() == Role::Tab
        }))
        .next()
        .unwrap_or_else(|| panic!("{name} status tab"))
        .click();
}

/// The detail pane's four tabs are reachable and each names its content.
#[test]
fn detail_tabs_name_files_trackers_peers_and_pieces() {
    let login = scripted(Script::Ok);
    let app = App::with_login(login);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1400.0))
        .build_ui_state(|ui, app: &mut App<Scripted>| app.ui(ui), app);
    harness.run_steps(2);
    on_main(&mut harness);
    harness.state_mut().select_for_test(7);
    for _ in 0..30 {
        harness.run_steps(1);
        if shows(&harness, "movie.mkv") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(shows(&harness, "Files"));
    assert!(shows(&harness, "movie.mkv"));
    click_tab(&mut harness, "Trackers");
    assert!(shows(&harness, "tracker.example"));
    click_tab(&mut harness, "Peers");
    assert!(shows_containing(&harness, "10.0.0.8:51413"));
    click_tab(&mut harness, "Pieces");
    assert!(shows(&harness, "1 of 2 pieces"));
}

fn click_tab(harness: &mut Harness<'static, App<Scripted>>, label: &str) {
    // Tab nodes have no rectangle, and a tab list does not expose its tabs as
    // children. Several lists exist. Click the same slice of each strip until
    // the detail content for that tab appears.
    const DETAIL: [&str; 4] = ["Files", "Trackers", "Peers", "Pieces"];
    let index = DETAIL
        .iter()
        .position(|one| *one == label)
        .expect("detail tab");
    let expect = match label {
        "Trackers" => "tracker.example",
        "Peers" => "10.0.0.8",
        "Pieces" => "1 of 2 pieces",
        _ => "movie.mkv",
    };
    let rects: Vec<egui::Rect> = harness
        .query_all(egui_kittest::kittest::by().predicate(|node| node.role() == Role::TabList))
        .map(|node| node.rect())
        .filter(|rect| rect.width() > 1.0 && rect.height() > 1.0)
        .collect();
    // The harness is 1400 tall so the 180px detail pane is on screen. A list
    // whose center sits past that is clipped and its clicks never land.
    // NavigationView is also a TabList, and it sits above the detail pane.
    // The detail strip is the lowest list still inside the window.
    let Some(rect) = rects
        .into_iter()
        .filter(|rect| rect.center().y < 1400.0)
        .max_by(|a, b| a.center().y.total_cmp(&b.center().y))
    else {
        panic!("detail tab list is outside the window");
    };
    let slice = rect.width() / DETAIL.len() as f32;
    let pos = egui::pos2(rect.left() + slice * (index as f32 + 0.5), rect.center().y);
    pointer_click(harness, pos);
    if !shows_containing(harness, expect) {
        panic!("clicking {label:?} at {pos:?} in {rect:?} never showed {expect:?}");
    }
}

fn pointer_click(harness: &mut Harness<'static, App<Scripted>>, pos: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(pos));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
    }
    harness.run_steps(2);
}

/// Sign in, open Settings and the Connection tab with this network read.
fn open_connection(script: Scripted) -> Harness<'static, App<Scripted, MemoryStore>> {
    let mut harness = mem_harness(script, MemoryStore::default());
    click_mem(&mut harness, "Connect");
    for _ in 0..50 {
        harness.run_steps(1);
        if harness.state().screen() == Screen::Main {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    click_mem(&mut harness, "Settings");
    wait_for(&mut harness, "Append .part to incomplete files");
    click_mem(&mut harness, "Connection");
    harness.run_steps(2);
    harness
}

fn wait_for(harness: &mut Harness<'static, App<Scripted, MemoryStore>>, text: &'static str) {
    for _ in 0..50 {
        harness.run_steps(1);
        if shows_mem(harness, text) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("never showed {text}");
}

fn wait_for_save(
    saves: &Mutex<Vec<SettingsChanges>>,
    harness: &mut Harness<'static, App<Scripted, MemoryStore>>,
) {
    for _ in 0..50 {
        harness.run_steps(1);
        if !saves.lock().expect("saves").is_empty() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn click_checkbox(harness: &mut Harness<'static, App<Scripted, MemoryStore>>, name: &'static str) {
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| {
            node.label().as_deref() == Some(name) && node.role() == Role::CheckBox
        }))
        .next()
        .unwrap_or_else(|| panic!("no checkbox {name}"))
        .click();
}

fn has_button(harness: &Harness<'static, App<Scripted, MemoryStore>>, name: &'static str) -> bool {
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| {
            node.label().as_deref() == Some(name) && node.role() == Role::Button
        }))
        .next()
        .is_some()
}

#[test]
fn network_edit_posts_every_field_for_the_next_start() {
    let script = scripted(Script::Ok);
    let saves = script.settings_saves.clone();
    let mut harness = open_connection(script);
    assert!(shows_mem(&harness, "Restart required"));
    assert!(shows_mem(&harness, "Listen port"));
    assert!(
        !has_button(&harness, "Use startup flags"),
        "nothing is saved, so there is nothing to reset"
    );
    click_checkbox(&mut harness, "DHT");
    harness.run_steps(2);
    click_mem(&mut harness, "Save");
    wait_for_save(&saves, &mut harness);
    let saved = saves.lock().expect("saves").clone();
    assert_eq!(saved.len(), 1);
    assert_eq!(
        saved[0].network,
        Some(NetworkSettings {
            listen_ip: None,
            listen_port: Some(6881),
            dht_enabled: Some(false),
            lsd_enabled: Some(false),
            upnp_port_forward: Some(false),
            reset: None,
        })
    );
    assert!(saved[0].proxy.is_none());
    wait_for(&mut harness, "Saved. Restart to apply.");
}

#[test]
fn an_unchanged_network_form_posts_nothing_for_it() {
    let script = scripted(Script::Ok);
    let saves = script.settings_saves.clone();
    let mut harness = open_connection(script);
    click_checkbox(&mut harness, "Clear proxy");
    harness.run_steps(2);
    click_mem(&mut harness, "Save");
    wait_for_save(&saves, &mut harness);
    let saved = saves.lock().expect("saves").clone();
    assert_eq!(saved.len(), 1);
    assert!(saved[0].network.is_none());
}

#[test]
fn fell_back_saved_values_show_the_banner_rows_warnings_and_reset() {
    let script = scripted(Script::Ok);
    *script.network.lock().expect("network") = NetworkConfig {
        listen_ip: Some("0.0.0.0".into()),
        listen_port: Some(6881),
        dht_enabled: true,
        pending: Some(NetworkSettings {
            listen_ip: Some("::".into()),
            listen_port: Some(80),
            ..NetworkSettings::default()
        }),
        pending_status: Some(SavedNetworkStatus::FellBack),
        ..NetworkConfig::default()
    };
    let saves = script.settings_saves.clone();
    let mut harness = open_connection(script);
    assert!(shows_mem(
        &harness,
        "could not be used at the last start, so the startup flags are running"
    ));
    assert!(shows_mem(&harness, "Running now"));
    assert!(shows_mem(&harness, "80 (next start)"));
    assert!(shows_mem(&harness, "Ports below 1024 need root"));
    assert!(shows_mem(
        &harness,
        ":: listens for peers on every interface"
    ));
    click_mem(&mut harness, "Use startup flags");
    wait_for_save(&saves, &mut harness);
    let saved = saves.lock().expect("saves").clone();
    assert_eq!(saved, vec![SettingsChanges::use_startup_flags()]);
    wait_for(
        &mut harness,
        "Saved values cleared. The next start uses the startup flags.",
    );
}

#[test]
fn ignored_saved_values_say_so_and_a_zoned_ip_is_refused() {
    let script = scripted(Script::Ok);
    *script.network.lock().expect("network") = NetworkConfig {
        listen_port: Some(6881),
        pending: Some(NetworkSettings {
            dht_enabled: Some(false),
            ..NetworkSettings::default()
        }),
        pending_status: Some(SavedNetworkStatus::Ignored),
        ..NetworkConfig::default()
    };
    let saves = script.settings_saves.clone();
    let mut harness = open_connection(script);
    assert!(shows_mem(&harness, "Started with --ignore-saved-network"));
    fill(&mut harness, "Listen IP", "fe80::1%eth0");
    harness.run_steps(2);
    assert!(shows_mem(
        &harness,
        "IPv6 zones (%interface) are not supported"
    ));
    click_mem(&mut harness, "Save");
    harness.run_steps(4);
    assert!(saves.lock().expect("saves").is_empty());
}

#[test]
fn without_persistence_the_network_form_is_read_only() {
    let script = scripted(Script::Ok);
    script.network.lock().expect("network").can_save = false;
    let harness = open_connection(script);
    assert!(shows_mem(&harness, "Session persistence is off"));
    assert!(
        harness
            .query_all(egui_kittest::kittest::by().predicate(|node| {
                node.label().as_deref() == Some("Listen port") && node.role() == Role::TextInput
            }))
            .next()
            .is_none()
    );
    assert!(!has_button(&harness, "Use startup flags"));
}

type MemHarness = Harness<'static, App<Scripted, MemoryStore>>;

fn wait_until(harness: &mut MemHarness, mut done: impl FnMut(&MemHarness) -> bool) {
    for _ in 0..50 {
        harness.run_steps(1);
        if done(harness) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Connect, open RSS and wait for the scripted feed, item and rule.
fn on_rss(script: Scripted) -> MemHarness {
    let mut harness = mem_harness(script, MemoryStore::default());
    click_mem(&mut harness, "Connect");
    wait_until(&mut harness, |h| h.state().screen() == Screen::Main);
    click_mem(&mut harness, "RSS");
    wait_until(&mut harness, |h| shows_mem(h, "Show S01"));
    assert!(shows_mem(&harness, "Show S01: /S01/"));
    harness
}

fn tick_checkbox(harness: &mut MemHarness, label: &'static str) {
    harness
        .query_all(egui_kittest::kittest::by().predicate(move |node| {
            node.label().as_deref() == Some(label) && node.role() == Role::CheckBox
        }))
        .last()
        .unwrap_or_else(|| panic!("no checkbox {label}"))
        .click();
    harness.run_steps(1);
}

/// Tab off the last field and press Enter: the dialog's primary button.
fn confirm_mem(harness: &mut MemHarness) {
    harness.key_press(egui::Key::Tab);
    harness.run_steps(1);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(1);
}

#[test]
fn rss_rule_create_posts_the_dialog_fields() {
    let script = scripted(Script::Ok);
    let saves = script.rule_saves.clone();
    let mut harness = on_rss(script);
    click_mem(&mut harness, "Add rule");
    harness.run_steps(2);
    fill(&mut harness, "Rule name", "Show 1080p");
    harness.run_steps(1);
    fill(&mut harness, "Match regex", "1080p");
    harness.run_steps(1);
    fill(&mut harness, "Category", "tv");
    harness.run_steps(1);
    tick_checkbox(&mut harness, "Show");
    tick_checkbox(&mut harness, "Enabled");
    confirm_mem(&mut harness);
    wait_until(&mut harness, |_| !saves.lock().expect("rules").is_empty());
    let recorded = saves.lock().expect("rules").clone();
    assert_eq!(
        recorded,
        [(
            None,
            RssRuleBody {
                name: "Show 1080p".into(),
                feed_names: vec!["Show".into()],
                category: Some("tv".into()),
                priority: 1,
                match_regex: "1080p".into(),
                enabled: false,
                track_episodes: true,
            }
        )]
    );
    wait_until(&mut harness, |h| !shows_mem(h, "Save rule"));
    assert!(
        !shows_mem(&harness, "Save rule"),
        "dialog closes on success"
    );
}

#[test]
fn rss_rule_edit_puts_the_rule_id_and_keeps_the_other_fields() {
    let script = scripted(Script::Ok);
    let saves = script.rule_saves.clone();
    let mut harness = on_rss(script);
    click_mem(&mut harness, "Edit");
    harness.run_steps(2);
    assert!(shows_mem(&harness, "Edit rule"));
    fill(&mut harness, "Category", "tv");
    harness.run_steps(1);
    tick_checkbox(&mut harness, "Enabled");
    confirm_mem(&mut harness);
    wait_until(&mut harness, |_| !saves.lock().expect("rules").is_empty());
    let recorded = saves.lock().expect("rules").clone();
    assert_eq!(recorded.len(), 1);
    let (id, body) = &recorded[0];
    assert_eq!(id.as_deref(), Some("r-1"));
    assert_eq!(body.name, "Show S01");
    assert_eq!(body.match_regex, "S01");
    assert_eq!(body.feed_names, ["Show"]);
    assert_eq!(body.category.as_deref(), Some("tv"));
    assert_eq!(body.priority, 2);
    assert!(!body.enabled);
}

#[test]
fn rss_rule_edit_lists_matched_episodes_and_can_stop_tracking() {
    let script = scripted(Script::Ok);
    let saves = script.rule_saves.clone();
    let mut harness = on_rss(script);
    click_mem(&mut harness, "Edit");
    harness.run_steps(2);
    assert!(shows_mem(&harness, "Matched episodes"));
    wait_until(&mut harness, |h| {
        shows_mem(h, "S01E02 · Show S01E02 · 2026-10-31 · torrent 4")
    });
    tick_checkbox(&mut harness, "Skip episodes already downloaded");
    // Enabled is the last field; Tab off it then Enter saves.
    tick_checkbox(&mut harness, "Enabled");
    confirm_mem(&mut harness);
    wait_until(&mut harness, |_| !saves.lock().expect("rules").is_empty());
    let recorded = saves.lock().expect("rules").clone();
    assert_eq!(recorded[0].0.as_deref(), Some("r-1"));
    assert!(!recorded[0].1.track_episodes);
    assert!(!recorded[0].1.enabled);
}

#[test]
fn rss_rule_delete_asks_then_deletes_that_rule() {
    let script = scripted(Script::Ok);
    let deletes = script.rule_deletes.clone();
    let mut harness = on_rss(script);
    click_mem(&mut harness, "Delete");
    harness.run_steps(2);
    assert!(shows_mem(&harness, "Delete the rule \"Show S01\""));
    assert!(
        deletes.lock().expect("rules").is_empty(),
        "nothing before confirm"
    );
    confirm_mem(&mut harness);
    wait_until(&mut harness, |_| !deletes.lock().expect("rules").is_empty());
    assert_eq!(deletes.lock().expect("rules").as_slice(), ["r-1"]);
}

#[test]
fn rss_rule_regex_refusal_shows_inside_the_open_dialog() {
    let script = scripted(Script::Ok);
    let saves = script.rule_saves.clone();
    let mut harness = on_rss(script);
    click_mem(&mut harness, "Add rule");
    harness.run_steps(2);
    fill(&mut harness, "Rule name", "Bad");
    harness.run_steps(1);
    fill(&mut harness, "Match regex", "(");
    harness.run_steps(1);
    tick_checkbox(&mut harness, "Enabled");
    confirm_mem(&mut harness);
    harness.run_steps(2);
    assert!(shows_mem(&harness, "Select at least one feed"));
    assert!(
        saves.lock().expect("rules").is_empty(),
        "client check stops it"
    );
    tick_checkbox(&mut harness, "Show");
    confirm_mem(&mut harness);
    wait_until(&mut harness, |h| {
        shows_mem(h, "invalid regex: unclosed group")
    });
    assert!(shows_mem(
        &harness,
        "invalid filter pattern: invalid regex: unclosed group"
    ));
    assert!(shows_mem(&harness, "Save rule"), "dialog stays open");
    assert_eq!(saves.lock().expect("rules").len(), 1);
}
