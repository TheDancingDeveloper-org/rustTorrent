use anyhow::Context;
use std::path::Path;
use tracing::{info, warn};

fn migrate_directory(old: &Path, new: &Path) {
    match (old.exists(), new.exists()) {
        (true, true) => {
            warn!(
                ?old,
                ?new,
                "can't migrate configuration as both directories exist, not sure what to do"
            )
        }
        (true, false) => {
            info!(
                ?old,
                ?new,
                "migrating configuration directories as rtbit was upgraded"
            );
            if let Err(e) = std::fs::rename(old, new) {
                warn!(?old, ?new, "error migrating: {e:#}");
            }
        }
        // In these cases, nothing to migrate, so do nothing.
        (false, true) => {}
        (false, false) => {}
    }
}

pub fn get_configuration_directory_linux(
    application: &str,
) -> anyhow::Result<directories::ProjectDirs> {
    // https://github.com/ikatson/rtbit/issues/252
    // On Linux, "com.rtbit" isn't used resulting in weird folder names like "session".
    // The code below migrates from old configuration to new.
    let legacy_dir =
        directories::ProjectDirs::from("com", "rtbit", application).with_context(|| {
            format!("cannot determine project directory for com.rtbit.{application}")
        })?;
    let new_dir =
        directories::ProjectDirs::from("com", "rtbit", &format!("com.rtbit.{application}"))
            .with_context(|| {
                format!("cannot determine project directory for com.rtbit.{application}")
            })?;
    for (old, new) in [
        (legacy_dir.cache_dir(), new_dir.cache_dir()),
        (legacy_dir.config_dir(), new_dir.config_dir()),
        (legacy_dir.data_dir(), new_dir.data_dir()),
    ] {
        migrate_directory(old, new);
    }
    Ok(new_dir)
}

#[cfg(test)]
mod tests {
    use super::migrate_directory;

    #[test]
    fn migrates_legacy_directory_when_current_directory_is_absent() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("session");
        let current = temp.path().join("com.rtbit.session");
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(legacy.join("session.json"), "legacy state").unwrap();

        migrate_directory(&legacy, &current);

        assert!(!legacy.exists());
        assert_eq!(
            std::fs::read_to_string(current.join("session.json")).unwrap(),
            "legacy state"
        );
    }
}

pub fn get_configuration_directory(application: &str) -> anyhow::Result<directories::ProjectDirs> {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        get_configuration_directory_linux(application)
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        directories::ProjectDirs::from("com", "rtbit", application).with_context(|| {
            format!("cannot determine project directory for com.rtbit.{application}")
        })
    }
}
