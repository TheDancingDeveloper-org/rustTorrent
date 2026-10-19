//! Torrent creator controller (`torrentcreator/*`) and its task store.

use super::*;

// ---------------------------------------------------------------------------
// Torrent-creator task store (bridges the native create_torrent)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) enum CreatorStatus {
    Running,
    Finished,
    Failed,
}

/// What an `addTask` asked for, echoed back by `status`.
#[derive(Clone, Default)]
pub(super) struct CreatorParams {
    pub(super) source_path: String,
    pub(super) torrent_file_path: String,
    pub(super) format: String,
    pub(super) piece_size: u32,
    pub(super) private: bool,
    pub(super) start_seeding: bool,
    pub(super) comment: String,
    pub(super) source: String,
    pub(super) trackers: Vec<String>,
    pub(super) url_seeds: Vec<String>,
    pub(super) optimize_alignment: bool,
    pub(super) padded_file_size_limit: i64,
}

#[derive(Clone)]
pub(super) struct CreatorTask {
    pub(super) status: CreatorStatus,
    pub(super) params: CreatorParams,
    pub(super) error: Option<String>,
    pub(super) torrent_bytes: Option<Bytes>,
    pub(super) info_hash: Option<String>,
    pub(super) time_added: chrono::DateTime<chrono::Utc>,
    pub(super) time_finished: Option<chrono::DateTime<chrono::Utc>>,
}

/// In-memory registry of torrent-creation tasks. rtbit creates torrents
/// with the native `create_torrent`; each task runs that on a spawned
/// future and records the result here. Not persisted across restarts.
#[derive(Default)]
pub(super) struct QbitTorrentCreator {
    pub(super) tasks: RwLock<HashMap<String, CreatorTask>>,
}

impl QbitTorrentCreator {
    pub(super) fn new_task(&self, params: CreatorParams) -> String {
        let id: String = (0..16)
            .map(|_| format!("{:02x}", rand::random::<u8>()))
            .collect();
        self.tasks.write().insert(
            id.clone(),
            CreatorTask {
                status: CreatorStatus::Running,
                params,
                error: None,
                torrent_bytes: None,
                info_hash: None,
                time_added: chrono::Utc::now(),
                time_finished: None,
            },
        );
        id
    }

    pub(super) fn finish(&self, id: &str, torrent_bytes: Option<Bytes>, info_hash: String) {
        if let Some(task) = self.tasks.write().get_mut(id) {
            task.status = CreatorStatus::Finished;
            task.torrent_bytes = torrent_bytes;
            task.info_hash = Some(info_hash);
            task.time_finished = Some(chrono::Utc::now());
        }
    }

    pub(super) fn fail(&self, id: &str, error: String) {
        if let Some(task) = self.tasks.write().get_mut(id) {
            task.status = CreatorStatus::Failed;
            task.error = Some(error);
            task.time_finished = Some(chrono::Utc::now());
        }
    }

    pub(super) fn get(&self, id: &str) -> Option<CreatorTask> {
        self.tasks.read().get(id).cloned()
    }

    pub(super) fn all(&self) -> Vec<(String, CreatorTask)> {
        let mut all: Vec<_> = self
            .tasks
            .read()
            .iter()
            .map(|(id, task)| (id.clone(), task.clone()))
            .collect();
        all.sort_by_key(|(_, task)| task.time_added);
        all
    }

    pub(super) fn remove(&self, id: &str) -> bool {
        self.tasks.write().remove(id).is_some()
    }
}

/// WebUI API v2 share-limit encoding: `-1` (and `-2`, "use the global
/// limit", which rtbit leaves disabled) mean no limit.
pub(super) fn ratio_from_qbit(raw: f64) -> Option<f64> {
    (raw >= 0.0).then_some(raw)
}

pub(super) fn ratio_to_qbit(limit: Option<f64>) -> f64 {
    limit.unwrap_or(-1.0)
}

/// Minutes on the wire, `Duration` in the session; negative means no limit.
pub(super) fn minutes_from_qbit(raw: i64) -> Option<std::time::Duration> {
    u64::try_from(raw)
        .ok()
        .map(|m| std::time::Duration::from_secs(m.saturating_mul(60)))
}

pub(super) fn minutes_to_qbit(limit: Option<std::time::Duration>) -> i32 {
    limit.map_or(-1, |d| i32::try_from(d.as_secs() / 60).unwrap_or(i32::MAX))
}

pub(super) fn parse_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

#[derive(Clone, Serialize)]
pub(super) struct QbitCategory {
    pub(super) name: String,
    #[serde(rename = "savePath")]
    pub(super) save_path: String,
}

/// Combined qBit compat state.
// ---------------------------------------------------------------------------
// Torrent creator endpoints
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct AddTaskForm {
    #[serde(default)]
    pub(super) source_path: String,
    #[serde(default)]
    pub(super) torrent_file_path: String,
    #[serde(default)]
    pub(super) format: String,
    #[serde(default)]
    pub(super) trackers: String,
    #[serde(default)]
    pub(super) url_seeds: String,
    #[serde(default)]
    pub(super) piece_size: u32,
    #[serde(default, alias = "torrentName")]
    pub(super) name: String,
    #[serde(default)]
    pub(super) private: String,
    #[serde(default)]
    pub(super) start_seeding: String,
    #[serde(default)]
    pub(super) comment: String,
    #[serde(default)]
    pub(super) source: String,
    #[serde(default)]
    pub(super) optimize_alignment: String,
    #[serde(default)]
    pub(super) padded_file_size_limit: i64,
}

pub(super) fn form_bool(raw: &str) -> bool {
    matches!(raw.trim(), "true" | "1")
}

/// Trackers and web seeds arrive one per line; trackers may also be split
/// with `|`. Blank entries (tier separators) are dropped: rtbit writes a
/// single tracker tier.
pub(super) fn split_url_list(raw: &str, also_pipe: bool) -> Vec<String> {
    raw.split(|c| c == '\n' || c == '\r' || (also_pipe && c == '|'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Validate an `addTask` form into task parameters. The native creator only
/// writes v1 torrents, so v2 and hybrid are refused rather than silently
/// produced as v1.
pub(super) fn creator_params(form: AddTaskForm) -> Result<CreatorParams, &'static str> {
    if form.source_path.trim().is_empty() {
        return Err("sourcePath is required");
    }
    let format = match form.format.trim() {
        "" | "v1" => "v1",
        "v2" | "hybrid" => return Err("only the v1 torrent format is supported"),
        _ => return Err("format must be one of v1, v2, hybrid"),
    };
    if form.piece_size != 0 && (!form.piece_size.is_power_of_two() || form.piece_size < 16_384) {
        return Err("pieceSize must be 0 (auto) or a power of two of at least 16 KiB");
    }
    Ok(CreatorParams {
        source_path: form.source_path,
        torrent_file_path: form.torrent_file_path.trim().to_owned(),
        format: format.to_owned(),
        piece_size: form.piece_size,
        private: form_bool(&form.private),
        start_seeding: form_bool(&form.start_seeding),
        comment: form.comment,
        source: form.source,
        trackers: split_url_list(&form.trackers, true),
        url_seeds: split_url_list(&form.url_seeds, false),
        optimize_alignment: form_bool(&form.optimize_alignment),
        padded_file_size_limit: form.padded_file_size_limit,
    })
}

pub(super) async fn h_torrentcreator_add_task(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> axum::response::Response {
    if !state.api_state.opts.allow_create {
        return (
            StatusCode::FORBIDDEN,
            "creating torrents is not enabled on this server",
        )
            .into_response();
    }
    let form: AddTaskForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let name = (!form.name.trim().is_empty()).then(|| form.name.clone());
    let params = match creator_params(form) {
        Ok(params) => params,
        Err(message) => return qbit_error(StatusCode::BAD_REQUEST, message),
    };
    let task_id = state.creator.new_task(params.clone());

    let state = state.clone();
    let spawned_id = task_id.clone();
    tokio::spawn(async move {
        match run_creator_task(&state, &params, name.as_deref()).await {
            Ok((bytes, info_hash)) => state.creator.finish(&spawned_id, Some(bytes), info_hash),
            Err(error) => state.creator.fail(&spawned_id, format!("{error:#}")),
        }
    });

    axum::Json(serde_json::json!({ "taskID": task_id })).into_response()
}

/// Create the torrent, then do what the task asked with it: save it to
/// `torrentFilePath` and/or add it to the session seeding from the source.
pub(super) async fn run_creator_task(
    state: &QbitState,
    params: &CreatorParams,
    name: Option<&str>,
) -> anyhow::Result<(Bytes, String)> {
    use anyhow::Context as _;
    let spawner = BlockingSpawner::new(1);
    let options = CreateTorrentOptions {
        name,
        trackers: params.trackers.clone(),
        piece_length: (params.piece_size > 0).then_some(params.piece_size),
        private: params.private,
        comment: (!params.comment.is_empty()).then_some(params.comment.as_str()),
        url_seeds: params.url_seeds.clone(),
    };
    let result = create_torrent(Path::new(&params.source_path), options, &spawner).await?;
    let bytes = result.as_bytes()?;
    let info_hash = result.info_hash().as_string();

    if !params.torrent_file_path.is_empty() {
        tokio::fs::write(&params.torrent_file_path, &bytes)
            .await
            .with_context(|| format!("error writing {}", params.torrent_file_path))?;
    }
    if params.start_seeding {
        state
            .api_state
            .api
            .session()
            .add_torrent(
                AddTorrent::from_bytes(bytes.clone()),
                Some(AddTorrentOptions {
                    overwrite: true,
                    output_folder: Some(result.output_folder.to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await
            .context("error adding the created torrent for seeding")?;
    }
    Ok((bytes, info_hash))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreatorTaskStatus {
    #[serde(rename = "taskID")]
    pub(super) task_id: String,
    pub(super) source_path: String,
    pub(super) torrent_file_path: String,
    pub(super) format: String,
    pub(super) piece_size: u32,
    pub(super) private: bool,
    pub(super) comment: String,
    pub(super) source: String,
    pub(super) trackers: Vec<String>,
    pub(super) url_seeds: Vec<String>,
    pub(super) optimize_alignment: bool,
    pub(super) padded_file_size_limit: i64,
    pub(super) status: CreatorStatus,
    pub(super) error_message: String,
    /// Percent, 0–100.
    pub(super) progress: f64,
    pub(super) time_added: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) time_started: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) time_finished: Option<String>,
}

pub(super) fn creator_task_status(id: String, task: CreatorTask) -> CreatorTaskStatus {
    let time_added = task.time_added.to_rfc2822();
    let params = task.params;
    CreatorTaskStatus {
        progress: match task.status {
            CreatorStatus::Finished => 100.0,
            _ => 0.0,
        },
        error_message: task.error.unwrap_or_default(),
        task_id: id,
        source_path: params.source_path,
        torrent_file_path: params.torrent_file_path,
        format: params.format,
        piece_size: params.piece_size,
        private: params.private,
        comment: params.comment,
        source: params.source,
        trackers: params.trackers,
        url_seeds: params.url_seeds,
        optimize_alignment: params.optimize_alignment,
        padded_file_size_limit: params.padded_file_size_limit,
        status: task.status,
        // Tasks start as soon as they are added.
        time_started: Some(time_added.clone()),
        time_added,
        time_finished: task.time_finished.map(|t| t.to_rfc2822()),
    }
}

#[derive(Deserialize, Default)]
pub(super) struct OptionalTaskIdQuery {
    #[serde(default, alias = "taskID")]
    pub(super) task_id: String,
}

pub(super) async fn h_torrentcreator_status(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<OptionalTaskIdQuery>,
) -> axum::response::Response {
    if !query.task_id.is_empty() {
        return match state.creator.get(&query.task_id) {
            Some(task) => {
                axum::Json(vec![creator_task_status(query.task_id, task)]).into_response()
            }
            None => qbit_error(StatusCode::NOT_FOUND, "task not found"),
        };
    }
    let all: Vec<CreatorTaskStatus> = state
        .creator
        .all()
        .into_iter()
        .map(|(id, task)| creator_task_status(id, task))
        .collect();
    axum::Json(all).into_response()
}

#[derive(Deserialize, Default)]
pub(super) struct RequiredTaskIdQuery {
    #[serde(default, alias = "taskID")]
    pub(super) task_id: String,
}

pub(super) async fn h_torrentcreator_torrent_file(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<RequiredTaskIdQuery>,
) -> axum::response::Response {
    if query.task_id.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "taskID is required");
    }
    match state.creator.get(&query.task_id) {
        Some(task) => match (task.status, task.torrent_bytes) {
            (CreatorStatus::Finished, Some(bytes)) => {
                ([("content-type", "application/x-bittorrent")], bytes).into_response()
            }
            (CreatorStatus::Failed, _) => qbit_error(
                StatusCode::CONFLICT,
                format!("task failed: {}", task.error.unwrap_or_default()),
            ),
            _ => qbit_error(StatusCode::CONFLICT, "task has not finished"),
        },
        None => qbit_error(StatusCode::NOT_FOUND, "task not found"),
    }
}

#[derive(Deserialize, Default)]
pub(super) struct DeleteTaskForm {
    #[serde(default, alias = "taskID")]
    pub(super) task_id: String,
}

pub(super) async fn h_torrentcreator_delete_task(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> axum::response::Response {
    let form: DeleteTaskForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if form.task_id.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "taskID is required");
    }
    if state.creator.remove(&form.task_id) {
        (StatusCode::OK, "Ok.").into_response()
    } else {
        qbit_error(StatusCode::NOT_FOUND, "task not found")
    }
}
