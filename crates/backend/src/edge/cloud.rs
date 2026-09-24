//! `/api/cloud` (`web-api.md` § Cloud): the caller's Cloud status, which
//! wakes nothing, and a reset named by the page's operation id.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use demi_web_api::cloud::{CloudReset, CloudResetAnswer, CloudStatus};
use demi_web_api::error::ErrorCode;

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use crate::managed::{CloudError, operation_dto};

pub(super) async fn status(State(state): State<AppState>, AuthUser(user): AuthUser) -> Result<Json<CloudStatus>, ApiError> {
    let status = state
        .shards
        .of(&user.id)
        .call(|shard, _| async move { shard.cloud_status().await })
        .await??;
    Ok(Json(status))
}

/// Admits the reset, or answers the one of the same id, with 202: the reset
/// runs on, and the status reports its phases.
pub(super) async fn reset(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(CloudReset { operation_id }): JsonBody<CloudReset>,
) -> Result<(StatusCode, Json<CloudResetAnswer>), ApiError> {
    let operation = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.reset_cloud(operation_id).await })
        .await?
        .map_err(|error| match error {
            // A reset that cannot start now is refused like a second reset.
            CloudError::Capacity => ApiError::new(StatusCode::CONFLICT, ErrorCode::CloudCapacity, error.to_string()),
            error => error.into(),
        })?;
    let answer = CloudResetAnswer {
        operation: operation_dto(&operation),
    };
    Ok((StatusCode::ACCEPTED, Json(answer)))
}
