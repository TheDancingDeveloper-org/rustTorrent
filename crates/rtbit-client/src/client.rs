use std::time::Duration;

use http::StatusCode;
use reqwest::Client as Http;
use serde::de::DeserializeOwned;
use url::Url;

use crate::error::Error;
use crate::types::{LoginBody, RefreshBody, SessionStats, TokenPair, TorrentList};

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

    async fn authed_get<T: DeserializeOwned>(&mut self, path: &str) -> Result<T, Error> {
        let response = self.get_once(path).await?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return decode(response).await;
        }
        // Refresh once, then retry the call once. A second 401 is not refreshed again.
        self.refresh_once().await?;
        decode(self.get_once(path).await?).await
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
