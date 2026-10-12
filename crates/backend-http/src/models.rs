//! `GET /api/models` (`models.md` § What the web app receives): the catalog
//! of every entry the caller infers with, each model with the selection the
//! backend builds from it and each entry with its health. It wakes no Cloud
//! and runs no model; `refresh=true` waits for a shared forced refresh.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use demi_web_api_protocol::providers::{ModelCatalog, ModelSearchResults};
use demi_web_api_protocol::query::{ModelSearch, Refresh};

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

pub(super) async fn models(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    QueryParams(Refresh { refresh }): QueryParams<Refresh>,
) -> Result<Json<ModelCatalog>, ApiError> {
    let owner = services.vault.owner_for(&user.id).await?;
    let entries = services.vault.entries(owner).await?;
    let providers = services.assembly.model_catalog(&entries, refresh.0).await;
    Ok(Json(ModelCatalog { providers }))
}
