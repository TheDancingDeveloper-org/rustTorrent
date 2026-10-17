// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod launch;
mod tray;
mod updates;

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::Context;
use config::RtbitDesktopConfig;
use librqbit_dualstack_sockets::TcpListener;
use librtbit::{
    Api, Session, SessionOptions, SessionPersistenceConfig, TorrentStatsState,
    api::ApiTorrentListOpts,
    dht::PersistentDhtConfig,
    rss::db::RssDatabase,
    tracing_subscriber_config_utils::{InitLoggingOptions, InitLoggingResult, init_logging},
};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tracing::{debug_span, error, info, warn};

// ---------------------------------------------------------------------------
// Engine state — shared via Tauri's managed state
// ---------------------------------------------------------------------------

pub struct EngineState {
    pub api: Api,
    pub listen_addr: SocketAddr,
}

/// Where the desktop config lives. The config is read and written here.
fn config_file_path() -> PathBuf {
    directories::ProjectDirs::from("com", "rtbit", "desktop")
        .expect("directories::ProjectDirs::from")
        .config_dir()
        .join("config.json")
}

// ---------------------------------------------------------------------------
// Config helpers (unchanged from previous implementation)
// ---------------------------------------------------------------------------

fn read_config(path: &str) -> anyhow::Result<RtbitDesktopConfig> {
    let rdr = BufReader::new(File::open(path)?);
    let mut config: RtbitDesktopConfig = serde_json::from_reader(rdr)?;
    config.persistence.fix_backwards_compat();
    Ok(config)
}

fn write_config(path: &str, config: &RtbitDesktopConfig) -> anyhow::Result<()> {
    std::fs::create_dir_all(Path::new(path).parent().context("no parent")?)
        .context("error creating dirs")?;
    let tmp = format!("{}.tmp", path);
    let mut tmp_file = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(&tmp)?,
    );
    serde_json::to_writer(&mut tmp_file, config)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Engine startup (Session + HTTP API + RSS)
// ---------------------------------------------------------------------------

async fn api_from_config(
    init_logging: &InitLoggingResult,
    config: &RtbitDesktopConfig,
) -> anyhow::Result<(Api, SocketAddr, Option<String>)> {
    config
        .validate()
        .context("error validating configuration")?;

    let persistence = if config.persistence.disable {
        None
    } else {
        Some(SessionPersistenceConfig::Json {
            folder: if config.persistence.folder == Path::new("") {
                None
            } else {
                Some(config.persistence.folder.clone())
            },
        })
    };

    let (listen, connect) = config.connections.as_listener_and_connect_opts();

    // Credentials and sessions live in the desktop config directory.
    let config_dir = directories::ProjectDirs::from("com", "rtbit", "desktop")
        .expect("directories::ProjectDirs::from")
        .config_dir()
        .to_owned();
    // Refuses to start if credentials.json is corrupt, rather than running unauthenticated.
    let credential_store = Arc::new(librtbit::http_api::auth::CredentialStore::load(
        config_dir.clone(),
    )?);
    // Set only while setup is pending. The webview gets it directly, so a Windows build with
    // no console can still finish first-time setup.
    let setup_token = credential_store.setup_token().map(str::to_owned);

    // Stored credentials are checked against their hash by the HTTP API, so they are not
    // copied into basic_auth.
    // Always create token store (needed for setup flow and login)
    let token_store = Some(Arc::new(librtbit::http_api::auth::TokenStore::load(
        config_dir,
    )));

    let mut http_api_opts = librtbit::http_api::HttpApiOptions {
        read_only: config.http_api.read_only,
        basic_auth: None,
        token_store,
        credential_store: Some(credential_store),
        ..Default::default()
    };

    // Install prometheus recorder before session creation.
    if !config.http_api.disable {
        match metrics_exporter_prometheus::PrometheusBuilder::new().install_recorder() {
            Ok(handle) => {
                http_api_opts.prometheus_handle = Some(handle);
            }
            Err(e) => {
                warn!("error installing prometheus recorder: {e:#}");
            }
        }
    }

    let session = Session::new_with_opts(
        config.default_download_location.clone(),
        SessionOptions {
            disable_dht: config.dht.disable,
            disable_dht_persistence: config.dht.disable_persistence,
            dht_config: Some(PersistentDhtConfig {
                config_filename: Some(config.dht.persistence_filename.clone()),
                ..Default::default()
            }),
            persistence,
            connect: Some(connect),
            listen,
            fastresume: config.persistence.fastresume,
            fastresume_validation_denom: config.persistence.fastresume_validation_denom,
            ratelimits: config.ratelimits,
            completed_folder: config.completed_folder.clone(),
            #[cfg(feature = "disable-upload")]
            disable_upload: config.disable_upload,
            ..Default::default()
        },
    )
    .await
    .context("couldn't set up librtbit session")?;

    let api = Api::new(
        session.clone(),
        Some(init_logging.rust_log_reload_tx.clone()),
        Some(init_logging.line_broadcast.clone()),
    );

    // Initialize RSS database alongside session persistence.
    let rss_db = {
        let rss_dir = if config.persistence.folder == std::path::Path::new("") {
            SessionPersistenceConfig::default_json_persistence_folder()?
        } else {
            config.persistence.folder.clone()
        };
        let rss_db_path = rss_dir.join("rss.db");
        match RssDatabase::open(&rss_db_path) {
            Ok(db) => {
                let db = std::sync::Arc::new(parking_lot::Mutex::new(db));
                info!("RSS database opened at {}", rss_db_path.display());

                let monitor = librtbit::rss::monitor::RssMonitor::new(
                    db.clone(),
                    api.clone(),
                    config.rss_history_limit,
                );
                tokio::spawn(async move { monitor.run().await });
                info!("RSS monitor started");

                Some(db)
            }
            Err(e) => {
                warn!("Failed to open RSS database: {:#}", e);
                None
            }
        }
    };

    http_api_opts.rss_db = rss_db;
    http_api_opts.rss_history_limit = config.rss_history_limit;

    let listen_addr = config.http_api.listen_addr;

    if !config.http_api.disable {
        let api = api.clone();
        let upnp_router = if config.upnp.enable_server {
            let friendly_name = config
                .upnp
                .server_friendly_name
                .as_ref()
                .map(|f| f.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_owned())
                .unwrap_or_else(|| {
                    format!(
                        "rtbit-desktop@{}",
                        gethostname::gethostname().to_string_lossy()
                    )
                });

            let mut upnp_adapter = session
                .make_upnp_adapter(friendly_name, listen_addr.port())
                .await
                .context("error starting UPnP server")?;
            let router = upnp_adapter.take_router()?;
            session.spawn(debug_span!("ssdp"), "ssdp", async move {
                upnp_adapter.run_ssdp_forever().await
            });
            Some(router)
        } else {
            None
        };

        let http_api_task = async move {
            let listener = TcpListener::bind_tcp(listen_addr, Default::default())
                .with_context(|| format!("error listening on {}", listen_addr))?;
            librtbit::http_api::HttpApi::new(api.clone(), Some(http_api_opts))
                .make_http_api_and_run(listener, upnp_router)
                .await
        };

        session.spawn(debug_span!("http_api"), "http_api", http_api_task);
    }

    Ok((api, listen_addr, setup_token))
}

/// Start the engine, HTTP server, and wait for readiness.
async fn start_engine(
    init_logging: &InitLoggingResult,
) -> anyhow::Result<(Api, SocketAddr, Option<String>)> {
    let config_path = config_file_path();
    let config_filename = config_path.display().to_string();

    let config = match read_config(&config_filename) {
        Ok(config) => {
            info!("Loaded config from {}", config_filename);
            config
        }
        Err(e) => {
            info!("No config found ({e:#}), creating default at {config_filename}");
            let config = RtbitDesktopConfig::default();
            if let Err(e) = write_config(&config_filename, &config) {
                warn!("Failed to write default config: {e:#}");
            }
            config
        }
    };

    let (api, listen_addr, setup_token) = api_from_config(init_logging, &config).await?;

    // Wait for HTTP server readiness by polling /stats.
    let health_url = format!("http://{listen_addr}/stats");
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Ok(resp) = reqwest::get(&health_url).await
            && resp.status().is_success()
        {
            info!("HTTP server ready on {listen_addr}");
            return Ok((api, listen_addr, setup_token));
        }
    }

    // Server may still work even if polling didn't succeed.
    warn!("Could not confirm HTTP server readiness, continuing anyway");
    Ok((api, listen_addr, setup_token))
}

// ---------------------------------------------------------------------------
// Notification monitor — detect download complete / failed
// ---------------------------------------------------------------------------

fn spawn_notification_monitor(app: &AppHandle) {
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // Map of torrent id -> (finished, is_error)
        let mut known_states: HashMap<usize, (bool, bool)> = HashMap::new();
        let mut interval = tokio::time::interval(Duration::from_secs(5));

        loop {
            interval.tick().await;
            let Some(state) = app_handle.try_state::<EngineState>() else {
                continue;
            };

            let list = state.api.api_torrent_list_ext(ApiTorrentListOpts {
                with_stats: true,
                ..Default::default()
            });

            for torrent in &list.torrents {
                let (Some(id), Some(stats)) = (torrent.id, &torrent.stats) else {
                    continue;
                };
                let name = torrent
                    .name
                    .as_deref()
                    .unwrap_or("Unknown torrent")
                    .to_string();
                let is_error = matches!(stats.state, TorrentStatsState::Error);

                if let Some(&(was_finished, was_error)) = known_states.get(&id) {
                    // Detect completion transition.
                    if stats.finished && !was_finished {
                        send_notification(&app_handle, "Download Complete", &name);
                    }
                    // Detect error transition.
                    if is_error && !was_error {
                        let msg = stats.error.as_deref().unwrap_or("Unknown error");
                        send_notification(
                            &app_handle,
                            "Download Failed",
                            &format!("{}: {}", name, msg),
                        );
                    }
                }

                known_states.insert(id, (stats.finished, is_error));
            }

            // Clean up removed torrents.
            let current_ids: std::collections::HashSet<usize> =
                list.torrents.iter().filter_map(|t| t.id).collect();
            known_states.retain(|k, _| current_ids.contains(k));
        }
    });
}

pub(crate) fn send_notification(app: &AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        warn!("Failed to send notification: {e}");
    }
}

// ---------------------------------------------------------------------------
// Tauri commands (minimal — desktop-specific only)
// ---------------------------------------------------------------------------

#[tauri::command]
fn cmd_open_in_browser(state: tauri::State<'_, EngineState>) {
    let url = format!("http://{}", state.listen_addr);
    let _ = open::that(&url);
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

/// Shown when the engine cannot start. Without this a GUI user sees nothing:
/// the message only ever went to stderr, which a double-clicked app never shows.
fn show_engine_failure(app: &AppHandle, cause: &str) {
    let message = format!(
        "rtbit could not start its engine.\n\n{cause}\n\n\
         If the error mentions an address already in use, another program or another rtbit \
         is using the HTTP API port (http_api.listen_addr, default 127.0.0.1:3030) or the \
         peer port (connections.listen_port, default 4240).\n\n\
         Config: {}\n\
         Logs go to stderr; start rtbit from a terminal to see them.",
        config_file_path().display()
    );
    let handle = app.clone();
    app.dialog()
        .message(message)
        .title("rtbit failed to start")
        .kind(MessageDialogKind::Error)
        .show(move |_| handle.exit(1));
}

/// Registers the magnet scheme at runtime. Installed builds get it from the bundle.
#[cfg(all(debug_assertions, any(windows, target_os = "linux")))]
fn register_dev_deep_links(app: &AppHandle) {
    use tauri_plugin_deep_link::DeepLinkExt;
    if let Err(e) = app.deep_link().register("magnet") {
        warn!("could not register magnet: scheme for development: {e}");
    }
}

fn run_app() {
    let init_logging_result = init_logging(InitLoggingOptions {
        default_rust_log_value: Some("info"),
        log_file: None,
        log_file_rust_log: None,
    })
    .unwrap();

    match librtbit::try_increase_nofile_limit() {
        Ok(limit) => info!(limit = limit, "increased open file limit"),
        Err(e) => warn!("failed increasing open file limit: {:#}", e),
    };

    let argv: Vec<String> = std::env::args().collect();
    let start_hidden = launch::starts_hidden(&argv);

    tauri::Builder::default()
        // Must be first. A second instance hands its argv (a .torrent or magnet:
        // link to open) to this one and exits here, before it starts an engine
        // that would fight for the same ports.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            tray::show_main_window(app);
            let targets = launch::targets_from_args(&argv, std::path::Path::new(&cwd));
            launch::spawn_add(app.clone(), targets);
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![launch::AUTOSTART_ARG]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![cmd_open_in_browser])
        .setup(move |app| {
            // The engine starts here, after the single-instance check above, and
            // before the window that needs its address.
            let (api, listen_addr, setup_token) =
                match tauri::async_runtime::block_on(start_engine(&init_logging_result)) {
                    Ok(engine) => engine,
                    Err(e) => {
                        error!("Failed to start engine: {e:#}");
                        show_engine_failure(app.handle(), &format!("{e:#}"));
                        return Ok(());
                    }
                };
            app.manage(EngineState { api, listen_addr });

            #[cfg(all(debug_assertions, any(windows, target_os = "linux")))]
            register_dev_deep_links(app.handle());

            // Create main window pointing at the HTTP server's web UI.
            let url = WebviewUrl::External(format!("http://{listen_addr}/web/").parse()?);
            let mut builder = WebviewWindowBuilder::new(app, "main", url)
                .title("rtbit")
                .inner_size(1280.0, 800.0)
                .visible(!start_hidden);
            if let Some(token) = &setup_token {
                // Only this webview sees the token; the setup page reads it and fills the field.
                let script = format!(
                    "window.__RTBIT_SETUP_TOKEN__ = {};",
                    serde_json::to_string(token)?
                );
                builder = builder.initialization_script(&script);
            }
            let window = builder.build()?;

            // Close hides to the tray instead of quitting. Quit is in the tray menu.
            let close_handle = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event
                    && tray::is_available(close_handle.app_handle())
                {
                    api.prevent_close();
                    let _ = close_handle.hide();
                }
            });

            // Set up system tray.
            let tray_ready = match tray::setup(app.handle()) {
                Ok(()) => true,
                Err(e) => {
                    warn!("Failed to set up system tray: {e}");
                    false
                }
            };
            // A hidden start is only safe with a tray to bring the window back from.
            if start_hidden && !tray_ready {
                let _ = window.show();
            }

            // Start notification monitor for download complete/failed.
            spawn_notification_monitor(app.handle());

            // Anything the OS handed us on the first launch (a .torrent or magnet: link).
            launch::spawn_add(
                app.handle().clone(),
                launch::targets_from_args(&argv, &std::env::current_dir()?),
            );

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn main() {
    // The runtime lives for the whole process; Tauri's event loop runs on this thread.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("couldn't set up tokio runtime");
    tauri::async_runtime::set(runtime.handle().clone());
    run_app();
}
