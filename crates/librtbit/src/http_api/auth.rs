use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tracing::warn;

const ACCESS_TOKEN_TTL: Duration = Duration::from_secs(15 * 60); // 15 minutes
const REFRESH_TOKEN_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60); // 30 days

struct TokenEntry {
    expires_at: Instant,
}

#[derive(Default)]
pub struct TokenStore {
    access_tokens: RwLock<HashMap<String, TokenEntry>>,
    refresh_tokens: RwLock<HashMap<String, TokenEntry>>,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Deserialize)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

fn generate_token() -> String {
    let bytes: [u8; 32] = rand::random();
    hex::encode(bytes)
}

impl TokenStore {
    pub fn new() -> Self {
        Self {
            access_tokens: RwLock::new(HashMap::new()),
            refresh_tokens: RwLock::new(HashMap::new()),
        }
    }

    pub fn create_tokens(&self) -> TokenResponse {
        let access_token = generate_token();
        let refresh_token = generate_token();
        let now = Instant::now();

        self.access_tokens.write().insert(
            access_token.clone(),
            TokenEntry {
                expires_at: now + ACCESS_TOKEN_TTL,
            },
        );
        self.refresh_tokens.write().insert(
            refresh_token.clone(),
            TokenEntry {
                expires_at: now + REFRESH_TOKEN_TTL,
            },
        );

        TokenResponse {
            access_token,
            refresh_token,
            token_type: "Bearer",
            expires_in: ACCESS_TOKEN_TTL.as_secs(),
        }
    }

    pub fn validate_access_token(&self, token: &str) -> bool {
        let tokens = self.access_tokens.read();
        tokens
            .get(token)
            .is_some_and(|entry| entry.expires_at > Instant::now())
    }

    pub fn refresh(&self, refresh_token: &str) -> Option<TokenResponse> {
        let valid = {
            let tokens = self.refresh_tokens.read();
            tokens
                .get(refresh_token)
                .is_some_and(|entry| entry.expires_at > Instant::now())
        };

        if !valid {
            return None;
        }

        // Revoke the old refresh token (rotation)
        self.refresh_tokens.write().remove(refresh_token);

        Some(self.create_tokens())
    }

    pub fn revoke_refresh_token(&self, refresh_token: &str) {
        self.refresh_tokens.write().remove(refresh_token);
    }

    pub fn cleanup_expired(&self) {
        let now = Instant::now();
        self.access_tokens
            .write()
            .retain(|_, entry| entry.expires_at > now);
        self.refresh_tokens
            .write()
            .retain(|_, entry| entry.expires_at > now);
    }
}

// --- Login rate limiting ---

const LOGIN_FAILURES_BEFORE_LOCKOUT: u32 = 5;
const LOGIN_INITIAL_LOCKOUT: Duration = Duration::from_secs(30);
const LOGIN_MAX_LOCKOUT: Duration = Duration::from_secs(60 * 60);
/// Failures older than this no longer count towards a lockout. It must be longer than
/// the longest lockout, or a client that waits out a lockout would start counting afresh.
const LOGIN_FAILURE_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);
const LOGIN_TRACKED_CLIENTS_MAX: usize = 10_000;

#[derive(Clone, Copy)]
struct ClientLoginState {
    failures: u32,
    last_failure: Instant,
    locked_until: Option<Instant>,
}

/// Per-client lockout for the login endpoints. After a run of failed logins a client is
/// refused for an exponentially growing period; a successful login clears its record.
#[derive(Default)]
pub struct LoginLimiter {
    clients: Mutex<HashMap<IpAddr, ClientLoginState>>,
}

impl LoginLimiter {
    /// Returns how long `client` must wait before it may try to log in again, if locked out.
    pub fn check(&self, client: IpAddr, now: Instant) -> Option<Duration> {
        let clients = self.clients.lock();
        clients
            .get(&client)
            .and_then(|state| state.locked_until)
            .and_then(|until| until.checked_duration_since(now))
            .filter(|remaining| !remaining.is_zero())
    }

    pub fn record_failure(&self, client: IpAddr, now: Instant) {
        let mut clients = self.clients.lock();
        if clients.len() >= LOGIN_TRACKED_CLIENTS_MAX && !clients.contains_key(&client) {
            clients.retain(|_, state| {
                state.locked_until.is_some_and(|until| until > now)
                    || now.duration_since(state.last_failure) < LOGIN_FAILURE_WINDOW
            });
        }

        let state = clients.entry(client).or_insert(ClientLoginState {
            failures: 0,
            last_failure: now,
            locked_until: None,
        });
        if now.duration_since(state.last_failure) >= LOGIN_FAILURE_WINDOW {
            state.failures = 0;
        }
        state.failures = state.failures.saturating_add(1);
        state.last_failure = now;

        if state.failures >= LOGIN_FAILURES_BEFORE_LOCKOUT {
            let doublings = state.failures - LOGIN_FAILURES_BEFORE_LOCKOUT;
            let factor = 1u32.checked_shl(doublings).unwrap_or(u32::MAX);
            let lockout = LOGIN_INITIAL_LOCKOUT
                .saturating_mul(factor)
                .min(LOGIN_MAX_LOCKOUT);
            state.locked_until = Some(now + lockout);
        }
    }

    pub fn record_success(&self, client: IpAddr) {
        self.clients.lock().remove(&client);
    }
}

/// The client address that login rate limiting keys on. Requests without a recorded
/// connection address (only in-process callers) all share one bucket.
pub fn client_addr_from_extensions(extensions: &http::Extensions) -> IpAddr {
    extensions
        .get::<ConnectInfo<librqbit_dualstack_sockets::WrappedSocketAddr>>()
        .map(|ConnectInfo(addr)| addr.0.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

/// Extractor form of [`client_addr_from_extensions`].
pub struct ClientAddr(pub IpAddr);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for ClientAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(ClientAddr(client_addr_from_extensions(&parts.extensions)))
    }
}

// --- Credential Store ---

/// Credentials as stored in `credentials.json`.
#[derive(Serialize, Deserialize, Clone)]
struct CredentialFile {
    username: String,
    /// Argon2 PHC string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password_hash: Option<String>,
    /// Plaintext password written by releases before hashing. It is only read so it can be
    /// migrated to a hash on the first successful login, and is never written back.
    #[serde(default, rename = "password", skip_serializing_if = "Option::is_none")]
    legacy_password: Option<String>,
}

pub struct CredentialStore {
    credentials: RwLock<Option<CredentialFile>>,
    file_path: PathBuf,
    /// One-time token that `/auth/setup` requires. Only generated while no credentials exist.
    setup_token: Option<String>,
}

impl CredentialStore {
    /// Loads `credentials.json` from `config_dir`. A missing file means setup is still
    /// required. A file that exists but cannot be read or parsed is an error, so the caller
    /// refuses to start rather than silently running without authentication.
    pub fn load(config_dir: PathBuf) -> anyhow::Result<Self> {
        let file_path = config_dir.join("credentials.json");
        let credentials = match std::fs::read_to_string(&file_path) {
            Ok(contents) => {
                let creds: CredentialFile = serde_json::from_str(&contents).with_context(|| {
                    format!(
                        "{} is corrupt; refusing to start without authentication. \
                         Fix or remove the file, or run `rtbit auth reset-password`",
                        file_path.display()
                    )
                })?;
                anyhow::ensure!(
                    creds.password_hash.is_some() || creds.legacy_password.is_some(),
                    "{} contains no password; refusing to start without authentication. \
                     Fix or remove the file, or run `rtbit auth reset-password`",
                    file_path.display()
                );
                Some(creds)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(e).with_context(|| format!("cannot read {}", file_path.display()));
            }
        };
        let setup_token = credentials.is_none().then(generate_token);
        Ok(Self {
            credentials: RwLock::new(credentials),
            file_path,
            setup_token,
        })
    }

    /// Replaces the stored credentials in `config_dir`, even if the existing file is corrupt.
    /// Backs the `rtbit auth reset-password` command.
    pub fn reset_credentials(
        config_dir: PathBuf,
        username: &str,
        password: &str,
    ) -> io::Result<()> {
        let store = Self {
            credentials: RwLock::new(None),
            file_path: config_dir.join("credentials.json"),
            setup_token: None,
        };
        store.set_credentials(username, password)
    }

    pub fn has_credentials(&self) -> bool {
        self.credentials.read().is_some()
    }

    pub fn username(&self) -> Option<String> {
        self.credentials
            .read()
            .as_ref()
            .map(|creds| creds.username.clone())
    }

    /// The one-time setup token, if setup has not happened yet.
    pub fn setup_token(&self) -> Option<&str> {
        self.setup_token.as_deref()
    }

    pub fn setup_token_matches(&self, candidate: &str) -> bool {
        self.setup_token
            .as_deref()
            .is_some_and(|token| super::constant_time_eq(candidate.as_bytes(), token.as_bytes()))
    }

    /// Stores `username` with an argon2 hash of `password`, replacing any existing credentials.
    pub fn set_credentials(&self, username: &str, password: &str) -> io::Result<()> {
        let hash = hash_password(password)?;
        let file = CredentialFile {
            username: username.to_owned(),
            password_hash: Some(hash),
            legacy_password: None,
        };
        let json = serde_json::to_vec_pretty(&file).map_err(io::Error::other)?;
        let mut guard = self.credentials.write();
        write_private_file(&self.file_path, &json)?;
        *guard = Some(file);
        Ok(())
    }

    pub fn validate(&self, username: &str, password: &str) -> bool {
        let Some(creds) = self.credentials.read().clone() else {
            return false;
        };
        let username_ok = super::constant_time_eq(username.as_bytes(), creds.username.as_bytes());

        if let Some(hash) = &creds.password_hash {
            // The password is always checked, so a wrong username takes as long as a right one.
            return verify_password(password, hash) && username_ok;
        }

        let legacy = creds.legacy_password.as_deref().unwrap_or_default();
        let password_ok = super::constant_time_eq(password.as_bytes(), legacy.as_bytes());
        if username_ok && password_ok {
            self.migrate_legacy_password(password);
            return true;
        }
        false
    }

    /// Replaces a legacy plaintext entry with a hash of the password that just matched.
    fn migrate_legacy_password(&self, password: &str) {
        let hash = match hash_password(password) {
            Ok(hash) => hash,
            Err(e) => {
                warn!(
                    "could not hash the stored password; it stays in plaintext until the next login: {e}"
                );
                return;
            }
        };
        let mut guard = self.credentials.write();
        let Some(creds) = guard.as_mut() else {
            return;
        };
        // Something else changed the credentials while we were hashing; leave them alone.
        if creds.password_hash.is_some() || creds.legacy_password.as_deref() != Some(password) {
            return;
        }
        let migrated = CredentialFile {
            username: creds.username.clone(),
            password_hash: Some(hash),
            legacy_password: None,
        };
        let written = serde_json::to_vec_pretty(&migrated)
            .map_err(io::Error::other)
            .and_then(|json| write_private_file(&self.file_path, &json));
        match written {
            Ok(()) => *creds = migrated,
            Err(e) => warn!("could not migrate the stored password to a hash: {e}"),
        }
    }
}

fn hash_password(password: &str) -> io::Result<String> {
    let salt_bytes: [u8; 16] = rand::random();
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| io::Error::other(e.to_string()))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| io::Error::other(e.to_string()))
}

fn verify_password(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc).is_ok_and(|parsed| {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
}

/// Writes `contents` to `path` through a temporary file, so a crash never leaves a
/// half-written credentials file. The file is owner-only on unix.
fn write_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

// --- HTTP Handlers ---

use axum::{
    Json,
    extract::{ConnectInfo, State},
    response::{IntoResponse, Response},
};
use http::StatusCode;

use super::HttpApi;

type ApiState = Arc<HttpApi>;

// --- Auth Status ---

#[derive(Serialize)]
pub struct AuthStatus {
    pub auth_enabled: bool,
    pub setup_required: bool,
}

pub async fn h_auth_status(State(state): State<ApiState>) -> impl IntoResponse {
    let has_env_auth = state.opts.basic_auth.is_some();
    let has_stored_creds = state
        .opts
        .credential_store
        .as_ref()
        .is_some_and(|cs| cs.has_credentials());
    let has_creds = has_env_auth || has_stored_creds;
    Json(AuthStatus {
        auth_enabled: has_creds,
        setup_required: !has_creds,
    })
}

// --- Auth Setup (first-boot) ---

#[derive(Deserialize)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
    /// The one-time setup token printed to the daemon log at startup.
    #[serde(default)]
    pub setup_token: String,
}

pub async fn h_auth_setup(
    State(state): State<ApiState>,
    Json(req): Json<SetupRequest>,
) -> impl IntoResponse {
    let cs = match &state.opts.credential_store {
        Some(cs) => cs,
        None => {
            return (StatusCode::NOT_FOUND, "credential management not available").into_response();
        }
    };

    // Only allow if no credentials exist anywhere
    if cs.has_credentials() || state.opts.basic_auth.is_some() {
        return (StatusCode::FORBIDDEN, "credentials already configured").into_response();
    }

    // Without this check, whoever reaches the port first would become the admin.
    if !cs.setup_token_matches(&req.setup_token) {
        return (
            StatusCode::FORBIDDEN,
            "invalid setup token; copy it from the rtbit log",
        )
            .into_response();
    }

    if req.username.is_empty() || req.password.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "username and password are required",
        )
            .into_response();
    }

    match cs.set_credentials(&req.username, &req.password) {
        Ok(_) => {
            // Create tokens for the new user so they're immediately logged in
            if let Some(ts) = &state.opts.token_store {
                let tokens = ts.create_tokens();
                return (StatusCode::OK, Json(tokens)).into_response();
            }
            StatusCode::OK.into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to save credentials: {e}"),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ip() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))
    }

    #[test]
    fn token_refresh_rotates_and_logout_revokes() {
        let store = TokenStore::new();
        let original = store.create_tokens();
        assert!(store.validate_access_token(&original.access_token));

        let replacement = store
            .refresh(&original.refresh_token)
            .expect("valid refresh token");
        assert!(store.refresh(&original.refresh_token).is_none());
        assert!(store.validate_access_token(&replacement.access_token));

        store.revoke_refresh_token(&replacement.refresh_token);
        assert!(store.refresh(&replacement.refresh_token).is_none());
    }

    #[test]
    fn credentials_are_hashed_persist_validate_and_use_private_permissions() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("credentials.json");
        let store = CredentialStore::load(temp.path().to_path_buf()).expect("load");
        assert!(!store.has_credentials());
        store
            .set_credentials("test-user", "correct horse battery staple")
            .expect("write credentials");
        assert!(store.validate("test-user", "correct horse battery staple"));
        assert!(!store.validate("test-user", "wrong"));
        assert!(!store.validate("wrong", "correct horse battery staple"));

        let on_disk = std::fs::read_to_string(&path).expect("credentials file");
        assert!(!on_disk.contains("correct horse battery staple"));
        assert!(on_disk.contains("$argon2id$"));

        let reloaded = CredentialStore::load(temp.path().to_path_buf()).expect("reload");
        assert!(reloaded.validate("test-user", "correct horse battery staple"));
        assert_eq!(reloaded.username().as_deref(), Some("test-user"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .expect("credential metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn corrupt_credentials_file_refuses_to_load() {
        let temp = tempfile::tempdir().expect("temporary directory");
        std::fs::write(temp.path().join("credentials.json"), "not json")
            .expect("write malformed credentials");
        assert!(CredentialStore::load(temp.path().to_path_buf()).is_err());

        std::fs::write(
            temp.path().join("credentials.json"),
            r#"{"username":"admin"}"#,
        )
        .expect("write passwordless credentials");
        assert!(CredentialStore::load(temp.path().to_path_buf()).is_err());
    }

    #[test]
    fn corrupt_credentials_can_be_reset() {
        let temp = tempfile::tempdir().expect("temporary directory");
        std::fs::write(temp.path().join("credentials.json"), "not json")
            .expect("write malformed credentials");
        CredentialStore::reset_credentials(temp.path().to_path_buf(), "admin", "new-pass")
            .expect("reset");
        let store = CredentialStore::load(temp.path().to_path_buf()).expect("load after reset");
        assert!(store.validate("admin", "new-pass"));
        assert!(store.setup_token().is_none());
    }

    #[test]
    fn plaintext_password_is_migrated_on_first_login() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("credentials.json");
        std::fs::write(&path, r#"{"username":"admin","password":"legacy-pass"}"#)
            .expect("write legacy credentials");

        let store = CredentialStore::load(temp.path().to_path_buf()).expect("load legacy");
        assert!(store.has_credentials());
        assert!(!store.validate("admin", "wrong"));
        assert!(
            std::fs::read_to_string(&path)
                .expect("credentials file")
                .contains("legacy-pass"),
            "a failed login must not rewrite the file"
        );

        assert!(store.validate("admin", "legacy-pass"));
        let migrated = std::fs::read_to_string(&path).expect("credentials file");
        assert!(!migrated.contains("legacy-pass"));
        assert!(!migrated.contains("\"password\""));
        assert!(migrated.contains("$argon2id$"));

        let reloaded = CredentialStore::load(temp.path().to_path_buf()).expect("reload");
        assert!(reloaded.validate("admin", "legacy-pass"));
        assert!(!reloaded.validate("admin", "wrong"));
    }

    #[test]
    fn setup_token_is_required_until_setup_is_done() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let store = CredentialStore::load(temp.path().to_path_buf()).expect("load");
        let token = store.setup_token().expect("token before setup").to_owned();
        assert!(!store.setup_token_matches(""));
        assert!(!store.setup_token_matches("not-the-token"));
        assert!(store.setup_token_matches(&token));

        store.set_credentials("admin", "pass").expect("set");
        let configured = CredentialStore::load(temp.path().to_path_buf()).expect("reload");
        assert!(configured.setup_token().is_none());
        assert!(!configured.setup_token_matches(&token));
    }

    #[test]
    fn login_locks_out_after_repeated_failures_and_clears_on_success() {
        let limiter = LoginLimiter::default();
        let start = Instant::now();
        let ip = test_ip();

        for _ in 0..LOGIN_FAILURES_BEFORE_LOCKOUT - 1 {
            limiter.record_failure(ip, start);
        }
        assert!(limiter.check(ip, start).is_none());

        limiter.record_failure(ip, start);
        let wait = limiter.check(ip, start).expect("locked out");
        assert_eq!(wait, LOGIN_INITIAL_LOCKOUT);
        assert!(limiter.check(ip, start + LOGIN_INITIAL_LOCKOUT).is_none());

        let other = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 8));
        assert!(limiter.check(other, start).is_none());

        limiter.record_success(ip);
        assert!(limiter.check(ip, start).is_none());
    }

    #[test]
    fn lockout_grows_exponentially_up_to_a_cap() {
        let limiter = LoginLimiter::default();
        let start = Instant::now();
        let ip = test_ip();
        let mut now = start;
        let mut last = Duration::ZERO;
        for _ in 0..LOGIN_FAILURES_BEFORE_LOCKOUT {
            limiter.record_failure(ip, now);
        }
        for _ in 0..12 {
            let wait = limiter.check(ip, now).expect("locked out");
            assert!(wait >= last);
            assert!(wait <= LOGIN_MAX_LOCKOUT);
            last = wait;
            now += wait;
            limiter.record_failure(ip, now);
        }
        assert_eq!(last, LOGIN_MAX_LOCKOUT);
    }
}

// --- Change Credentials ---

#[derive(Deserialize)]
pub struct ChangeCredentialsRequest {
    pub current_password: String,
    pub new_username: Option<String>,
    pub new_password: Option<String>,
}

pub async fn h_auth_change_credentials(
    State(state): State<ApiState>,
    Json(req): Json<ChangeCredentialsRequest>,
) -> impl IntoResponse {
    let cs = match &state.opts.credential_store {
        Some(cs) => cs,
        None => {
            return (StatusCode::NOT_FOUND, "credential management not available").into_response();
        }
    };

    // Verify the current password against the stored credentials, or the env var fallback.
    let (current_username, current_ok) = match cs.username() {
        Some(username) => {
            let ok = cs.validate(&username, &req.current_password);
            (username, ok)
        }
        None => match &state.opts.basic_auth {
            Some((u, p)) => (
                u.clone(),
                super::constant_time_eq(req.current_password.as_bytes(), p.as_bytes()),
            ),
            None => {
                return (StatusCode::NOT_FOUND, "no credentials configured").into_response();
            }
        },
    };
    if !current_ok {
        return (StatusCode::UNAUTHORIZED, "current password is incorrect").into_response();
    }

    let new_username = req.new_username.unwrap_or(current_username);
    // The current password was just verified, so it stays valid when no new one is given.
    let new_password = req.new_password.unwrap_or(req.current_password);
    if new_username.is_empty() || new_password.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "username and password must not be empty",
        )
            .into_response();
    }

    match cs.set_credentials(&new_username, &new_password) {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to save credentials: {e}"),
        )
            .into_response(),
    }
}

// --- Login ---

pub async fn h_auth_login(
    State(state): State<ApiState>,
    ClientAddr(client): ClientAddr,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    if let Some(wait) = state.login_limiter.check(client, Instant::now()) {
        return too_many_login_attempts(wait);
    }

    // Check credential store first, then fall back to basic_auth
    let valid = if let Some(cs) = &state.opts.credential_store {
        if cs.has_credentials() {
            cs.validate(&req.username, &req.password)
        } else {
            // Fall back to env var credentials
            match &state.opts.basic_auth {
                Some((u, p)) => {
                    super::constant_time_eq(req.username.as_bytes(), u.as_bytes())
                        && super::constant_time_eq(req.password.as_bytes(), p.as_bytes())
                }
                None => false,
            }
        }
    } else {
        match &state.opts.basic_auth {
            Some((u, p)) => {
                super::constant_time_eq(req.username.as_bytes(), u.as_bytes())
                    && super::constant_time_eq(req.password.as_bytes(), p.as_bytes())
            }
            None => {
                return (StatusCode::NOT_FOUND, "authentication not configured").into_response();
            }
        }
    };

    if !valid {
        state.login_limiter.record_failure(client, Instant::now());
        return (StatusCode::UNAUTHORIZED, "invalid credentials").into_response();
    }
    state.login_limiter.record_success(client);

    let token_store = match &state.opts.token_store {
        Some(ts) => ts,
        None => {
            return (StatusCode::NOT_FOUND, "token auth not configured").into_response();
        }
    };

    token_store.cleanup_expired();
    let tokens = token_store.create_tokens();
    (StatusCode::OK, Json(tokens)).into_response()
}

/// The refusal sent while a client is locked out of logging in.
pub fn too_many_login_attempts(wait: Duration) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(http::header::RETRY_AFTER, wait.as_secs().max(1).to_string())],
        "too many failed login attempts; try again later",
    )
        .into_response()
}

pub async fn h_auth_refresh(
    State(state): State<ApiState>,
    Json(req): Json<RefreshRequest>,
) -> impl IntoResponse {
    let token_store = match &state.opts.token_store {
        Some(ts) => ts,
        None => {
            return (StatusCode::NOT_FOUND, "token auth not configured").into_response();
        }
    };

    match token_store.refresh(&req.refresh_token) {
        Some(tokens) => (StatusCode::OK, Json(tokens)).into_response(),
        None => (StatusCode::UNAUTHORIZED, "invalid or expired refresh token").into_response(),
    }
}

pub async fn h_auth_logout(
    State(state): State<ApiState>,
    Json(req): Json<LogoutRequest>,
) -> impl IntoResponse {
    if let Some(ts) = &state.opts.token_store {
        ts.revoke_refresh_token(&req.refresh_token);
    }
    StatusCode::NO_CONTENT
}
