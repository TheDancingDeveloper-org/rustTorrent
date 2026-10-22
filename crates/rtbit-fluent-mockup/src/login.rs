//! The login page: a centred card with username, password and "remember me".

use egui::{Align, Layout, Ui};
use fluent2_egui::controls::{Appearance, Button};
use fluent2_egui::surfaces::{Intent, MessageBar};
use fluent2_egui::{Checkbox, Field, Input, TypeRamp, rich};

#[derive(Default)]
pub struct LoginState {
    pub username: String,
    pub password: String,
    pub remember: bool,
    pub error: bool,
}

/// Returns true when the user signs in. Any non-empty password is accepted, except "wrong".
pub fn show(ui: &mut Ui, state: &mut LoginState) -> bool {
    let mut signed_in = false;
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space((ui.available_height() * 0.18).max(24.0));
        ui.set_max_width(380.0);
        crate::widgets::card(ui, |ui| {
            ui.set_width(340.0);
            ui.vertical(|ui| {
                ui.label(rich("rustTorrent", TypeRamp::Title2));
                ui.label(rich("Sign in to continue", TypeRamp::Body1));
                ui.add_space(12.0);
                if state.error {
                    MessageBar::new(Intent::Error, "Invalid username or password.").show(ui);
                    ui.add_space(8.0);
                }
                Field::new("Username").required(true).show(ui, |ui| {
                    ui.add(
                        Input::new(&mut state.username)
                            .placeholder("admin")
                            .desired_width(320.0),
                    );
                });
                ui.add_space(6.0);
                let submit = Field::new("Password").required(true).show(ui, |ui| {
                    let r = ui.add(
                        Input::new(&mut state.password)
                            .password()
                            .placeholder("Password")
                            .desired_width(320.0),
                    );
                    r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                });
                ui.add_space(6.0);
                ui.add(Checkbox::bool(
                    &mut state.remember,
                    "Keep me signed in for 30 days",
                ));
                ui.add_space(12.0);
                let clicked = ui
                    .add(
                        Button::new("Sign In")
                            .appearance(Appearance::Primary)
                            .min_width(320.0),
                    )
                    .clicked();
                if clicked || submit {
                    state.error = state.password.is_empty() || state.password == "wrong";
                    signed_in = !state.error;
                }
                ui.add_space(8.0);
                ui.label(rich("Mockup: any password signs in.", TypeRamp::Caption1));
            });
        });
    });
    signed_in
}
