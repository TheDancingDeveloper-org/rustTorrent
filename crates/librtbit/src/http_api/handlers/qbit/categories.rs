//! Torrents controller (`torrents/*`): category endpoints.

use super::*;

// ---------------------------------------------------------------------------
// Category endpoints
// ---------------------------------------------------------------------------

pub(super) async fn h_categories(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let map: HashMap<String, QbitCategory> = state
        .api_state
        .api
        .api_list_categories()
        .into_iter()
        .map(|(name, category)| {
            let save_path = category
                .save_path
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            (
                name.clone(),
                QbitCategory {
                    name,
                    save_path,
                    ratio_limit: category.ratio_limit.unwrap_or(-1.0),
                    seeding_time_limit: category
                        .seeding_time_limit
                        .map_or(-1, |d| (d.as_secs() / 60) as i64),
                    inactive_seeding_time_limit: category
                        .inactive_seeding_time_limit
                        .map_or(-1, |d| (d.as_secs() / 60) as i64),
                },
            )
        })
        .collect();
    axum::Json(serde_json::to_value(&map).unwrap_or_default())
}

#[derive(Deserialize, Default)]
pub(super) struct CreateCategoryForm {
    #[serde(default)]
    pub(super) category: String,
    #[serde(default, alias = "savePath")]
    pub(super) save_path: String,
    #[serde(default, alias = "ratioLimit")]
    pub(super) ratio_limit: Option<f64>,
    #[serde(default, alias = "seedingTimeLimit")]
    pub(super) seeding_time_limit: Option<i64>,
    #[serde(default, alias = "inactiveSeedingTimeLimit")]
    pub(super) inactive_seeding_time_limit: Option<i64>,
}

/// Upstream rejects names with a leading or trailing `/`, or an empty path
/// segment (`a//b`).
pub(super) fn is_valid_category_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('/') && !name.ends_with('/') && !name.contains("//")
}

/// `torrents/createCategory`: 400 for a missing or invalid name, 409 when the
/// category already exists.
pub(super) async fn h_create_category(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: CreateCategoryForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if form.category.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "Category cannot be empty");
    }
    if !is_valid_category_name(&form.category) {
        return qbit_error(StatusCode::BAD_REQUEST, "Incorrect category name");
    }
    if state
        .api_state
        .api
        .session()
        .categories()
        .exists(&form.category)
    {
        return qbit_error(StatusCode::CONFLICT, "Unable to create category");
    }
    save_category(&state, form).await
}

/// `torrents/editCategory`: 400 for a missing name, 409 when the category
/// does not exist. An empty `savePath` resets the category to the default.
pub(super) async fn h_edit_category(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: CreateCategoryForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if form.category.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "Category cannot be empty");
    }
    if !state
        .api_state
        .api
        .session()
        .categories()
        .exists(&form.category)
    {
        return qbit_error(StatusCode::CONFLICT, "Unable to edit category");
    }
    save_category(&state, form).await
}

async fn save_category(state: &QbitState, form: CreateCategoryForm) -> axum::response::Response {
    let save_path = (!form.save_path.is_empty()).then(|| form.save_path.into());
    match state
        .api_state
        .api
        .api_create_or_edit_category(
            form.category,
            save_path,
            crate::session::ShareLimits {
                max_ratio: form.ratio_limit.filter(|v| *v >= 0.0),
                max_seeding_time: form
                    .seeding_time_limit
                    .and_then(crate::http_api::handlers::qbit::torrentcreator::minutes_from_qbit),
                max_inactive_seeding_time: form
                    .inactive_seeding_time_limit
                    .and_then(crate::http_api::handlers::qbit::torrentcreator::minutes_from_qbit),
            },
        )
        .await
    {
        Ok(_) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => qbit_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

#[derive(Deserialize, Default)]
pub(super) struct RemoveCategoriesForm {
    #[serde(default)]
    pub(super) categories: String,
}

pub(super) async fn h_remove_categories(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: RemoveCategoriesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    for name in form.categories.split('\n') {
        let name = name.trim();
        if !name.is_empty()
            && let Err(error) = state.api_state.api.api_remove_category(name).await
        {
            warn!(%error, %name, "qbit compat: error removing category");
        }
    }
    "Ok."
}
