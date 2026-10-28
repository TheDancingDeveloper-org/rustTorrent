//! Refresh-token store. The real backend is the OS keychain (Secret Service
//! on Linux). When that is not available the shell asks for a password again.
//! The token is never written to a file and is never logged.

use std::sync::Mutex;

const SERVICE: &str = "dev.rusttorrent.rtbit";
const SCHEMA: &str = "dev.rusttorrent.rtbit";

/// Where a refresh token is kept between launches.
pub trait TokenStore: Clone + Send + Sync + 'static {
    /// Store the token. `None` means the keychain refused or is missing.
    fn store(&self, base_url: &str, refresh_token: &str) -> Option<()>;
    /// The stored token for this daemon, if the keychain has one.
    fn load(&self, base_url: &str) -> Option<Option<String>>;
    /// Drop the stored token. A missing entry is success.
    fn clear(&self, base_url: &str) -> Option<()>;
}

/// The platform keychain. Linux uses the Secret Service over D-Bus. Other
/// platforms, and a machine with no keychain, return `None` so the caller
/// falls back to asking for the password. There is no plaintext fallback.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsKeychain;

impl TokenStore for OsKeychain {
    fn store(&self, base_url: &str, refresh_token: &str) -> Option<()> {
        linux_call(base_url, |collection, attributes| {
            collection
                .create_item(
                    "rustTorrent refresh token",
                    attributes,
                    refresh_token.as_bytes(),
                    true,
                    "text/plain",
                )
                .map(|_| ())
                .ok()
        })
    }

    fn load(&self, base_url: &str) -> Option<Option<String>> {
        linux_call(base_url, |collection, attributes| {
            let found = collection.search_items(attributes).ok()?;
            let Some(item) = found.into_iter().next() else {
                return Some(None);
            };
            let secret = item.get_secret().ok()?;
            String::from_utf8(secret).ok().map(Some)
        })
    }

    fn clear(&self, base_url: &str) -> Option<()> {
        linux_call(base_url, |collection, attributes| {
            let found = collection.search_items(attributes).ok()?;
            for item in found {
                item.delete().ok()?;
            }
            Some(())
        })
    }
}

/// Open the default collection for one call and drop the D-Bus connection
/// afterwards. A missing bus or collection is `None`, which the shell treats
/// as "ask for the password".
#[cfg(target_os = "linux")]
fn linux_call<T>(
    base_url: &str,
    call: impl FnOnce(
        &secret_service::blocking::Collection,
        std::collections::HashMap<&str, &str>,
    ) -> Option<T>,
) -> Option<T> {
    let account = account(base_url);
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .ok()?;
    let collection = service.get_default_collection().ok()?;
    let mut attributes = std::collections::HashMap::new();
    attributes.insert("service", SERVICE);
    attributes.insert("account", account.as_str());
    attributes.insert("xdg:schema", SCHEMA);
    call(&collection, attributes)
}

/// No OS keychain on this platform. The shell asks for the password.
#[cfg(not(target_os = "linux"))]
fn linux_call<T>(
    base_url: &str,
    call: impl FnOnce(&(), std::collections::HashMap<&str, &str>) -> Option<T>,
) -> Option<T> {
    let _ = (base_url, call);
    None
}

fn account(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_owned()
}

/// In-memory store for tests. It never touches the OS keychain.
#[derive(Clone, Default)]
pub struct MemoryStore {
    inner: std::sync::Arc<Mutex<Option<(String, String)>>>,
}

impl TokenStore for MemoryStore {
    fn store(&self, base_url: &str, refresh_token: &str) -> Option<()> {
        *self.inner.lock().unwrap_or_else(|err| err.into_inner()) =
            Some((account(base_url), refresh_token.to_owned()));
        Some(())
    }

    fn load(&self, base_url: &str) -> Option<Option<String>> {
        let guard = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        Some(
            guard
                .as_ref()
                .and_then(|(stored, token)| (stored == &account(base_url)).then(|| token.clone())),
        )
    }

    fn clear(&self, base_url: &str) -> Option<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        if guard
            .as_ref()
            .is_some_and(|(stored, _)| stored == &account(base_url))
        {
            *guard = None;
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::{MemoryStore, TokenStore};

    #[test]
    fn memory_store_round_trips_and_clears() {
        let store = MemoryStore::default();
        assert_eq!(store.load("http://127.0.0.1:3030/").unwrap(), None);
        store.store("http://127.0.0.1:3030/", "refresh-1").unwrap();
        assert_eq!(
            store.load("http://127.0.0.1:3030").unwrap().as_deref(),
            Some("refresh-1")
        );
        // A different origin does not see it.
        assert_eq!(store.load("http://127.0.0.1:9").unwrap(), None);
        store.clear("http://127.0.0.1:3030").unwrap();
        assert_eq!(store.load("http://127.0.0.1:3030").unwrap(), None);
    }

    /// A store that refuses every call, the way a missing keychain does.
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

    #[test]
    fn a_closed_store_does_not_yield_a_token() {
        assert!(Closed.load("http://127.0.0.1:3030").is_none());
    }
}
