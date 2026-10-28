//! The caller's integrations (`crates/backend-user-shard/src/integrations.rs`),
//! which Settings › Integrations makes and changes.
//!
//! Each route acts in the caller's shard. A token reaches a page only in the
//! answer of the request that made it.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::IntegrationId;
use demi_web_api_protocol::integrations::{IntegrationAnswer, IntegrationList, IntegrationPatch, IntegrationWithToken};

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;

/// `GET /integrations`.
pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<IntegrationList>, ApiError> {
    let integrations = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.integrations().await })
        .await??;
    Ok(Json(IntegrationList { integrations }))
}

/// `POST /integrations`: answers 201 with the integration and its token.
pub(super) async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<(StatusCode, Json<IntegrationWithToken>), ApiError> {
    let created = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.create_integration().await })
        .await??;
    Ok((StatusCode::CREATED, Json(created)))
}

/// `PATCH /integrations/:id`.
pub(super) async fn patch(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(patch): JsonBody<IntegrationPatch>,
) -> Result<Json<IntegrationAnswer>, ApiError> {
    let id = integration_id(id)?;
    let integration = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.update_integration(id, patch).await })
        .await??
        .ok_or_else(not_found)?;
    Ok(Json(IntegrationAnswer { integration }))
}

/// `POST /integrations/:id/token`: a new token, which the old one no longer
/// opens.
pub(super) async fn replace_token(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<IntegrationWithToken>, ApiError> {
    let id = integration_id(id)?;
    let replaced = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.replace_integration_token(id).await })
        .await??
        .ok_or_else(not_found)?;
    Ok(Json(replaced))
}

fn integration_id(id: String) -> Result<IntegrationId, ApiError> {
    IntegrationId::try_from(id).map_err(|_| not_found())
}

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "The integration is not there")
}
