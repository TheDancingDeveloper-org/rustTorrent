//! Smoke tests: drive the mockup headless with egui_kittest and check the main flows work.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use rtbit_fluent_mockup::{Launch, MockupApp, Screen, StartDialog};

fn harness(launch: Launch) -> Harness<'static, MockupApp> {
    Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(
            |ui, app: &mut MockupApp| app.show(ui),
            MockupApp::with_launch(launch),
        )
}

#[test]
fn torrents_screen_renders_and_opens_add_dialog() {
    let mut h = harness(Launch::default());
    h.run_steps(3);
    assert!(h.get_all_by_label("Torrents").count() >= 1);
    h.get_by_label("Add magnet").click();
    h.run_steps(3);
    h.get_by_label("Add torrent");
}

#[test]
fn login_signs_in() {
    let mut h = harness(Launch {
        screen: Screen::Login,
        ..Default::default()
    });
    h.run_steps(2);
    assert_eq!(h.state().screen(), Screen::Login);
    h.state_mut().set_password("hunter2");
    h.get_by_label("Sign In").click();
    h.run_steps(3);
    assert_eq!(h.state().screen(), Screen::Torrents);
}

#[test]
fn every_settings_tab_renders() {
    let mut h = harness(Launch {
        dialog: Some(StartDialog::Settings(0)),
        ..Default::default()
    });
    h.run_steps(2);
    for tab in 0..10 {
        h.state_mut().set_settings_tab(tab);
        h.run_steps(2);
        assert!(h.state().settings_open());
    }
}

#[test]
fn rss_and_logs_render_in_dark() {
    for screen in [Screen::Rss, Screen::Logs] {
        let mut h = harness(Launch {
            screen,
            dark: true,
            ..Default::default()
        });
        h.run_steps(3);
        assert_eq!(h.state().theme_kind(), fluent2_egui::ThemeKind::Dark);
    }
}
