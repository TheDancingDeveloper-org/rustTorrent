use std::path::PathBuf;

use axum::{Json, extract::Query, extract::State, response::IntoResponse};
use http::StatusCode;
use serde::{Deserialize, Serialize};

use super::ApiState;
use crate::{
    ApiError, WithStatusError,
    api::{EmptyJsonResponse, Result},
    limits::LimitsConfig,
};

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/limits",
    request_body(content = LimitsConfig, description = "Rate limits configuration"),
    responses(
        (status = 200, description = "Rate limits updated", body = EmptyJsonResponse)
    )
))]
pub async fn h_update_session_ratelimits(
    State(state): State<ApiState>,
    Json(limits): Json<LimitsConfig>,
) -> Result<impl IntoResponse> {
    // Goes through the alt-speed-aware setter: while alternative limits are
    // active this updates the saved normal limits instead of the live limiter.
    state
        .api
        .session()
        .set_normal_rate_limits(limits.download_bps, limits.upload_bps);
    if let Some(peer_limit) = limits.peer_limit {
        state.api.session().set_peer_limit(peer_limit);
    }
    if let Some(init_limit) = limits.concurrent_init_limit {
        state.api.session().set_concurrent_init_limit(init_limit);
    }
    Ok(Json(EmptyJsonResponse {}))
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/torrents/limits",
    responses(
        (status = 200, description = "Current rate limits", body = LimitsConfig)
    )
))]
pub async fn h_get_session_ratelimits(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    let mut config = state.api.session().ratelimits.get_config();
    config.peer_limit = Some(state.api.session().get_peer_limit());
    config.concurrent_init_limit = Some(state.api.session().get_concurrent_init_limit());
    Ok(Json(config))
}

// --- Folder management ---

#[derive(Serialize, Deserialize)]
pub struct FoldersConfig {
    pub download_folder: String,
    pub completed_folder: Option<String>,
}

pub async fn h_get_folders(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    Ok(Json(FoldersConfig {
        download_folder: state.api.api_output_folder(),
        completed_folder: state.api.api_completed_folder(),
    }))
}

pub async fn h_set_folders(
    State(state): State<ApiState>,
    Json(config): Json<FoldersConfig>,
) -> Result<impl IntoResponse> {
    state
        .api
        .api_set_folders(config.download_folder, config.completed_folder)
        .await?;
    Ok(Json(EmptyJsonResponse {}))
}

// --- Downloads settings ---

/// Session-wide download behaviour shown on the settings Downloads tab.
#[derive(Serialize, Deserialize)]
pub struct DownloadsConfig {
    /// Write incomplete files under a `.part` name; applies to torrents added afterwards.
    pub incomplete_file_suffix: bool,
}

pub async fn h_get_downloads(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    let session = state.api.session();
    Ok(Json(DownloadsConfig {
        incomplete_file_suffix: session.use_part_files(),
    }))
}

pub async fn h_set_downloads(
    State(state): State<ApiState>,
    Json(config): Json<DownloadsConfig>,
) -> Result<impl IntoResponse> {
    let session = state.api.session();
    session.set_use_part_files(config.incomplete_file_suffix);
    session
        .persist_session_settings()
        .await
        .map_err(ApiError::from)?;
    Ok(Json(EmptyJsonResponse {}))
}

// --- Directory browsing ---

#[derive(Deserialize)]
pub struct BrowseQuery {
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Serialize)]
pub struct BrowseResponse {
    pub current: String,
    pub parent: Option<String>,
    pub entries: Vec<DirEntry>,
}

/// The folders the browser may show: the session's download folder and, when set, its
/// completed folder. Folders that do not exist are left out.
fn browse_roots(state: &ApiState) -> Vec<PathBuf> {
    std::iter::once(state.api.api_output_folder())
        .chain(state.api.api_completed_folder())
        .filter_map(|folder| PathBuf::from(folder).canonicalize().ok())
        .collect()
}

/// Resolves a browse request to a canonical directory inside one of `roots`. With no
/// path, the first root is used. Symlinks and `..` are resolved before the check, so
/// neither can step out of a root.
fn resolve_browse_path(requested: Option<PathBuf>, roots: &[PathBuf]) -> Result<PathBuf> {
    let path = match requested {
        Some(path) => path,
        None => roots
            .first()
            .cloned()
            .with_status_error(StatusCode::FORBIDDEN, "no download folder is configured")?,
    };

    let canonical = path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("invalid path: {e}"))?;
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err(ApiError::from((
            StatusCode::FORBIDDEN,
            "path is outside the download folders",
        )));
    }
    Ok(canonical)
}

pub async fn h_browse_directory(
    State(state): State<ApiState>,
    Query(query): Query<BrowseQuery>,
) -> Result<impl IntoResponse> {
    let roots = browse_roots(&state);
    let canonical = resolve_browse_path(query.path.map(PathBuf::from), &roots)?;

    // The parent is only offered while it is still inside a root.
    let parent = canonical
        .parent()
        .filter(|parent| roots.iter().any(|root| parent.starts_with(root)))
        .map(|p| p.to_string_lossy().into_owned());

    let mut entries = Vec::new();
    let read_dir =
        std::fs::read_dir(&canonical).map_err(|e| anyhow::anyhow!("cannot read directory: {e}"))?;

    for entry in read_dir.flatten() {
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip hidden files/dirs
        if name.starts_with('.') {
            continue;
        }
        let entry_path = entry.path();
        entries.push(DirEntry {
            name,
            path: entry_path.to_string_lossy().into_owned(),
            is_dir: file_type.is_dir(),
        });
    }

    // Sort: directories first, then alphabetically
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(Json(BrowseResponse {
        current: canonical.to_string_lossy().into_owned(),
        parent,
        entries,
    }))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn canonical_roots(dirs: &[&Path]) -> Vec<PathBuf> {
        dirs.iter()
            .map(|dir| dir.canonicalize().expect("canonical test root"))
            .collect()
    }

    #[test]
    fn browse_defaults_to_first_root_and_allows_subdirectories() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let downloads = temp.path().join("downloads");
        std::fs::create_dir_all(downloads.join("movies")).expect("create folders");
        let roots = canonical_roots(&[&downloads]);

        assert_eq!(
            resolve_browse_path(None, &roots).expect("default"),
            roots[0]
        );
        let movies = resolve_browse_path(Some(downloads.join("movies")), &roots).expect("subdir");
        assert!(movies.ends_with("movies"));
    }

    #[test]
    fn browse_refuses_paths_outside_the_roots() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let downloads = temp.path().join("downloads");
        // Shares a string prefix with the root but is a different folder.
        let lookalike = temp.path().join("downloads-old");
        let secret = temp.path().join("secret");
        for dir in [&downloads, &lookalike, &secret] {
            std::fs::create_dir_all(dir).expect("create folders");
        }
        let roots = canonical_roots(&[&downloads]);

        assert!(resolve_browse_path(Some(PathBuf::from("/")), &roots).is_err());
        assert!(resolve_browse_path(Some(secret), &roots).is_err());
        assert!(resolve_browse_path(Some(lookalike), &roots).is_err());
        assert!(resolve_browse_path(Some(downloads.join("..")), &roots).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn browse_refuses_symlinks_that_leave_the_roots() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let downloads = temp.path().join("downloads");
        let secret = temp.path().join("secret");
        std::fs::create_dir_all(&downloads).expect("create downloads");
        std::fs::create_dir_all(&secret).expect("create secret");
        std::os::unix::fs::symlink(&secret, downloads.join("escape")).expect("symlink");
        let roots = canonical_roots(&[&downloads]);

        assert!(resolve_browse_path(Some(downloads.join("escape")), &roots).is_err());
    }

    #[test]
    fn browse_refuses_everything_when_no_root_exists() {
        let temp = tempfile::tempdir().expect("temporary directory");
        assert!(resolve_browse_path(None, &[]).is_err());
        assert!(resolve_browse_path(Some(temp.path().to_path_buf()), &[]).is_err());
    }
}
