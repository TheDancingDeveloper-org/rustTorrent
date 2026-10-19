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
            (name.clone(), QbitCategory { name, save_path })
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
}

pub(super) async fn h_create_category(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: CreateCategoryForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if form.category.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "category name is required");
    }
    let save_path = (!form.save_path.is_empty()).then(|| form.save_path.into());
    match state
        .api_state
        .api
        .api_create_or_edit_category(form.category, save_path)
        .await
    {
        Ok(_) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn h_edit_category(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: CreateCategoryForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if form.category.is_empty() {
        return qbit_error(StatusCode::BAD_REQUEST, "category name is required");
    }
    let save_path = (!form.save_path.is_empty()).then(|| form.save_path.into());
    match state
        .api_state
        .api
        .api_create_or_edit_category(form.category, save_path)
        .await
    {
        Ok(_) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
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
