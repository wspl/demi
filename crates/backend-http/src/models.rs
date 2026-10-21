//! The model routes (`crates/backend-providers/src/llm/catalog.rs`): the
//! search of models.dev, and the user's Refresh of the catalog, which the
//! product state carries (`docs/decisions/0029-a-conversation-opens-whole.md`).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use demi_backend_page_sync::Part;
use demi_backend_user_shard::provider_settings::CatalogError;
use demi_web_api_protocol::providers::ModelSearchResults;
use demi_web_api_protocol::query::ModelSearch;

use super::error::ApiError;
use super::gate::AuthUser;
use super::query::QueryParams;
use demi_backend_user_shard::services::Services;

/// `GET /api/models/search?q=&entry=`: the models.dev models the query
/// matches, filled from the entry's vendor where it lists them, from the
/// backend's kept copy of the document.
pub(super) async fn search(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    QueryParams(ModelSearch { q, entry }): QueryParams<ModelSearch>,
) -> Result<Json<ModelSearchResults>, ApiError> {
    let results = services
        .search_models(&user.id, &q, entry.as_ref().map(|entry| entry.as_str()))
        .await
        .map_err(super::providers::refused)?;
    Ok(Json(ModelSearchResults { results }))
}

/// `POST /api/models/refresh`: refreshes the source of every entry the
/// caller infers with, as the user's Refresh asks, and announces the catalog
/// on the caller's channels once it answered, whether it changed or not, so
/// a page shows what the refresh found.
pub(super) async fn refresh(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<StatusCode, ApiError> {
    services.model_catalog(&user.id, true).await.map_err(|error| match error {
        CatalogError::Storage(error) => ApiError::from(error),
        CatalogError::Assembly(error) => ApiError::from(error),
    })?;
    services.sync.mark(&user.id, Part::Models);
    Ok(StatusCode::NO_CONTENT)
}
