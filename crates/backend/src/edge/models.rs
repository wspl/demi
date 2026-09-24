//! `GET /api/models` (`models.md` § What the browser receives): the catalog
//! of every entry the caller infers with, each model with the selection the
//! backend builds from it and each entry with its health. It wakes no Cloud
//! and runs no model; `refresh=true` waits for a shared forced refresh.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use demi_web_api::providers::ModelCatalog;
use demi_web_api::query::Refresh;

use super::error::ApiError;
use super::gate::AuthUser;
use super::query::QueryParams;
use crate::backend::Services;

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
