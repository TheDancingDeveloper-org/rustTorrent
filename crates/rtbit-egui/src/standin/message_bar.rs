//! Local stand-in for a fluent2 message bar.
//!
//! A single dismissible strip. Replaced when `surfaces` grows one (design
//! item 14). The text is already mapped; this widget does not format errors.

/// One message the user can dismiss.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageBar {
    text: String,
}

impl MessageBar {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
    }

    pub fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    /// Draw the bar. Returns true when the user dismisses it.
    pub fn show(&mut self, ui: &mut egui::Ui) -> bool {
        if self.text.is_empty() {
            return false;
        }
        let mut dismissed = false;
        ui.horizontal(|ui| {
            ui.colored_label(egui::Color32::from_rgb(196, 43, 28), &self.text);
            if ui.button("Dismiss").clicked() {
                dismissed = true;
            }
        });
        if dismissed {
            self.text.clear();
        }
        dismissed
    }
}

/// Words for a daemon error. Never includes a token or a response body.
pub fn map_error(err: &rtbit_client::Error) -> String {
    match err {
        rtbit_client::Error::Unreachable(_) => "The daemon is unreachable.".to_owned(),
        rtbit_client::Error::Unauthorised => "Sign-in was refused.".to_owned(),
        rtbit_client::Error::NotSetUp => "The daemon is not set up yet.".to_owned(),
        rtbit_client::Error::Daemon { status } => {
            format!("The daemon returned an error ({status}).")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::map_error;
    use rtbit_client::Error;

    #[test]
    fn mapped_text_drops_the_transport_detail() {
        let raw = "connection refused, bearer eyJhbGciOi-secret, body: {\"token\":\"abc\"}";
        let text = map_error(&Error::Unreachable(raw.to_owned()));
        assert_eq!(text, "The daemon is unreachable.");
        assert!(!text.contains("eyJ"));
        assert!(!text.contains("token"));
        assert!(!text.contains(raw));
    }
}
