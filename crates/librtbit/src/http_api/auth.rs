use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;

const ACCESS_TOKEN_TTL: Duration = Duration::from_secs(15 * 60); // 15 minutes
const REFRESH_TOKEN_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60); // 30 days

/// Prefix on API keys, so `Authorization: Bearer` can tell a key from a session token.
pub const API_KEY_PREFIX: &str = "rtb_";

/// What a caller may do. A read-only caller may only use GET, HEAD and OPTIONS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    ReadOnly,
}

impl Role {
    pub fn allows_method(self, method: &http::Method) -> bool {
        match self {
            Role::Admin => true,
            Role::ReadOnly => matches!(
                *method,
                http::Method::GET | http::Method::HEAD | http::Method::OPTIONS
            ),
        }
    }
}

/// The role an authenticated request runs as. The auth middleware puts it in the request
/// extensions, for handlers that must check it.
#[derive(Debug, Clone, Copy)]
pub struct Caller(pub Role);

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Tokens and keys are stored by their SHA-256, so a copy of the file cannot be replayed.
fn sha256_hex(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

/// Longest a login may last, however often its refresh tokens are used. Refresh extends a
/// session only up to this point, so a stolen refresh chain cannot keep a session alive forever.
const SESSION_MAX_LIFETIME: Duration = Duration::from_secs(30 * 24 * 60 * 60); // 30 days

/// Answers the current credential fingerprint for `role`, or None if that account no longer
/// exists. Sessions are valid only while this still matches the fingerprint they were issued under.
pub type CurrentFingerprint<'a> = &'a dyn Fn(Role) -> Option<String>;

/// A session token as persisted. Times are Unix seconds, because an `Instant` means nothing
/// after a restart.
#[derive(Clone, Serialize, Deserialize)]
struct SessionRecord {
    role: Role,
    /// The credential the session was issued under. A password change, or removing the account,
    /// changes it and so ends the session. See `CredentialStore::session_fingerprint`.
    fingerprint: String,
    expires_at: u64,
    /// When the login that started this session ends, however it is refreshed.
    session_end: u64,
}

/// The contents of `sessions.json`. Keys are the SHA-256 of each token.
#[derive(Default, Serialize, Deserialize)]
struct SessionFile {
    #[serde(default)]
    access: HashMap<String, SessionRecord>,
    #[serde(default)]
    refresh: HashMap<String, SessionRecord>,
}

#[derive(Default)]
pub struct TokenStore {
    sessions: RwLock<SessionFile>,
    /// Where sessions are saved. None keeps them in memory only.
    file_path: Option<PathBuf>,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub role: Role,
}

/// Issues a new access and refresh token into `file`, for a session that ends at `session_end`.
fn issue_tokens(
    file: &mut SessionFile,
    role: Role,
    fingerprint: &str,
    now: u64,
    session_end: u64,
) -> TokenResponse {
    let access_token = generate_token();
    let refresh_token = generate_token();
    let record = |ttl: Duration| SessionRecord {
        role,
        fingerprint: fingerprint.to_owned(),
        expires_at: (now + ttl.as_secs()).min(session_end),
        session_end,
    };
    file.access
        .insert(sha256_hex(&access_token), record(ACCESS_TOKEN_TTL));
    file.refresh
        .insert(sha256_hex(&refresh_token), record(REFRESH_TOKEN_TTL));
    TokenResponse {
        access_token,
        refresh_token,
        token_type: "Bearer",
        expires_in: ACCESS_TOKEN_TTL.as_secs(),
        role,
    }
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

/// The fingerprint sessions for `role` are issued and checked under: the stored credentials when
/// there are any, otherwise the environment credentials. None when that account does not exist.
pub fn fingerprint_for(
    store: Option<&CredentialStore>,
    env: Option<&(String, String)>,
    role: Role,
) -> Option<String> {
    match store.filter(|cs| cs.has_credentials()) {
        Some(cs) => cs.session_fingerprint(role),
        None => match (role, env) {
            (Role::Admin, Some((user, pass))) => Some(sha256_hex(&format!("env\n{user}\n{pass}"))),
            _ => None,
        },
    }
}

/// Whether a session issued under `record`'s fingerprint is still backed by that credential.
fn is_current(record: &SessionRecord, current: CurrentFingerprint<'_>) -> bool {
    current(record.role).as_deref() == Some(record.fingerprint.as_str())
}

impl TokenStore {
    /// A store whose sessions last only as long as the process.
    pub fn new() -> Self {
        Self::default()
    }

    /// A store whose sessions are saved to `sessions.json` in `config_dir`, so logins survive a
    /// restart. A file that cannot be read starts empty, which only signs everyone out.
    pub fn load(config_dir: PathBuf) -> Self {
        let file_path = config_dir.join("sessions.json");
        let mut file = match std::fs::read_to_string(&file_path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|e| {
                warn!("ignoring unreadable {}: {e}", file_path.display());
                SessionFile::default()
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => SessionFile::default(),
            Err(e) => {
                warn!("cannot read {}: {e}", file_path.display());
                SessionFile::default()
            }
        };
        let now = now_unix();
        file.access.retain(|_, record| record.expires_at > now);
        file.refresh.retain(|_, record| record.expires_at > now);
        Self {
            sessions: RwLock::new(file),
            file_path: Some(file_path),
        }
    }

    /// Saves the sessions. The caller holds the write lock, so saves never interleave.
    fn persist(&self, file: &SessionFile) {
        let Some(path) = &self.file_path else {
            return;
        };
        let written = serde_json::to_vec(file)
            .map_err(io::Error::other)
            .and_then(|json| write_private_file(path, &json));
        if let Err(e) = written {
            warn!("could not save sessions to {}: {e}", path.display());
        }
    }

    /// Starts a new login session for `role`, issued under `fingerprint`.
    pub fn create_tokens(&self, role: Role, fingerprint: &str) -> TokenResponse {
        self.start_session_at(role, fingerprint, now_unix())
    }

    fn start_session_at(&self, role: Role, fingerprint: &str, now: u64) -> TokenResponse {
        let mut file = self.sessions.write();
        let tokens = issue_tokens(
            &mut file,
            role,
            fingerprint,
            now,
            now + SESSION_MAX_LIFETIME.as_secs(),
        );
        self.persist(&file);
        tokens
    }

    /// The role of `token` if it is a live access token whose credential is still current.
    pub fn validate_access_token(
        &self,
        token: &str,
        current: CurrentFingerprint<'_>,
    ) -> Option<Role> {
        let now = now_unix();
        self.sessions
            .read()
            .access
            .get(&sha256_hex(token))
            .filter(|record| record.expires_at > now && is_current(record, current))
            .map(|record| record.role)
    }

    /// Trades a live refresh token for a new pair with the same role and session. The old
    /// refresh token is revoked, so each one works once. Refresh never extends the session past
    /// its end, and it fails once the credential has changed.
    pub fn refresh(
        &self,
        refresh_token: &str,
        current: CurrentFingerprint<'_>,
    ) -> Option<TokenResponse> {
        self.refresh_at(refresh_token, current, now_unix())
    }

    fn refresh_at(
        &self,
        refresh_token: &str,
        current: CurrentFingerprint<'_>,
        now: u64,
    ) -> Option<TokenResponse> {
        let mut file = self.sessions.write();
        let record = file.refresh.remove(&sha256_hex(refresh_token))?;
        let live =
            record.expires_at > now && now < record.session_end && is_current(&record, current);
        if !live {
            self.persist(&file);
            return None;
        }
        let tokens = issue_tokens(
            &mut file,
            record.role,
            &record.fingerprint,
            now,
            record.session_end,
        );
        self.persist(&file);
        Some(tokens)
    }

    pub fn revoke_refresh_token(&self, refresh_token: &str) {
        let mut file = self.sessions.write();
        if file.refresh.remove(&sha256_hex(refresh_token)).is_some() {
            self.persist(&file);
        }
    }

    pub fn cleanup_expired(&self) {
        let now = now_unix();
        let mut file = self.sessions.write();
        let before = file.access.len() + file.refresh.len();
        file.access.retain(|_, record| record.expires_at > now);
        file.refresh.retain(|_, record| record.expires_at > now);
        if file.access.len() + file.refresh.len() != before {
            self.persist(&file);
        }
    }
}

/// An API key as listed. The key itself is only ever shown once, when it is created.
#[derive(Serialize)]
pub struct ApiKeyInfo {
    pub id: String,
    pub name: String,
    pub role: Role,
    /// Unix seconds.
    pub created_at: u64,
}

/// An API key as stored in `api-keys.json`. Only the SHA-256 of the key is kept.
#[derive(Clone, Serialize, Deserialize)]
struct ApiKeyRecord {
    id: String,
    name: String,
    role: Role,
    key_hash: String,
    created_at: u64,
}

impl From<&ApiKeyRecord> for ApiKeyInfo {
    fn from(record: &ApiKeyRecord) -> Self {
        Self {
            id: record.id.clone(),
            name: record.name.clone(),
            role: record.role,
            created_at: record.created_at,
        }
    }
}

/// Long-lived keys for automation, sent as `Authorization: Bearer rtb_...`.
pub struct ApiKeyStore {
    keys: RwLock<Vec<ApiKeyRecord>>,
    file_path: PathBuf,
}

impl ApiKeyStore {
    /// Loads `api-keys.json` from `config_dir`. A corrupt file is an error, as for the
    /// credentials: the server refuses to start rather than silently dropping keys.
    pub fn load(config_dir: PathBuf) -> anyhow::Result<Self> {
        let file_path = config_dir.join("api-keys.json");
        let keys = match std::fs::read_to_string(&file_path) {
            Ok(contents) => serde_json::from_str(&contents).with_context(|| {
                format!(
                    "{} is corrupt; refusing to start. Fix or remove the file",
                    file_path.display()
                )
            })?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                return Err(e).with_context(|| format!("cannot read {}", file_path.display()));
            }
        };
        Ok(Self {
            keys: RwLock::new(keys),
            file_path,
        })
    }

    fn save(&self, keys: &[ApiKeyRecord]) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(keys).map_err(io::Error::other)?;
        write_private_file(&self.file_path, &json)
    }

    /// Creates a key with `role`. Returns its metadata and the key itself.
    pub fn create(&self, name: &str, role: Role) -> io::Result<(ApiKeyInfo, String)> {
        let key = format!("{API_KEY_PREFIX}{}", generate_token());
        let record = ApiKeyRecord {
            id: hex::encode(rand::random::<[u8; 8]>()),
            name: name.to_owned(),
            role,
            key_hash: sha256_hex(&key),
            created_at: now_unix(),
        };
        let info = ApiKeyInfo::from(&record);
        let mut keys = self.keys.write();
        let mut next = keys.clone();
        next.push(record);
        self.save(&next)?;
        *keys = next;
        Ok((info, key))
    }

    pub fn list(&self) -> Vec<ApiKeyInfo> {
        self.keys.read().iter().map(ApiKeyInfo::from).collect()
    }

    /// Deletes key `id`. Returns false if there is no such key.
    pub fn delete(&self, id: &str) -> io::Result<bool> {
        let mut keys = self.keys.write();
        let Some(index) = keys.iter().position(|record| record.id == id) else {
            return Ok(false);
        };
        let mut next = keys.clone();
        next.remove(index);
        self.save(&next)?;
        *keys = next;
        Ok(true)
    }

    /// The role of `token` if it is one of the stored keys.
    pub fn authenticate(&self, token: &str) -> Option<Role> {
        let hash = sha256_hex(token);
        self.keys
            .read()
            .iter()
            .find(|record| record.key_hash == hash)
            .map(|record| record.role)
    }
}

// --- Login rate limiting ---

const LOGIN_FAILURES_BEFORE_LOCKOUT: u32 = 5;
const LOGIN_INITIAL_LOCKOUT: Duration = Duration::from_secs(30);
const LOGIN_MAX_LOCKOUT: Duration = Duration::from_secs(60 * 60);
/// Failures older than this no longer count towards a lockout. It must be longer than
/// the longest lockout, or a client that waits out a lockout would start counting afresh.
const LOGIN_FAILURE_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);
/// Most clients tracked at once. When full, the least active quarter is evicted, so memory
/// stays bounded however many addresses a client cycles through.
const LOGIN_TRACKED_CLIENTS_MAX: usize = 4096;
/// Most argon2 verifications running at once. Each costs tens of milliseconds of CPU and
/// tens of MiB of memory, so this bounds the work whatever the number of addresses.
const ARGON2_CONCURRENT_VERIFIES: usize = 4;

#[derive(Clone, Copy)]
struct ClientLoginState {
    failures: u32,
    last_failure: Instant,
    locked_until: Option<Instant>,
}

/// Counting limit on concurrent argon2 verifications. It never queues: a request that
/// finds every slot taken is turned away at once, so waiting requests cannot pile up.
struct VerifySlots {
    in_use: AtomicUsize,
    max: usize,
}

/// One argon2 verification in progress. Dropping it frees the slot.
pub struct VerifySlot(Arc<VerifySlots>);

impl Drop for VerifySlot {
    fn drop(&mut self) {
        self.0.in_use.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Per-client lockout for the login endpoints. After a run of failed logins a client is
/// refused for an exponentially growing period; a successful login clears its record.
pub struct LoginLimiter {
    clients: Mutex<HashMap<IpAddr, ClientLoginState>>,
    verifies: Arc<VerifySlots>,
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self {
            clients: Mutex::new(HashMap::new()),
            verifies: Arc::new(VerifySlots {
                in_use: AtomicUsize::new(0),
                max: ARGON2_CONCURRENT_VERIFIES,
            }),
        }
    }
}

/// The address a client's failures are counted against. An IPv6 client can cheaply pick
/// any address in its /64, so the whole /64 shares one bucket. IPv4-mapped addresses are
/// folded to IPv4 first.
fn limiter_key(client: IpAddr) -> IpAddr {
    match client.to_canonical() {
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & (!0u128 << 64))),
        v4 => v4,
    }
}

/// Drops the least active quarter of tracked clients. Clients not currently locked out go
/// first, so an attacker cannot shed a lockout by flooding the table.
fn evict_least_active(clients: &mut HashMap<IpAddr, ClientLoginState>, now: Instant) {
    let mut order: Vec<(bool, Instant, IpAddr)> = clients
        .iter()
        .map(|(ip, state)| {
            let locked = state.locked_until.is_some_and(|until| until > now);
            (locked, state.last_failure, *ip)
        })
        .collect();
    let quarter = (order.len() / 4).max(1);
    order.select_nth_unstable(quarter - 1);
    for (_, _, ip) in order.drain(..quarter) {
        clients.remove(&ip);
    }
}

impl LoginLimiter {
    /// Returns how long `client` must wait before it may try to log in again, if locked out.
    pub fn check(&self, client: IpAddr, now: Instant) -> Option<Duration> {
        let clients = self.clients.lock();
        clients
            .get(&limiter_key(client))
            .and_then(|state| state.locked_until)
            .and_then(|until| until.checked_duration_since(now))
            .filter(|remaining| !remaining.is_zero())
    }

    pub fn record_failure(&self, client: IpAddr, now: Instant) {
        let key = limiter_key(client);
        let mut clients = self.clients.lock();
        if !clients.contains_key(&key) && clients.len() >= LOGIN_TRACKED_CLIENTS_MAX {
            evict_least_active(&mut clients, now);
        }

        let state = clients.entry(key).or_insert(ClientLoginState {
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
        self.clients.lock().remove(&limiter_key(client));
    }

    /// Takes a slot for one argon2 verification, or `None` when all slots are in use.
    pub fn try_verify_slot(&self) -> Option<VerifySlot> {
        let slots = &self.verifies;
        let mut current = slots.in_use.load(Ordering::Acquire);
        loop {
            if current >= slots.max {
                return None;
            }
            match slots.in_use.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(VerifySlot(Arc::clone(slots))),
                Err(actual) => current = actual,
            }
        }
    }
}

/// Outcome of checking a login against the stored credentials.
pub enum Verdict {
    /// The login matched an account, with the role that account has.
    Valid(Role),
    Invalid,
    /// No verification slot was free, so nothing was checked and no failure is recorded.
    Busy,
}

/// Checks a login against the stored credentials. argon2 runs on a blocking thread, and only
/// while a verification slot is free. Callers check the lockout before calling this.
pub async fn verify_credentials(
    limiter: &LoginLimiter,
    store: Arc<CredentialStore>,
    username: String,
    password: String,
) -> Verdict {
    let Some(slot) = limiter.try_verify_slot() else {
        return Verdict::Busy;
    };
    let role = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        store.authenticate(&username, &password)
    })
    .await
    .unwrap_or(None);
    match role {
        Some(role) => Verdict::Valid(role),
        None => Verdict::Invalid,
    }
}

/// The refusal sent when no argon2 verification slot is free.
pub fn busy_response() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(http::header::RETRY_AFTER, "1")],
        "too many concurrent login checks; retry shortly",
    )
        .into_response()
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
    /// The optional read-only account, added with `rtbit auth set-read-only-user`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    read_only: Option<ReadOnlyFile>,
}

#[derive(Serialize, Deserialize, Clone)]
struct ReadOnlyFile {
    username: String,
    /// Argon2 PHC string.
    password_hash: String,
}

pub struct CredentialStore {
    /// The last credentials read from the file. The file is the source of truth; see `current`.
    credentials: RwLock<Option<CredentialFile>>,
    file_path: PathBuf,
    /// One-time token that `/auth/setup` requires. Only generated while no credentials exist.
    setup_token: Option<String>,
}

/// Reads `credentials.json` as it is on disk. None when there is no file. A file that cannot be
/// read or parsed is an error, so callers refuse rather than run without authentication.
fn read_credentials_file(path: &Path) -> anyhow::Result<Option<CredentialFile>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let creds: CredentialFile = serde_json::from_str(&contents).with_context(|| {
                format!(
                    "{} is corrupt; refusing to start without authentication. \
                     Fix or remove the file, or run `rtbit auth reset-password`",
                    path.display()
                )
            })?;
            anyhow::ensure!(
                creds.password_hash.is_some() || creds.legacy_password.is_some(),
                "{} contains no password; refusing to start without authentication. \
                 Fix or remove the file, or run `rtbit auth reset-password`",
                path.display()
            );
            Ok(Some(creds))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// The admin credential as sessions see it: the username and the password hash (or, before
/// migration, the legacy password). Changing either changes it.
fn admin_secret(creds: &CredentialFile) -> String {
    match &creds.password_hash {
        Some(hash) => format!("hash\n{}\n{hash}", creds.username),
        None => format!(
            "legacy\n{}\n{}",
            creds.username,
            creds.legacy_password.as_deref().unwrap_or_default()
        ),
    }
}

/// Rejects a read-only account that would share the admin username.
fn ensure_names_differ(admin: &str, read_only: Option<&ReadOnlyFile>) -> io::Result<()> {
    if read_only.is_some_and(|account| account.username == admin) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the admin username must differ from the read-only username",
        ));
    }
    Ok(())
}

/// An argon2 hash of a throwaway password, verified against when there is no read-only account
/// so that the work done does not reveal whether one exists.
fn dummy_password_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| hash_password("no-such-account").expect("argon2 is available"))
}

/// Compares two usernames by SHA-256 digest. Both digests are 32 bytes, so the time taken does
/// not depend on the usernames' lengths or on where they differ.
fn usernames_match(given: &str, expected: &str) -> bool {
    super::constant_time_eq(
        &Sha256::digest(given.as_bytes()),
        &Sha256::digest(expected.as_bytes()),
    )
}

/// Whether `username` and `password` are the read-only account's. The username is always
/// compared and the password always verified, even when there is no such account, so the work
/// done does not reveal whether the account exists or whether the username matched.
fn read_only_matches(account: Option<&ReadOnlyFile>, username: &str, password: &str) -> bool {
    let user_ok = usernames_match(username, account.map_or("", |a| a.username.as_str()));
    let hash = account.map_or(dummy_password_hash(), |a| a.password_hash.as_str());
    let pass_ok = verify_password(password, hash);
    account.is_some() && user_ok && pass_ok
}

impl CredentialStore {
    /// Loads `credentials.json` from `config_dir`. A missing file means setup is still
    /// required. A file that exists but cannot be read or parsed is an error, so the caller
    /// refuses to start rather than silently running without authentication.
    pub fn load(config_dir: PathBuf) -> anyhow::Result<Self> {
        let file_path = config_dir.join("credentials.json");
        let credentials = read_credentials_file(&file_path)?;
        let setup_token = credentials.is_none().then(generate_token);
        let store = Self {
            credentials: RwLock::new(credentials),
            file_path,
            setup_token,
        };
        match &store.setup_token {
            Some(token) => store.write_setup_token_file(token),
            None => {
                // Setup already happened, so a token file left by an earlier run is stale.
                let _ = std::fs::remove_file(store.setup_token_path());
            }
        }
        Ok(store)
    }

    /// Where the setup token is written, next to credentials.json.
    pub fn setup_token_path(&self) -> PathBuf {
        self.file_path.with_file_name("setup-token")
    }

    /// Writes the setup token to an owner-only file, for headless installs whose log nobody
    /// reads. A failure is only a warning, because the token is also printed to stderr.
    fn write_setup_token_file(&self, token: &str) {
        let path = self.setup_token_path();
        if let Err(e) = write_private_file(&path, token.as_bytes()) {
            warn!("could not write the setup token to {}: {e}", path.display());
        }
    }

    /// Replaces the admin account in `config_dir`, even if the existing file is corrupt. Backs
    /// the `rtbit auth reset-password` command. The read-only account is kept when the existing
    /// file can be read, because it is not the admin's to lose.
    pub fn reset_credentials(
        config_dir: PathBuf,
        username: &str,
        password: &str,
    ) -> io::Result<()> {
        let file_path = config_dir.join("credentials.json");
        let read_only = match read_credentials_file(&file_path) {
            Ok(existing) => existing.and_then(|creds| creds.read_only),
            Err(e) => {
                eprintln!(
                    "warning: the existing credentials cannot be read, so the read-only account \
                     could not be kept: {e:#}"
                );
                None
            }
        };
        ensure_names_differ(username, read_only.as_ref())?;
        let hash = hash_password(password)?;
        let file = CredentialFile {
            username: username.to_owned(),
            password_hash: Some(hash),
            legacy_password: None,
            read_only,
        };
        let json = serde_json::to_vec_pretty(&file).map_err(io::Error::other)?;
        write_private_file(&file_path, &json)?;
        let _ = std::fs::remove_file(file_path.with_file_name("setup-token"));
        Ok(())
    }

    /// The credentials as they are on disk now. Every check goes through here, so a change made
    /// by another process (`rtbit auth ...`) applies at once. If the file cannot be read, the last
    /// good copy is used and the failure is logged.
    fn current(&self) -> Option<CredentialFile> {
        let mut guard = self.credentials.write();
        match read_credentials_file(&self.file_path) {
            Ok(creds) => {
                *guard = creds.clone();
                creds
            }
            Err(e) => {
                warn!("using the last known credentials: {e:#}");
                guard.clone()
            }
        }
    }

    /// Applies `change` to the credentials on disk and saves the result, returning whether anything
    /// was written. It runs under the lock and starts from the file, so two changes cannot interleave
    /// and a change saved by another process is never overwritten. If the file cannot be read,
    /// nothing is written.
    fn update(
        &self,
        change: impl FnOnce(Option<&CredentialFile>) -> io::Result<Option<CredentialFile>>,
    ) -> io::Result<bool> {
        let mut guard = self.credentials.write();
        let on_disk = read_credentials_file(&self.file_path).map_err(io::Error::other)?;
        let Some(next) = change(on_disk.as_ref())? else {
            return Ok(false);
        };
        let json = serde_json::to_vec_pretty(&next).map_err(io::Error::other)?;
        write_private_file(&self.file_path, &json)?;
        *guard = Some(next);
        Ok(true)
    }

    pub fn has_credentials(&self) -> bool {
        self.current().is_some()
    }

    pub fn username(&self) -> Option<String> {
        self.current().map(|creds| creds.username)
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

    /// Stores `username` with an argon2 hash of `password`, replacing the admin account. The
    /// read-only account is kept. Every session issued under the old password ends.
    pub fn set_credentials(&self, username: &str, password: &str) -> io::Result<()> {
        let hash = hash_password(password)?;
        self.update(|on_disk| {
            let read_only = on_disk.and_then(|creds| creds.read_only.clone());
            ensure_names_differ(username, read_only.as_ref())?;
            Ok(Some(CredentialFile {
                username: username.to_owned(),
                password_hash: Some(hash),
                legacy_password: None,
                read_only,
            }))
        })?;
        let _ = std::fs::remove_file(self.setup_token_path());
        Ok(())
    }

    /// Adds or replaces the read-only account. Setup must already have made the admin account,
    /// and the two usernames must differ.
    pub fn set_read_only_user(&self, username: &str, password: &str) -> io::Result<()> {
        let hash = hash_password(password)?;
        self.update(|on_disk| {
            let current = on_disk.ok_or_else(|| {
                io::Error::other("finish first-run setup before adding a read-only user")
            })?;
            if current.username == username {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "the read-only username must differ from the admin username",
                ));
            }
            Ok(Some(CredentialFile {
                read_only: Some(ReadOnlyFile {
                    username: username.to_owned(),
                    password_hash: hash,
                }),
                ..current.clone()
            }))
        })?;
        Ok(())
    }

    /// Removes the read-only account, ending its sessions. Returns false if there was none.
    pub fn remove_read_only_user(&self) -> io::Result<bool> {
        self.update(|on_disk| {
            Ok(on_disk
                .filter(|creds| creds.read_only.is_some())
                .map(|creds| CredentialFile {
                    read_only: None,
                    ..creds.clone()
                }))
        })
    }

    /// The username of the read-only account, if there is one.
    pub fn read_only_username(&self) -> Option<String> {
        self.current()
            .and_then(|creds| creds.read_only.map(|account| account.username))
    }

    /// What a session issued for `role` is bound to, as the credentials stand now. It changes with
    /// the admin username or password, so a password change ends every session. It also changes
    /// with the read-only account, so those sessions end when that account is removed or changed.
    /// None when the account does not exist.
    pub fn session_fingerprint(&self, role: Role) -> Option<String> {
        let creds = self.current()?;
        let admin = admin_secret(&creds);
        match role {
            Role::Admin => Some(sha256_hex(&admin)),
            Role::ReadOnly => {
                let account = creds.read_only.as_ref()?;
                Some(sha256_hex(&format!(
                    "{admin}\nread-only\n{}\n{}",
                    account.username, account.password_hash
                )))
            }
        }
    }

    /// The role that `username` and `password` log in as, or None if neither account matches.
    pub fn authenticate(&self, username: &str, password: &str) -> Option<Role> {
        let creds = self.current()?;
        let admin_user_ok = super::constant_time_eq(username.as_bytes(), creds.username.as_bytes());
        // The password is always checked, so a wrong username takes as long as a right one.
        let admin_pass_ok = match &creds.password_hash {
            Some(hash) => verify_password(password, hash),
            None => {
                let legacy = creds.legacy_password.as_deref().unwrap_or_default();
                super::constant_time_eq(password.as_bytes(), legacy.as_bytes())
            }
        };
        if admin_user_ok && admin_pass_ok {
            if creds.password_hash.is_none() {
                self.migrate_legacy_password(password);
            }
            return Some(Role::Admin);
        }

        read_only_matches(creds.read_only.as_ref(), username, password).then_some(Role::ReadOnly)
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
        let result = self.update(|on_disk| {
            let Some(creds) = on_disk else {
                return Ok(None);
            };
            // Something else changed the credentials while we were hashing; leave them alone.
            if creds.password_hash.is_some() || creds.legacy_password.as_deref() != Some(password) {
                return Ok(None);
            }
            Ok(Some(CredentialFile {
                username: creds.username.clone(),
                password_hash: Some(hash),
                legacy_password: None,
                read_only: creds.read_only.clone(),
            }))
        });
        if let Err(e) = result {
            warn!("could not migrate the stored password to a hash: {e}");
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
    Extension, Json,
    extract::{ConnectInfo, Path as UrlPath, State},
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
                let Some(fingerprint) = fingerprint_for(Some(cs), None, Role::Admin) else {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "failed to start a session",
                    )
                        .into_response();
                };
                let tokens = ts.create_tokens(Role::Admin, &fingerprint);
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
        let original = store.create_tokens(Role::Admin, "fp");
        assert_eq!(
            store.validate_access_token(&original.access_token, &always("fp")),
            Some(Role::Admin)
        );

        let replacement = store
            .refresh(&original.refresh_token, &always("fp"))
            .expect("valid refresh token");
        assert!(
            store
                .refresh(&original.refresh_token, &always("fp"))
                .is_none()
        );
        assert_eq!(
            store.validate_access_token(&replacement.access_token, &always("fp")),
            Some(Role::Admin)
        );

        store.revoke_refresh_token(&replacement.refresh_token);
        assert!(
            store
                .refresh(&replacement.refresh_token, &always("fp"))
                .is_none()
        );
    }

    /// A credential check that always answers `fp`, as if the account were unchanged.
    fn always(fp: &'static str) -> impl Fn(Role) -> Option<String> {
        move |_| Some(fp.to_owned())
    }

    /// True when `username` and `password` log in as the admin account.
    fn admin_logs_in(store: &CredentialStore, username: &str, password: &str) -> bool {
        store.authenticate(username, password) == Some(Role::Admin)
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
        assert!(admin_logs_in(
            &store,
            "test-user",
            "correct horse battery staple"
        ));
        assert!(!admin_logs_in(&store, "test-user", "wrong"));
        assert!(!admin_logs_in(
            &store,
            "wrong",
            "correct horse battery staple"
        ));

        let on_disk = std::fs::read_to_string(&path).expect("credentials file");
        assert!(!on_disk.contains("correct horse battery staple"));
        assert!(on_disk.contains("$argon2id$"));

        let reloaded = CredentialStore::load(temp.path().to_path_buf()).expect("reload");
        assert!(admin_logs_in(
            &reloaded,
            "test-user",
            "correct horse battery staple"
        ));
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
        assert!(admin_logs_in(&store, "admin", "new-pass"));
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
        assert!(!admin_logs_in(&store, "admin", "wrong"));
        assert!(
            std::fs::read_to_string(&path)
                .expect("credentials file")
                .contains("legacy-pass"),
            "a failed login must not rewrite the file"
        );

        assert!(admin_logs_in(&store, "admin", "legacy-pass"));
        let migrated = std::fs::read_to_string(&path).expect("credentials file");
        assert!(!migrated.contains("legacy-pass"));
        assert!(!migrated.contains("\"password\""));
        assert!(migrated.contains("$argon2id$"));

        let reloaded = CredentialStore::load(temp.path().to_path_buf()).expect("reload");
        assert!(admin_logs_in(&reloaded, "admin", "legacy-pass"));
        assert!(!admin_logs_in(&reloaded, "admin", "wrong"));
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

    fn lock_out(limiter: &LoginLimiter, ip: IpAddr, now: Instant) {
        for _ in 0..LOGIN_FAILURES_BEFORE_LOCKOUT {
            limiter.record_failure(ip, now);
        }
    }

    #[test]
    fn ipv6_addresses_in_one_slash64_share_a_lockout() {
        let limiter = LoginLimiter::default();
        let now = Instant::now();
        let first: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let rotated: IpAddr = "2001:db8:1:2:ffff:ffff:ffff:ffff".parse().unwrap();
        let other_net: IpAddr = "2001:db8:1:3::1".parse().unwrap();

        lock_out(&limiter, first, now);
        assert!(
            limiter.check(rotated, now).is_some(),
            "same /64 must be locked"
        );
        assert!(
            limiter.check(other_net, now).is_none(),
            "other /64 is separate"
        );
    }

    #[test]
    fn ipv4_mapped_ipv6_is_the_same_client_as_ipv4() {
        let limiter = LoginLimiter::default();
        let now = Instant::now();
        let v4: IpAddr = "192.0.2.7".parse().unwrap();
        let mapped: IpAddr = "::ffff:192.0.2.7".parse().unwrap();

        lock_out(&limiter, mapped, now);
        assert!(limiter.check(v4, now).is_some());
    }

    #[test]
    fn tracked_clients_stay_within_the_bound() {
        let limiter = LoginLimiter::default();
        let now = Instant::now();
        for i in 0..(LOGIN_TRACKED_CLIENTS_MAX as u32 + 500) {
            let ip = IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i));
            limiter.record_failure(ip, now);
        }
        assert!(limiter.clients.lock().len() <= LOGIN_TRACKED_CLIENTS_MAX);
    }

    #[test]
    fn eviction_keeps_locked_clients_when_unlocked_ones_remain() {
        let limiter = LoginLimiter::default();
        let now = Instant::now();
        let locked = test_ip();
        lock_out(&limiter, locked, now);
        for i in 0..(LOGIN_TRACKED_CLIENTS_MAX as u32 + 500) {
            let ip = IpAddr::V4(Ipv4Addr::from(0x0b00_0000 + i));
            limiter.record_failure(ip, now);
        }
        assert!(
            limiter.check(locked, now).is_some(),
            "a lockout must survive a flood"
        );
    }

    #[test]
    fn verify_slots_are_bounded_and_released_on_drop() {
        let limiter = LoginLimiter::default();
        let held: Vec<VerifySlot> = (0..ARGON2_CONCURRENT_VERIFIES)
            .map(|_| limiter.try_verify_slot().expect("slot free"))
            .collect();
        assert!(
            limiter.try_verify_slot().is_none(),
            "no slot beyond the limit"
        );
        drop(held);
        assert!(
            limiter.try_verify_slot().is_some(),
            "dropping frees the slots"
        );
    }

    #[test]
    fn setup_token_file_is_owner_only_and_removed_by_setup() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let store = CredentialStore::load(temp.path().to_path_buf()).expect("load");
        let token = store.setup_token().expect("token").to_owned();
        let token_file = temp.path().join("setup-token");
        assert_eq!(
            std::fs::read_to_string(&token_file).expect("token file"),
            token
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&token_file)
                .expect("token metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        store.set_credentials("admin", "pass").expect("set");
        assert!(!token_file.exists(), "setup must spend the token file");
    }

    #[test]
    fn sessions_survive_a_restart_but_are_stored_hashed() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let store = TokenStore::load(temp.path().to_path_buf());
        let tokens = store.create_tokens(Role::ReadOnly, "fp");

        let on_disk =
            std::fs::read_to_string(temp.path().join("sessions.json")).expect("sessions file");
        assert!(!on_disk.contains(&tokens.access_token));
        assert!(!on_disk.contains(&tokens.refresh_token));

        let reloaded = TokenStore::load(temp.path().to_path_buf());
        assert_eq!(
            reloaded.validate_access_token(&tokens.access_token, &always("fp")),
            Some(Role::ReadOnly)
        );
        let refreshed = reloaded
            .refresh(&tokens.refresh_token, &always("fp"))
            .expect("refresh after restart");
        assert_eq!(refreshed.role, Role::ReadOnly);
        assert!(
            reloaded
                .refresh(&tokens.refresh_token, &always("fp"))
                .is_none()
        );
    }

    #[test]
    fn expired_sessions_are_not_loaded() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let expired = SessionRecord {
            role: Role::Admin,
            fingerprint: "fp".to_owned(),
            expires_at: 1,
            session_end: 1,
        };
        let mut file = SessionFile::default();
        file.access.insert(sha256_hex("old-token"), expired.clone());
        file.refresh.insert(sha256_hex("old-refresh"), expired);
        std::fs::write(
            temp.path().join("sessions.json"),
            serde_json::to_vec(&file).expect("json"),
        )
        .expect("write sessions");

        let store = TokenStore::load(temp.path().to_path_buf());
        assert_eq!(
            store.validate_access_token("old-token", &always("fp")),
            None
        );
        assert!(store.refresh("old-refresh", &always("fp")).is_none());
    }

    #[test]
    fn unreadable_sessions_file_starts_signed_out() {
        let temp = tempfile::tempdir().expect("temporary directory");
        std::fs::write(temp.path().join("sessions.json"), "not json").expect("write");
        let store = TokenStore::load(temp.path().to_path_buf());
        let tokens = store.create_tokens(Role::Admin, "fp");
        assert_eq!(
            store.validate_access_token(&tokens.access_token, &always("fp")),
            Some(Role::Admin)
        );
    }

    #[test]
    fn read_only_role_may_only_read() {
        let get = http::Method::GET;
        let post = http::Method::POST;
        assert!(Role::ReadOnly.allows_method(&get));
        assert!(Role::ReadOnly.allows_method(&http::Method::HEAD));
        assert!(Role::ReadOnly.allows_method(&http::Method::OPTIONS));
        assert!(!Role::ReadOnly.allows_method(&post));
        assert!(!Role::ReadOnly.allows_method(&http::Method::DELETE));
        assert!(Role::Admin.allows_method(&post));
        assert!(Role::Admin.allows_method(&http::Method::DELETE));
    }

    #[test]
    fn api_keys_authenticate_persist_and_delete() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let keys = ApiKeyStore::load(temp.path().to_path_buf()).expect("load");
        let (info, key) = keys
            .create("backup script", Role::ReadOnly)
            .expect("create");
        assert!(key.starts_with(API_KEY_PREFIX));
        assert_eq!(keys.authenticate(&key), Some(Role::ReadOnly));
        assert_eq!(keys.authenticate(&format!("{key}x")), None);
        assert_eq!(keys.list().len(), 1);

        let on_disk = std::fs::read_to_string(temp.path().join("api-keys.json")).expect("file");
        assert!(!on_disk.contains(&key), "the key itself must not be stored");

        let reloaded = ApiKeyStore::load(temp.path().to_path_buf()).expect("reload");
        assert_eq!(reloaded.authenticate(&key), Some(Role::ReadOnly));

        assert!(reloaded.delete(&info.id).expect("delete"));
        assert_eq!(reloaded.authenticate(&key), None);
        assert!(!reloaded.delete(&info.id).expect("second delete"));
    }

    #[test]
    fn corrupt_api_key_file_refuses_to_load() {
        let temp = tempfile::tempdir().expect("temporary directory");
        std::fs::write(temp.path().join("api-keys.json"), "not json").expect("write");
        assert!(ApiKeyStore::load(temp.path().to_path_buf()).is_err());
    }

    #[test]
    fn read_only_account_logs_in_as_read_only_and_survives_admin_changes() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let store = CredentialStore::load(temp.path().to_path_buf()).expect("load");
        store.set_credentials("admin", "admin-pass").expect("admin");
        store
            .set_read_only_user("viewer", "view-pass")
            .expect("viewer");

        assert_eq!(
            store.authenticate("viewer", "view-pass"),
            Some(Role::ReadOnly)
        );
        assert_eq!(store.authenticate("viewer", "admin-pass"), None);
        assert_eq!(store.authenticate("admin", "view-pass"), None);
        assert_eq!(store.authenticate("admin", "admin-pass"), Some(Role::Admin));

        store
            .set_credentials("admin", "new-admin-pass")
            .expect("rotate");
        assert_eq!(store.read_only_username().as_deref(), Some("viewer"));
        let reloaded = CredentialStore::load(temp.path().to_path_buf()).expect("reload");
        assert_eq!(
            reloaded.authenticate("viewer", "view-pass"),
            Some(Role::ReadOnly)
        );

        assert!(
            store.set_read_only_user("admin", "x").is_err(),
            "the read-only username must differ from the admin one"
        );
        assert!(store.set_credentials("viewer", "x").is_err());

        assert!(store.remove_read_only_user().expect("remove"));
        assert_eq!(store.authenticate("viewer", "view-pass"), None);
        assert!(!store.remove_read_only_user().expect("second remove"));
    }

    #[test]
    fn read_only_account_needs_first_run_setup() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let store = CredentialStore::load(temp.path().to_path_buf()).expect("load");
        assert!(store.set_read_only_user("viewer", "view-pass").is_err());
    }

    #[test]
    fn password_change_ends_every_session() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let store = CredentialStore::load(temp.path().to_path_buf()).expect("load");
        store.set_credentials("admin", "old-pass").expect("admin");
        store
            .set_read_only_user("viewer", "view-pass")
            .expect("viewer");
        let tokens = TokenStore::new();
        let current = |role: Role| store.session_fingerprint(role);
        let admin = tokens.create_tokens(Role::Admin, &current(Role::Admin).expect("admin fp"));
        let viewer = tokens.create_tokens(
            Role::ReadOnly,
            &current(Role::ReadOnly).expect("read-only fp"),
        );
        assert_eq!(
            tokens.validate_access_token(&admin.access_token, &current),
            Some(Role::Admin)
        );
        assert_eq!(
            tokens.validate_access_token(&viewer.access_token, &current),
            Some(Role::ReadOnly)
        );

        store.set_credentials("admin", "new-pass").expect("change");
        assert_eq!(
            tokens.validate_access_token(&admin.access_token, &current),
            None
        );
        assert_eq!(
            tokens.validate_access_token(&viewer.access_token, &current),
            None
        );
        assert!(tokens.refresh(&admin.refresh_token, &current).is_none());
        assert!(tokens.refresh(&viewer.refresh_token, &current).is_none());
    }

    #[test]
    fn reset_password_ends_sessions_and_keeps_the_read_only_account() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let dir = temp.path().to_path_buf();
        let store = CredentialStore::load(dir.clone()).expect("load");
        store.set_credentials("admin", "old-pass").expect("admin");
        store
            .set_read_only_user("viewer", "view-pass")
            .expect("viewer");
        let tokens = TokenStore::new();
        let admin = tokens.create_tokens(
            Role::Admin,
            &store.session_fingerprint(Role::Admin).expect("fp"),
        );

        CredentialStore::reset_credentials(dir.clone(), "admin", "reset-pass").expect("reset");
        let reloaded = CredentialStore::load(dir.clone()).expect("reload");
        assert_eq!(
            reloaded.authenticate("admin", "reset-pass"),
            Some(Role::Admin)
        );
        assert_eq!(
            reloaded.authenticate("viewer", "view-pass"),
            Some(Role::ReadOnly),
            "reset-password must keep the read-only account"
        );
        assert_eq!(
            tokens.validate_access_token(&admin.access_token, &|role| reloaded
                .session_fingerprint(role)),
            None,
            "reset-password must end the admin's sessions"
        );

        assert!(
            CredentialStore::reset_credentials(dir.clone(), "viewer", "x").is_err(),
            "reset must not give the admin the read-only account's name"
        );
        let unchanged = CredentialStore::load(dir).expect("reload");
        assert_eq!(unchanged.read_only_username().as_deref(), Some("viewer"));
    }

    #[test]
    fn refresh_cannot_extend_a_session_past_its_cap() {
        let store = TokenStore::new();
        let start = 1_000_000;
        let session_end = start + SESSION_MAX_LIFETIME.as_secs();
        let first = store.start_session_at(Role::Admin, "fp", start);

        let second = store
            .refresh_at(&first.refresh_token, &always("fp"), session_end - 60)
            .expect("refresh inside the cap");
        let record = store
            .sessions
            .read()
            .refresh
            .get(&sha256_hex(&second.refresh_token))
            .cloned()
            .expect("new refresh record");
        assert_eq!(
            record.session_end, session_end,
            "the cap is kept, not restarted"
        );
        assert!(record.expires_at <= session_end);

        assert!(
            store
                .refresh_at(&second.refresh_token, &always("fp"), session_end)
                .is_none(),
            "a refresh at the cap is refused"
        );
    }

    #[test]
    fn removing_the_read_only_account_ends_its_sessions_at_once() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let dir = temp.path().to_path_buf();
        let server = CredentialStore::load(dir.clone()).expect("server copy");
        server
            .set_credentials("admin", "admin-pass")
            .expect("admin");
        let cli = CredentialStore::load(dir.clone()).expect("cli copy");
        cli.set_read_only_user("viewer", "view-pass").expect("add");

        // The running server sees the account the CLI added, without a restart.
        assert_eq!(
            server.authenticate("viewer", "view-pass"),
            Some(Role::ReadOnly)
        );
        let tokens = TokenStore::new();
        let viewer = tokens.create_tokens(
            Role::ReadOnly,
            &server.session_fingerprint(Role::ReadOnly).expect("fp"),
        );
        let admin = tokens.create_tokens(
            Role::Admin,
            &server.session_fingerprint(Role::Admin).expect("fp"),
        );

        assert!(cli.remove_read_only_user().expect("remove"));
        let current = |role: Role| server.session_fingerprint(role);
        assert_eq!(
            tokens.validate_access_token(&viewer.access_token, &current),
            None
        );
        assert!(tokens.refresh(&viewer.refresh_token, &current).is_none());
        assert_eq!(
            tokens.validate_access_token(&admin.access_token, &current),
            Some(Role::Admin)
        );
        assert_eq!(server.authenticate("viewer", "view-pass"), None);

        // A stale server copy must not bring the account back when the admin password is saved.
        server
            .set_credentials("admin", "rotated-pass")
            .expect("save");
        let reloaded = CredentialStore::load(dir).expect("reload");
        assert_eq!(reloaded.authenticate("viewer", "view-pass"), None);
        assert_eq!(
            reloaded.authenticate("admin", "rotated-pass"),
            Some(Role::Admin)
        );
    }

    #[test]
    fn an_admin_save_keeps_accounts_added_by_another_process() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let dir = temp.path().to_path_buf();
        let server = CredentialStore::load(dir.clone()).expect("server copy");
        server
            .set_credentials("admin", "admin-pass")
            .expect("admin");
        let cli = CredentialStore::load(dir.clone()).expect("cli copy");
        cli.set_read_only_user("viewer", "view-pass").expect("add");

        server
            .set_credentials("admin", "rotated-pass")
            .expect("save");
        let reloaded = CredentialStore::load(dir).expect("reload");
        assert_eq!(
            reloaded.authenticate("viewer", "view-pass"),
            Some(Role::ReadOnly)
        );
        assert_eq!(
            reloaded.authenticate("admin", "rotated-pass"),
            Some(Role::Admin)
        );
    }

    #[test]
    fn read_only_login_needs_the_exact_username_and_password() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let store = CredentialStore::load(temp.path().to_path_buf()).expect("load");
        store.set_credentials("admin", "admin-pass").expect("admin");

        // No read-only account: nothing gets in through that path, whatever it is sent.
        for (user, pass) in [("viewer", "view-pass"), ("", ""), ("admin", "view-pass")] {
            assert_eq!(store.authenticate(user, pass), None, "{user:?}");
        }

        store
            .set_read_only_user("viewer", "view-pass")
            .expect("viewer");
        for (user, pass) in [
            ("viewer-", "view-pass"),
            ("Viewer", "view-pass"),
            ("viewe", "view-pass"),
            ("viewer", "wrong"),
            ("viewer", ""),
        ] {
            assert_eq!(store.authenticate(user, pass), None, "{user:?} / {pass:?}");
        }
        assert_eq!(
            store.authenticate("viewer", "view-pass"),
            Some(Role::ReadOnly)
        );
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
    ClientAddr(client): ClientAddr,
    Json(req): Json<ChangeCredentialsRequest>,
) -> impl IntoResponse {
    if let Some(wait) = state.login_limiter.check(client, Instant::now()) {
        return too_many_login_attempts(wait);
    }

    let cs = match &state.opts.credential_store {
        Some(cs) => cs,
        None => {
            return (StatusCode::NOT_FOUND, "credential management not available").into_response();
        }
    };

    // Verify the current password against the stored credentials, or the env var fallback.
    let (current_username, current_ok) = match cs.username() {
        Some(username) => {
            let verdict = verify_credentials(
                &state.login_limiter,
                cs.clone(),
                username.clone(),
                req.current_password.clone(),
            )
            .await;
            match verdict {
                Verdict::Busy => return busy_response(),
                Verdict::Valid(Role::Admin) => (username, true),
                // A read-only account cannot have matched the admin username, so this is only
                // ever a wrong password. It is refused like one.
                Verdict::Valid(Role::ReadOnly) | Verdict::Invalid => (username, false),
            }
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
        state.login_limiter.record_failure(client, Instant::now());
        return (StatusCode::UNAUTHORIZED, "current password is incorrect").into_response();
    }
    state.login_limiter.record_success(client);

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
    // Checked before any password hash is computed, so a locked-out client costs nothing.
    if let Some(wait) = state.login_limiter.check(client, Instant::now()) {
        return too_many_login_attempts(wait);
    }

    // The stored credentials take precedence; the env var is the fallback when none are stored.
    let verdict = match (&state.opts.credential_store, &state.opts.basic_auth) {
        (Some(cs), _) if cs.has_credentials() => {
            verify_credentials(&state.login_limiter, cs.clone(), req.username, req.password).await
        }
        (_, Some((u, p))) => {
            if super::constant_time_eq(req.username.as_bytes(), u.as_bytes())
                && super::constant_time_eq(req.password.as_bytes(), p.as_bytes())
            {
                Verdict::Valid(Role::Admin)
            } else {
                Verdict::Invalid
            }
        }
        (Some(_), None) => Verdict::Invalid,
        (None, None) => {
            return (StatusCode::NOT_FOUND, "authentication not configured").into_response();
        }
    };

    let role = match verdict {
        Verdict::Busy => return busy_response(),
        Verdict::Invalid => {
            state.login_limiter.record_failure(client, Instant::now());
            return (StatusCode::UNAUTHORIZED, "invalid credentials").into_response();
        }
        Verdict::Valid(role) => {
            state.login_limiter.record_success(client);
            role
        }
    };

    let token_store = match &state.opts.token_store {
        Some(ts) => ts,
        None => {
            return (StatusCode::NOT_FOUND, "token auth not configured").into_response();
        }
    };

    token_store.cleanup_expired();
    let Some(fingerprint) = fingerprint_for(
        state.opts.credential_store.as_deref(),
        state.opts.basic_auth.as_ref(),
        role,
    ) else {
        return (StatusCode::UNAUTHORIZED, "the account changed; try again").into_response();
    };
    let tokens = token_store.create_tokens(role, &fingerprint);
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

    let current = |role: Role| {
        fingerprint_for(
            state.opts.credential_store.as_deref(),
            state.opts.basic_auth.as_ref(),
            role,
        )
    };
    match token_store.refresh(&req.refresh_token, &current) {
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

// --- API keys ---

/// Refuses the request unless its caller is an admin. Key management is admin-only, whatever
/// the method.
fn require_admin(caller: Caller) -> Option<Response> {
    (caller.0 != Role::Admin)
        .then(|| (StatusCode::FORBIDDEN, "API keys need an admin account").into_response())
}

#[derive(Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    /// Defaults to `read_only`, the least privilege.
    #[serde(default)]
    pub role: Option<Role>,
}

#[derive(Serialize)]
pub struct CreatedApiKey {
    #[serde(flatten)]
    pub info: ApiKeyInfo,
    /// The key itself. This is the only response that contains it.
    pub key: String,
}

pub async fn h_api_keys_list(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
) -> impl IntoResponse {
    if let Some(refusal) = require_admin(caller) {
        return refusal;
    }
    match &state.opts.api_keys {
        Some(keys) => Json(keys.list()).into_response(),
        None => (StatusCode::NOT_FOUND, "API keys not available").into_response(),
    }
}

pub async fn h_api_key_create(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    Json(req): Json<CreateApiKeyRequest>,
) -> impl IntoResponse {
    if let Some(refusal) = require_admin(caller) {
        return refusal;
    }
    let Some(keys) = &state.opts.api_keys else {
        return (StatusCode::NOT_FOUND, "API keys not available").into_response();
    };
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return (StatusCode::BAD_REQUEST, "name must be 1 to 64 characters").into_response();
    }
    match keys.create(name, req.role.unwrap_or(Role::ReadOnly)) {
        Ok((info, key)) => (StatusCode::CREATED, Json(CreatedApiKey { info, key })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to save API keys: {e}"),
        )
            .into_response(),
    }
}

pub async fn h_api_key_delete(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    UrlPath(id): UrlPath<String>,
) -> impl IntoResponse {
    if let Some(refusal) = require_admin(caller) {
        return refusal;
    }
    let Some(keys) = &state.opts.api_keys else {
        return (StatusCode::NOT_FOUND, "API keys not available").into_response();
    };
    match keys.delete(&id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "no such API key").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to save API keys: {e}"),
        )
            .into_response(),
    }
}
