//! The model routes (`crates/provider-registry/src/catalog.rs`): the
//! search of models.dev, and the user's Refresh of the catalog, which the
//! product state carries.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use demi_core_sync::Part;
use demi_core::provider_settings::CatalogError;
use demi_web_api_protocol::providers::ModelSearchResults;
use demi_web_api_protocol::query::ModelSearch;

use crate::error::ApiError;
use crate::caller::Principal;
use crate::query::QueryParams;
use demi_core::services::CoreServices;

/// `GET /api/models/search?q=&entry=`: the models.dev models the query
/// matches, filled from the entry's vendor where it lists them, from the
/// backend's kept copy of the document.
pub(crate) async fn search(
    State(services): State<Arc<CoreServices>>,
    Principal(user): Principal,
    QueryParams(ModelSearch { q, entry }): QueryParams<ModelSearch>,
) -> Result<Json<ModelSearchResults>, ApiError> {
    let results = services
        .search_models(&user, &q, entry.as_ref().map(|entry| entry.as_str()))
        .await
        .map_err(crate::providers::refused)?;
    Ok(Json(ModelSearchResults { results }))
}

/// `POST /api/models/refresh`: refreshes the source of every entry the
/// caller infers with, as the user's Refresh asks, and announces the catalog
/// on the caller's channels once it answered, whether it changed or not, so
/// a page shows what the refresh found.
pub(crate) async fn refresh(
    State(services): State<Arc<CoreServices>>,
    Principal(user): Principal,
) -> Result<StatusCode, ApiError> {
    services.model_catalog(&user, true).await.map_err(|error| match error {
        CatalogError::Storage(error) => ApiError::from(error),
        CatalogError::Registry(error) => ApiError::from(error),
    })?;
    services.sync.mark(&user, Part::Models);
    Ok(StatusCode::NO_CONTENT)
}
