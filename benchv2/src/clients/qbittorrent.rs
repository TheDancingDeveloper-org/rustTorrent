use anyhow::Result;
use std::path::Path;

pub struct QBittorrentClient {
    url: String,
    http: reqwest::Client,
}

impl QBittorrentClient {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .cookie_store(true)
                .build()
                .unwrap(),
        }
    }

    pub async fn healthy(&self) -> bool {
        self.http
            .get(format!("{}/", self.url))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().as_u16() < 500)
            .unwrap_or(false)
    }

    pub async fn authenticate(&self, docker: &bollard::Docker) -> Result<()> {
        // Try no-auth first
        if let Ok(resp) = self
            .http
            .get(format!("{}/api/v2/app/version", self.url))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            if resp.status().is_success() {
                tracing::info!("qBittorrent: no auth required");
                return Ok(());
            }
        }

        // Try default creds
        if self.try_login("admin", "adminadmin").await {
            return Ok(());
        }

        // Parse temp password from container logs
        use bollard::query_parameters::LogsOptions;
        use futures_util::StreamExt;
        let containers = docker
            .list_containers(Some(bollard::query_parameters::ListContainersOptions {
                filters: Some(std::collections::HashMap::from([(
                    "label".to_string(),
                    vec!["com.docker.compose.service=qbittorrent".to_string()],
                )])),
                ..Default::default()
            }))
            .await?;

        for c in containers {
            let id = c.id.as_deref().unwrap_or("");
            let mut log_stream = docker.logs(
                id,
                Some(LogsOptions {
                    stdout: true,
                    stderr: true,
                    tail: "200".to_string(),
                    ..Default::default()
                }),
            );
            let mut logs = String::new();
            while let Some(Ok(chunk)) = log_stream.next().await {
                logs.push_str(&chunk.to_string());
            }
            // Find line with "temporary password ... session: XXXXX"
            for line in logs.lines() {
                if line.contains("temporary password") {
                    tracing::debug!("qbt password line: {line}");
                    // Extract after last colon
                    if let Some((_, after_colon)) = line.rsplit_once(':') {
                        let pw = after_colon.trim();
                        if !pw.is_empty() {
                            tracing::info!("qBittorrent: found temp password ({} chars)", pw.len());
                            if self.try_login("admin", pw).await {
                                return Ok(());
                            }
                        }
                    }
                    // Fallback: last whitespace-delimited word
                    if let Some(pw) = line.split_whitespace().last() {
                        let pw = pw.trim();
                        if self.try_login("admin", pw).await {
                            return Ok(());
                        }
                    }
                }
            }
        }

        anyhow::bail!("Could not authenticate to qBittorrent")
    }

    async fn try_login(&self, user: &str, pass: &str) -> bool {
        let resp = self
            .http
            .post(format!("{}/api/v2/auth/login", self.url))
            .form(&[("username", user), ("password", pass)])
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        match resp {
            Ok(r) => {
                let text = r.text().await.unwrap_or_default();
                if text.to_lowercase().starts_with("ok") {
                    tracing::info!("qBittorrent: logged in as {user}");
                    return true;
                }
                false
            }
            Err(_) => false,
        }
    }

    pub async fn configure_for_bench(&self) -> Result<()> {
        let prefs = serde_json::json!({
            "dht": false, "pex": false, "lsd": false,
            "upnp": false, "natpmp": false,
            "save_path": "/downloads",
            "temp_path_enabled": false,
            "max_connec": 2000,
            "max_connec_per_torrent": 200,
            "dl_limit": 0, "up_limit": 0,
        });
        self.http
            .post(format!("{}/api/v2/app/setPreferences", self.url))
            .form(&[("json", prefs.to_string())])
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;
        tracing::info!("qBittorrent: configured for benchmarking");
        Ok(())
    }

    pub async fn add_torrent(&self, torrent_path: &Path) -> Result<()> {
        let data = tokio::fs::read(torrent_path).await?;
        let fname = torrent_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let part = reqwest::multipart::Part::bytes(data)
            .file_name(fname)
            .mime_str("application/x-bittorrent")?;
        let form = reqwest::multipart::Form::new()
            .part("torrents", part)
            .text("savepath", "/downloads")
            .text("skip_checking", "false");

        self.http
            .post(format!("{}/api/v2/torrents/add", self.url))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await?;
        Ok(())
    }

    pub async fn get_torrents(&self) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .http
            .get(format!("{}/api/v2/torrents/info", self.url))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn delete_all(&self) -> Result<()> {
        let torrents = self.get_torrents().await?;
        if torrents.is_empty() {
            return Ok(());
        }
        let hashes: Vec<&str> = torrents
            .iter()
            .filter_map(|t| t.get("hash").and_then(|v| v.as_str()))
            .collect();
        let hash_str = hashes.join("|");
        self.http
            .post(format!("{}/api/v2/torrents/delete", self.url))
            .form(&[("hashes", &hash_str), ("deleteFiles", &"true".to_string())])
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await?;
        Ok(())
    }

    pub async fn all_finished(&self) -> Result<bool> {
        let torrents = self.get_torrents().await?;
        if torrents.is_empty() {
            return Ok(false);
        }
        Ok(torrents.iter().all(|t| {
            t.get("progress")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                >= 1.0
        }))
    }

    pub async fn aggregate_speed(&self) -> f64 {
        self.get_torrents()
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|t| t.get("dlspeed").and_then(|v| v.as_f64()))
            .sum()
    }

    pub async fn progress_fraction(&self) -> f64 {
        let torrents = self.get_torrents().await.unwrap_or_default();
        if torrents.is_empty() {
            return 0.0;
        }
        let sum: f64 = torrents
            .iter()
            .filter_map(|t| t.get("progress").and_then(|v| v.as_f64()))
            .sum();
        sum / torrents.len() as f64
    }
}
