use std::time::Duration;

use http::StatusCode;
use reqwest::Client as Http;
use serde::de::DeserializeOwned;
use url::Url;

use crate::error::Error;
use crate::types::{
    LoginBody, PieceProgress, RefreshBody, SessionStats, TokenPair, Torrent, TorrentList,
    TrackerList,
};
use serde::Serialize;

const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Reserved for a later add-by-file call. Item 1 does not send torrent bytes.
const ADD_FILE_TIMEOUT: Duration = Duration::from_secs(30);

/// A client for the daemon's own HTTP API. It never calls the WebUI API v2 paths.
pub struct Client {
    http: Http,
    base: Url,
    access: Option<String>,
    refresh: Option<String>,
}

impl Client {
    /// `base_url` is the daemon origin, for example `http://127.0.0.1:3030`.
    /// Reads time out after 5 seconds. TLS uses rustls and the platform root store.
    pub fn new(base_url: impl AsRef<str>) -> Result<Self, Error> {
        let base = normalize_base(base_url.as_ref())?;
        let http = Http::builder()
            .timeout(READ_TIMEOUT)
            .connect_timeout(READ_TIMEOUT)
            .use_rustls_tls()
            .build()
            .map_err(|err| Error::Unreachable(err.to_string()))?;
        Ok(Self {
            http,
            base,
            access: None,
            refresh: None,
        })
    }

    /// The timeout a later add-by-file call should use.
    pub const fn add_file_timeout() -> Duration {
        ADD_FILE_TIMEOUT
    }

    /// `POST /auth/login`. Stores the returned token pair.
    pub async fn login(&mut self, username: &str, password: &str) -> Result<TokenPair, Error> {
        let url = self.url("auth/login")?;
        let response = self
            .http
            .post(url)
            .json(&LoginBody { username, password })
            .send()
            .await
            .map_err(|err| Error::Unreachable(err.to_string()))?;
        let tokens: TokenPair = decode(response).await?;
        self.access = Some(tokens.access_token.clone());
        self.refresh = Some(tokens.refresh_token.clone());
        Ok(tokens)
    }

    /// Resume from a refresh token the caller kept outside this process.
    /// One `POST /auth/refresh` replaces it with the pair the daemon returns.
    pub async fn resume(&mut self, refresh_token: &str) -> Result<(), Error> {
        self.refresh = Some(refresh_token.to_owned());
        self.refresh_once().await
    }

    /// The current refresh token, when a login or refresh has stored one.
    #[must_use]
    pub fn refresh_token(&self) -> Option<&str> {
        self.refresh.as_deref()
    }

    /// Forget both tokens. The caller also drops whatever it stored.
    pub fn logout(&mut self) {
        self.access = None;
        self.refresh = None;
    }

    /// `GET /torrents`.
    pub async fn torrents(&mut self) -> Result<TorrentList, Error> {
        self.authed_get("torrents").await
    }

    /// `GET /torrents?with_stats=true`. The list carries per-torrent state and speeds.
    pub async fn torrents_with_stats(&mut self) -> Result<TorrentList, Error> {
        self.authed_get("torrents?with_stats=true").await
    }

    /// `GET /stats`.
    pub async fn stats(&mut self) -> Result<SessionStats, Error> {
        self.authed_get("stats").await
    }

    /// `POST /torrents/{id}/start`.
    pub async fn start_torrent(&mut self, id: u64) -> Result<(), Error> {
        self.authed_post_empty(&format!("torrents/{id}/start"))
            .await
    }

    /// `POST /torrents/{id}/pause`.
    pub async fn pause_torrent(&mut self, id: u64) -> Result<(), Error> {
        self.authed_post_empty(&format!("torrents/{id}/pause"))
            .await
    }

    /// `POST /torrents/{id}/delete`. Removes the torrent and its files.
    pub async fn delete_torrent(&mut self, id: u64) -> Result<(), Error> {
        self.authed_post_empty(&format!("torrents/{id}/delete"))
            .await
    }

    /// `POST /torrents` with a magnet or http URL as the body.
    ///
    /// `save_path` and `category` are optional. An empty string is omitted.
    /// `start_paused` maps to `paused=true`. The body is marked `is_url=true`
    /// so the daemon does not try to read it as a torrent file.
    pub async fn add_magnet(
        &mut self,
        magnet: &str,
        save_path: Option<&str>,
        category: Option<&str>,
        start_paused: bool,
    ) -> Result<(), Error> {
        self.add_torrent(magnet.as_bytes(), true, save_path, category, start_paused)
            .await
    }

    /// `GET /torrents/{id}`. Files, piece count and the download selection.
    pub async fn torrent(&mut self, id: u64) -> Result<Torrent, Error> {
        self.authed_get(&format!("torrents/{id}")).await
    }

    /// `GET /torrents/{id}/trackers`.
    pub async fn trackers(&mut self, id: u64) -> Result<TrackerList, Error> {
        self.authed_get(&format!("torrents/{id}/trackers")).await
    }

    /// `GET /torrents/{id}/peer_stats?state=live`.
    pub async fn peers(&mut self, id: u64) -> Result<crate::types::PeerList, Error> {
        let value: serde_json::Value = self
            .authed_get(&format!("torrents/{id}/peer_stats?state=live"))
            .await?;
        Ok(crate::types::PeerList::from_value(&value))
    }

    /// `GET /torrents/{id}/haves` as the binary bitfield.
    pub async fn haves(&mut self, id: u64) -> Result<PieceProgress, Error> {
        let response = self.get_haves_once(id).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            self.refresh_once().await?;
            decode_haves(self.get_haves_once(id).await?).await
        } else {
            decode_haves(response).await
        }
    }

    /// `POST /torrents/{id}/update_only_files`. `indices` is the new selection.
    pub async fn update_only_files(&mut self, id: u64, indices: &[usize]) -> Result<(), Error> {
        self.authed_post_json(
            &format!("torrents/{id}/update_only_files"),
            &OnlyFilesBody {
                only_files: indices,
            },
        )
        .await
    }

    /// `POST /torrents/{id}/trackers`. Merges the announce URLs.
    pub async fn add_trackers(&mut self, id: u64, trackers: &[String]) -> Result<(), Error> {
        self.authed_post_json(
            &format!("torrents/{id}/trackers"),
            &TrackersBody { trackers },
        )
        .await
    }

    /// `POST /torrents` with the bytes of a `.torrent` file.
    pub async fn add_torrent_file(
        &mut self,
        bytes: &[u8],
        save_path: Option<&str>,
        category: Option<&str>,
        start_paused: bool,
    ) -> Result<(), Error> {
        self.add_torrent(bytes, false, save_path, category, start_paused)
            .await
    }

    async fn add_torrent(
        &mut self,
        body: &[u8],
        is_url: bool,
        save_path: Option<&str>,
        category: Option<&str>,
        start_paused: bool,
    ) -> Result<(), Error> {
        let path = add_path(is_url, save_path, category, start_paused);
        let response = self.post_body_once(&path, body, is_url).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            self.refresh_once().await?;
            decode_empty(self.post_body_once(&path, body, is_url).await?).await
        } else {
            decode_empty(response).await
        }
    }

    async fn authed_get<T: DeserializeOwned>(&mut self, path: &str) -> Result<T, Error> {
        let response = self.get_once(path).await?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return decode(response).await;
        }
        // Refresh once, then retry the call once. A second 401 is not refreshed again.
        self.refresh_once().await?;
        decode(self.get_once(path).await?).await
    }

    async fn authed_post_json(&mut self, path: &str, body: &impl Serialize) -> Result<(), Error> {
        let response = self.post_json_once(path, body).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            self.refresh_once().await?;
            decode_empty(self.post_json_once(path, body).await?).await
        } else {
            decode_empty(response).await
        }
    }

    async fn authed_post_empty(&mut self, path: &str) -> Result<(), Error> {
        let response = self.post_once(path).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            self.refresh_once().await?;
            decode_empty(self.post_once(path).await?).await
        } else {
            decode_empty(response).await
        }
    }

    async fn get_once(&self, path: &str) -> Result<reqwest::Response, Error> {
        let token = self.access.as_deref().ok_or(Error::Unauthorised)?;
        let url = self.url(path)?;
        self.http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|err| Error::Unreachable(err.to_string()))
    }

    async fn get_haves_once(&self, id: u64) -> Result<reqwest::Response, Error> {
        let token = self.access.as_deref().ok_or(Error::Unauthorised)?;
        let url = self.url(&format!("torrents/{id}/haves"))?;
        self.http
            .get(url)
            .bearer_auth(token)
            .header(http::header::ACCEPT, "application/octet-stream")
            .send()
            .await
            .map_err(|err| Error::Unreachable(err.to_string()))
    }

    async fn post_json_once(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<reqwest::Response, Error> {
        let token = self.access.as_deref().ok_or(Error::Unauthorised)?;
        let url = self.url(path)?;
        self.http
            .post(url)
            .bearer_auth(token)
            .json(body)
            .send()
            .await
            .map_err(|err| Error::Unreachable(err.to_string()))
    }

    async fn post_once(&self, path: &str) -> Result<reqwest::Response, Error> {
        let token = self.access.as_deref().ok_or(Error::Unauthorised)?;
        let url = self.url(path)?;
        self.http
            .post(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|err| Error::Unreachable(err.to_string()))
    }

    /// Adds can take longer than a read. A file body gets the add timeout;
    /// a magnet stays on the read timeout.
    async fn post_body_once(
        &self,
        path: &str,
        body: &[u8],
        is_url: bool,
    ) -> Result<reqwest::Response, Error> {
        let token = self.access.as_deref().ok_or(Error::Unauthorised)?;
        let url = self.url(path)?;
        let mut request = self.http.post(url).bearer_auth(token);
        if !is_url {
            request = request.timeout(ADD_FILE_TIMEOUT);
        }
        request
            .body(body.to_vec())
            .send()
            .await
            .map_err(|err| Error::Unreachable(err.to_string()))
    }

    async fn refresh_once(&mut self) -> Result<(), Error> {
        let refresh = self.refresh.clone().ok_or(Error::Unauthorised)?;
        let url = self.url("auth/refresh")?;
        let response = self
            .http
            .post(url)
            .json(&RefreshBody {
                refresh_token: &refresh,
            })
            .send()
            .await
            .map_err(|err| Error::Unreachable(err.to_string()))?;
        let tokens: TokenPair = decode(response).await?;
        self.access = Some(tokens.access_token);
        self.refresh = Some(tokens.refresh_token);
        Ok(())
    }

    fn url(&self, path: &str) -> Result<Url, Error> {
        self.base
            .join(path)
            .map_err(|err| Error::Unreachable(err.to_string()))
    }
}

/// Query string the daemon's add handler already accepts.
fn add_path(
    is_url: bool,
    save_path: Option<&str>,
    category: Option<&str>,
    start_paused: bool,
) -> String {
    let mut path = String::from("torrents?overwrite=true");
    if is_url {
        path.push_str("&is_url=true");
    }
    if start_paused {
        path.push_str("&paused=true");
    }
    if let Some(folder) = save_path.map(str::trim).filter(|s| !s.is_empty()) {
        path.push_str("&output_folder=");
        path.push_str(&urlencoding_minimal(folder));
    }
    if let Some(name) = category.map(str::trim).filter(|s| !s.is_empty()) {
        path.push_str("&category=");
        path.push_str(&urlencoding_minimal(name));
    }
    path
}

/// Encode a query value. The client has no `urlencoding` dependency.
fn urlencoding_minimal(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn normalize_base(raw: &str) -> Result<Url, Error> {
    let mut base =
        Url::parse(raw).map_err(|err| Error::Unreachable(format!("bad base url: {err}")))?;
    let path = base.path();
    if !path.ends_with('/') {
        let next = if path.is_empty() {
            "/".to_owned()
        } else {
            format!("{path}/")
        };
        base.set_path(&next);
    }
    Ok(base)
}

async fn decode<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, Error> {
    let status = response.status();
    if !status.is_success() {
        return Err(Error::from_status(status));
    }
    response
        .json()
        .await
        .map_err(|err| Error::Unreachable(format!("bad json: {err}")))
}

async fn decode_empty(response: reqwest::Response) -> Result<(), Error> {
    let status = response.status();
    if !status.is_success() {
        return Err(Error::from_status(status));
    }
    Ok(())
}

#[derive(Serialize)]
struct OnlyFilesBody<'a> {
    only_files: &'a [usize],
}

#[derive(Serialize)]
struct TrackersBody<'a> {
    trackers: &'a [String],
}

async fn decode_haves(response: reqwest::Response) -> Result<PieceProgress, Error> {
    let status = response.status();
    if !status.is_success() {
        return Err(Error::from_status(status));
    }
    let len = response
        .headers()
        .get("x-bitfield-len")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let bytes = response
        .bytes()
        .await
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    Ok(PieceProgress::from_bytes(&bytes, len))
}
