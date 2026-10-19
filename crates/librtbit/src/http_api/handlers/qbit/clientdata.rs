//! Client data controller (`clientdata/*`): the WebUI key/value store.

use super::*;

// ---------------------------------------------------------------------------
// Client data (WebUI key/value store)
// ---------------------------------------------------------------------------

pub(super) type ClientDataMap = serde_json::Map<String, serde_json::Value>;

/// Opaque key/value pairs a WebUI client stores on the server. Kept in memory
/// and, when `path` is set, saved as one small JSON file on every change.
pub(super) struct ClientDataStore {
    pub(super) path: Option<PathBuf>,
    pub(super) data: tokio::sync::Mutex<ClientDataMap>,
}

impl ClientDataStore {
    pub(super) fn open(path: Option<PathBuf>) -> Self {
        let data = path
            .as_deref()
            .and_then(|path| {
                match std::fs::read(path) {
                Ok(bytes) => serde_json::from_slice(&bytes)
                    .inspect_err(|error| {
                        warn!(?path, %error, "qbit compat: ignoring unreadable client data")
                    })
                    .ok(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    warn!(?path, %error, "qbit compat: cannot read client data");
                    None
                }
            }
            })
            .unwrap_or_default();
        Self {
            path,
            data: tokio::sync::Mutex::new(data),
        }
    }

    /// The requested keys that are set, or every pair when `keys` is empty.
    pub(super) async fn load(&self, keys: &[String]) -> ClientDataMap {
        let data = self.data.lock().await;
        if keys.is_empty() {
            return data.clone();
        }
        keys.iter()
            .filter_map(|key| data.get(key).map(|value| (key.clone(), value.clone())))
            .collect()
    }

    /// Merge `changes` in; a `null` value removes its key.
    pub(super) async fn store(&self, changes: ClientDataMap) -> std::io::Result<()> {
        let mut data = self.data.lock().await;
        for (key, value) in changes {
            if value.is_null() {
                data.remove(&key);
            } else {
                data.insert(key, value);
            }
        }
        let Some(path) = &self.path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(&*data)?;
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, path).await
    }
}

/// Where client data is saved: beside the session's JSON persistence.
pub(super) fn clientdata_path() -> Option<PathBuf> {
    // Tests build many routers; keep them off the user's real data directory.
    if cfg!(test) {
        return None;
    }
    SessionPersistenceConfig::default_json_persistence_folder()
        .inspect_err(|error| debug!(%error, "qbit compat: no data directory for client data"))
        .ok()
        .map(|dir| dir.join("qbit-clientdata.json"))
}

#[derive(Deserialize, Default)]
pub(super) struct ClientDataLoadQuery {
    /// JSON array of key names; absent means every key.
    #[serde(default)]
    pub(super) keys: Option<String>,
}

pub(super) async fn h_clientdata_load(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<ClientDataLoadQuery>,
) -> Response {
    let keys: Vec<String> = match query.keys.as_deref() {
        None | Some("") => Vec::new(),
        Some(raw) => match serde_json::from_str(raw) {
            Ok(keys) => keys,
            Err(_) => {
                return qbit_error(
                    StatusCode::BAD_REQUEST,
                    "keys must be a JSON array of strings",
                );
            }
        },
    };
    axum::Json(state.clientdata.load(&keys).await).into_response()
}

#[derive(Deserialize, Default)]
pub(super) struct ClientDataStoreForm {
    /// JSON object of key/value pairs to set; `null` removes a key.
    #[serde(default)]
    pub(super) data: Option<String>,
}

pub(super) async fn h_clientdata_store(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> Response {
    let form: ClientDataStoreForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let Some(raw) = form.data else {
        return qbit_error(StatusCode::BAD_REQUEST, "data is required");
    };
    let Ok(changes) = serde_json::from_str::<ClientDataMap>(&raw) else {
        return qbit_error(StatusCode::BAD_REQUEST, "data must be a JSON object");
    };
    match state.clientdata.store(changes).await {
        Ok(()) => "".into_response(),
        Err(error) => {
            warn!(%error, "qbit compat: error saving client data");
            qbit_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to save client data",
            )
        }
    }
}
