//! `GET /api/models` (`models.md` § What the web app receives): the catalog
//! of every entry the caller infers with, each model with the selection the
//! backend builds from it and each entry with its health, and what the
//! small model's Automatic resolves to. It wakes no Cloud and runs no model;
//! `refresh=true` waits for a shared forced refresh.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use demi_backend_providers::llm::small_model::automatic_small_model;
use demi_shared_types::MediaKind;
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
    let preferences = services.control.preferences(user.id.clone()).await?;
    let mut providers = services.assembly.model_catalog(&entries, refresh.0).await;
    let mut images = services.entry_media_models(&user.id, MediaKind::Image).await?;
    let mut videos = services.entry_media_models(&user.id, MediaKind::Video).await?;
    for provider in &mut providers {
        provider.image_models = images.remove(provider.provider_id.as_str()).unwrap_or_default();
        provider.video_models = videos.remove(provider.provider_id.as_str()).unwrap_or_default();
    }
    let small_model = automatic_small_model(&providers, &preferences);
    Ok(Json(ModelCatalog {
        providers,
        small_model,
    }))
}
