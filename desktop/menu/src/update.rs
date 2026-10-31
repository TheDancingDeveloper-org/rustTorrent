//! The manual update check's decisions, separate from the updater plugin so
//! they can be tested against a fake release feed.

/// A release the update feed offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub version: String,
    pub current_version: String,
}

/// Where an update check gets its answer. The desktop binary wraps the Tauri
/// updater; tests use a fake.
pub trait UpdateFeed {
    /// `Ok(None)` when no newer release exists.
    fn check(&mut self) -> Result<Option<AvailableUpdate>, String>;
    /// Download, verify the signature and install.
    fn install(&mut self, update: &AvailableUpdate) -> Result<(), String>;
}

/// What the user is shown after a check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdatePrompt {
    /// An information dialog.
    Info { title: String, body: String },
    /// Ask whether to install, with Install and Later buttons.
    Offer { title: String, body: String },
}

pub fn up_to_date(current_version: &str) -> UpdatePrompt {
    UpdatePrompt::Info {
        title: "rtbit is up to date".into(),
        body: format!("rtbit {current_version} is the latest version."),
    }
}

pub fn offer(update: &AvailableUpdate) -> UpdatePrompt {
    UpdatePrompt::Offer {
        title: "Update available".into(),
        body: format!(
            "rtbit {} is available (you have {}). Download and install it now?",
            update.version, update.current_version
        ),
    }
}

pub fn failed(error: &str) -> UpdatePrompt {
    UpdatePrompt::Info {
        title: "Update check failed".into(),
        body: error.to_owned(),
    }
}

/// How a check ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    UpToDate,
    Declined,
    /// Installed; the app restarts.
    Installed,
    Failed(String),
}

/// One manual check. `show` displays a prompt and returns whether an offer
/// was accepted (its return is ignored for information dialogs).
pub fn run_check(
    feed: &mut dyn UpdateFeed,
    current_version: &str,
    show: &mut dyn FnMut(UpdatePrompt) -> bool,
) -> UpdateOutcome {
    let fail = |show: &mut dyn FnMut(UpdatePrompt) -> bool, e: String| {
        show(failed(&e));
        UpdateOutcome::Failed(e)
    };
    let update = match feed.check() {
        Ok(Some(update)) => update,
        Ok(None) => {
            show(up_to_date(current_version));
            return UpdateOutcome::UpToDate;
        }
        Err(e) => return fail(show, e),
    };
    if !show(offer(&update)) {
        return UpdateOutcome::Declined;
    }
    match feed.install(&update) {
        Ok(()) => UpdateOutcome::Installed,
        Err(e) => fail(show, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canned release feed. No network.
    struct FakeFeed {
        check: Result<Option<AvailableUpdate>, String>,
        install: Result<(), String>,
        installed: Vec<String>,
    }

    impl FakeFeed {
        fn new(check: Result<Option<AvailableUpdate>, String>) -> Self {
            Self {
                check,
                install: Ok(()),
                installed: Vec::new(),
            }
        }
    }

    impl UpdateFeed for FakeFeed {
        fn check(&mut self) -> Result<Option<AvailableUpdate>, String> {
            self.check.clone()
        }
        fn install(&mut self, update: &AvailableUpdate) -> Result<(), String> {
            self.installed.push(update.version.clone());
            self.install.clone()
        }
    }

    fn release() -> AvailableUpdate {
        AvailableUpdate {
            version: "0.3.0".into(),
            current_version: "0.2.0".into(),
        }
    }

    fn run(feed: &mut FakeFeed, accept: bool) -> (UpdateOutcome, Vec<UpdatePrompt>) {
        let mut shown = Vec::new();
        let outcome = run_check(feed, "0.2.0", &mut |prompt| {
            shown.push(prompt);
            accept
        });
        (outcome, shown)
    }

    #[test]
    fn no_release_reports_up_to_date() {
        let mut feed = FakeFeed::new(Ok(None));
        let (outcome, shown) = run(&mut feed, true);
        assert_eq!(outcome, UpdateOutcome::UpToDate);
        assert_eq!(shown, [up_to_date("0.2.0")]);
        assert!(feed.installed.is_empty());
    }

    #[test]
    fn a_release_is_offered_and_installed_when_accepted() {
        let mut feed = FakeFeed::new(Ok(Some(release())));
        let (outcome, shown) = run(&mut feed, true);
        assert_eq!(outcome, UpdateOutcome::Installed);
        assert_eq!(
            shown,
            [UpdatePrompt::Offer {
                title: "Update available".into(),
                body: "rtbit 0.3.0 is available (you have 0.2.0). Download and install it now?"
                    .into(),
            }]
        );
        assert_eq!(feed.installed, ["0.3.0"]);
    }

    #[test]
    fn declining_installs_nothing() {
        let mut feed = FakeFeed::new(Ok(Some(release())));
        let (outcome, shown) = run(&mut feed, false);
        assert_eq!(outcome, UpdateOutcome::Declined);
        assert_eq!(shown.len(), 1);
        assert!(feed.installed.is_empty());
    }

    #[test]
    fn an_unreachable_feed_reports_the_failure() {
        let mut feed = FakeFeed::new(Err("error sending request".into()));
        let (outcome, shown) = run(&mut feed, true);
        assert_eq!(
            outcome,
            UpdateOutcome::Failed("error sending request".into())
        );
        assert_eq!(shown, [failed("error sending request")]);
        assert!(feed.installed.is_empty());
    }

    #[test]
    fn a_bad_signature_fails_closed_after_the_offer() {
        let mut feed = FakeFeed::new(Ok(Some(release())));
        feed.install = Err("signature verification failed".into());
        let (outcome, shown) = run(&mut feed, true);
        assert_eq!(
            outcome,
            UpdateOutcome::Failed("signature verification failed".into())
        );
        assert_eq!(shown[1], failed("signature verification failed"));
    }
}
