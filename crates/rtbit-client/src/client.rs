use std::time::Duration;

use http::StatusCode;
use reqwest::Client as Http;
use serde::de::DeserializeOwned;
use url::Url;

use crate::error::Error;
use crate::types::{
    AuthStatus, LoginBody, PieceProgress, RefreshBody, SessionStats, SetupBody, TokenPair, Torrent,
    TorrentList, TrackerList,
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

    /// `GET /auth/status`. No credentials. Tells the shell whether first-run
    /// setup is still open.
    pub async fn auth_status(&self) -> Result<AuthStatus, Error> {
        let url = self.url("auth/status")?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|err| Error::Unreachable(err.to_string()))?;
        decode(response).await
    }

    /// `POST /auth/setup`. Stores the returned token pair, the same way login does.
    /// The setup token and password are not logged.
    pub async fn setup(
        &mut self,
        username: &str,
        password: &str,
        setup_token: &str,
    ) -> Result<TokenPair, Error> {
        let url = self.url("auth/setup")?;
        let response = self
            .http
            .post(url)
            .json(&SetupBody {
                username,
                password,
                setup_token,
            })
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

    /// Read every route the settings screen shows.
    pub async fn settings(&mut self) -> Result<crate::types::SettingsSnapshot, Error> {
        use crate::types::SettingsSnapshot;
        Ok(SettingsSnapshot {
            downloads: self.authed_get("session/downloads").await?,
            advanced: self.authed_get("session/advanced").await?,
            network: self.authed_get("session/network").await?,
            proxy: self.authed_get("session/proxy").await?,
            ip_filter: self.authed_get("session/ip-filter").await?,
            folders: self.authed_get("session/folders").await?,
            limits: self.authed_get("torrents/limits").await?,
            alt: self.authed_get("speed/alt").await?,
            schedule: self.authed_get("speed/schedule").await?,
            seed_limits: self.authed_get("torrents/seed_limits").await?,
        })
    }

    /// `POST /session/downloads`.
    pub async fn set_downloads(
        &mut self,
        body: &crate::types::DownloadsConfig,
    ) -> Result<(), Error> {
        self.authed_post_json("session/downloads", body).await
    }

    /// `POST /session/advanced`. The daemon refuses 0 and values over its caps.
    pub async fn set_advanced(&mut self, body: &crate::types::AdvancedConfig) -> Result<(), Error> {
        self.authed_post_json("session/advanced", body).await
    }

    /// `POST /session/network`. Saved for the next start; nothing changes live.
    /// The daemon answers 409 when session persistence is off.
    pub async fn set_network(&mut self, body: &crate::types::NetworkSettings) -> Result<(), Error> {
        self.authed_post_json("session/network", body).await
    }

    /// `POST /session/network` with `{"reset": true}`: forget the saved values,
    /// so the next start uses the startup flags.
    pub async fn reset_network(&mut self) -> Result<(), Error> {
        self.set_network(&crate::types::NetworkSettings {
            reset: Some(true),
            ..Default::default()
        })
        .await
    }

    /// `POST /session/proxy`. A password in the URL is stored; the daemon never echoes it.
    pub async fn set_proxy(&mut self, body: &crate::types::ProxyConfig) -> Result<(), Error> {
        self.authed_post_json("session/proxy", body).await
    }

    /// `POST /session/ip-filter`. The daemon caps the text and rejects a bad line.
    pub async fn set_ip_filter(
        &mut self,
        body: &crate::types::IpFilterConfig,
    ) -> Result<(), Error> {
        self.authed_post_json("session/ip-filter", body).await
    }

    /// `POST /session/folders`.
    pub async fn set_folders(&mut self, body: &crate::types::FoldersConfig) -> Result<(), Error> {
        self.authed_post_json("session/folders", body).await
    }

    /// `POST /torrents/limits`.
    pub async fn set_limits(&mut self, body: &crate::types::RateLimits) -> Result<(), Error> {
        self.authed_post_json("torrents/limits", body).await
    }

    /// `POST /speed/alt`.
    pub async fn set_alt_enabled(&mut self, enabled: bool) -> Result<(), Error> {
        #[derive(Serialize)]
        struct Body {
            enabled: bool,
        }
        self.authed_post_json("speed/alt", &Body { enabled }).await
    }

    /// `POST /speed/alt/config`.
    pub async fn set_alt_config(
        &mut self,
        body: &crate::types::AltSpeedConfig,
    ) -> Result<(), Error> {
        self.authed_post_json("speed/alt/config", body).await
    }

    /// `POST /speed/schedule`.
    pub async fn set_schedule(
        &mut self,
        body: &crate::types::AltSpeedSchedule,
    ) -> Result<(), Error> {
        self.authed_post_json("speed/schedule", body).await
    }

    /// `POST /torrents/seed_limits`. `None` turns that limit off.
    pub async fn set_seed_limits(&mut self, body: &crate::types::SeedLimits) -> Result<(), Error> {
        self.authed_post_json("torrents/seed_limits", body).await
    }

    /// `GET /rss/feeds`, `GET /rss/items` and `GET /rss/rules`.
    pub async fn rss(&mut self) -> Result<crate::types::RssSnapshot, Error> {
        Ok(crate::types::RssSnapshot {
            feeds: self.authed_get("rss/feeds").await?,
            items: self.authed_get("rss/items").await?,
            rules: self.rss_rules().await?,
        })
    }

    /// `GET /rss/rules`.
    pub async fn rss_rules(&mut self) -> Result<Vec<crate::types::RssRule>, Error> {
        self.authed_get("rss/rules").await
    }

    /// `POST /rss/rules`. The daemon picks the id. A pattern that does not
    /// compile is a 400 whose message says why.
    pub async fn add_rss_rule(&mut self, body: &crate::types::RssRuleBody) -> Result<(), Error> {
        self.authed_post_json("rss/rules", body).await
    }

    /// `GET /rss/rules/{id}/episodes`. The daemon caps the list.
    pub async fn rss_rule_episodes(
        &mut self,
        id: &str,
    ) -> Result<Vec<crate::types::RssRuleEpisode>, Error> {
        let path = format!("rss/rules/{}/episodes", urlencoding_minimal(id));
        self.authed_get(&path).await
    }

    /// `PUT /rss/rules/{id}`. Replaces the whole rule.
    pub async fn update_rss_rule(
        &mut self,
        id: &str,
        body: &crate::types::RssRuleBody,
    ) -> Result<(), Error> {
        let path = format!("rss/rules/{}", urlencoding_minimal(id));
        let response = self
            .send_json_once(reqwest::Method::PUT, &path, Some(body))
            .await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            self.refresh_once().await?;
            decode_empty(
                self.send_json_once(reqwest::Method::PUT, &path, Some(body))
                    .await?,
            )
            .await
        } else {
            decode_empty(response).await
        }
    }

    /// `DELETE /rss/rules/{id}`.
    pub async fn delete_rss_rule(&mut self, id: &str) -> Result<(), Error> {
        let path = format!("rss/rules/{}", urlencoding_minimal(id));
        let none: Option<&()> = None;
        let response = self
            .send_json_once(reqwest::Method::DELETE, &path, none)
            .await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            self.refresh_once().await?;
            decode_empty(
                self.send_json_once(reqwest::Method::DELETE, &path, none)
                    .await?,
            )
            .await
        } else {
            decode_empty(response).await
        }
    }

    /// `POST /rss/items/{id}/download`. The daemon adds the item's URL as a torrent.
    pub async fn download_rss_item(&mut self, id: &str) -> Result<(), Error> {
        let path = format!("rss/items/{}/download", urlencoding_minimal(id));
        self.authed_post_json(&path, &serde_json::json!({})).await
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

    async fn send_json_once(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<reqwest::Response, Error> {
        let token = self.access.as_deref().ok_or(Error::Unauthorised)?;
        let url = self.url(path)?;
        let mut request = self.http.request(method, url).bearer_auth(token);
        if let Some(body) = body {
            request = request.json(body);
        }
        request
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

/// Encode one path or query segment. `.` and `..` are percent-encoded so a
/// segment cannot be a dot path. Space is `%20` (a `+` would be a literal plus
/// once the segment is in a path). The client has no `urlencoding` dependency.
fn urlencoding_minimal(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'~' => {
                out.push(byte as char);
            }
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
        return Err(error_with_body(status, response).await);
    }
    response
        .json()
        .await
        .map_err(|err| Error::Unreachable(format!("bad json: {err}")))
}

async fn decode_empty(response: reqwest::Response) -> Result<(), Error> {
    let status = response.status();
    if !status.is_success() {
        return Err(error_with_body(status, response).await);
    }
    Ok(())
}

async fn error_with_body(status: http::StatusCode, response: reqwest::Response) -> Error {
    if status != http::StatusCode::BAD_REQUEST {
        return Error::from_status(status);
    }
    let body = response.text().await.unwrap_or_default();
    Error::from_status_body(status, &body)
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
        return Err(error_with_body(status, response).await);
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

#[cfg(test)]
mod encoding {
    use super::urlencoding_minimal;

    #[test]
    fn spaces_are_percent_twenty_and_dots_are_encoded() {
        assert_eq!(urlencoding_minimal("a b"), "a%20b");
        assert_eq!(urlencoding_minimal("."), "%2E");
        assert_eq!(urlencoding_minimal(".."), "%2E%2E");
        assert_eq!(urlencoding_minimal("keep-it_ok~"), "keep-it_ok~");
        assert_eq!(urlencoding_minimal("a/b"), "a%2Fb");
    }
}
