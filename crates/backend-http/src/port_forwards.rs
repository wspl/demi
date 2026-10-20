//! The caller's port forwards (`crates/runner-protocol/src/forward.rs`).
//!
//! - Each route acts in the caller's shard.
//! - A change reaches the caller's pages as the `portForwards` part.
//! - Each refusal is in words for the user, which the page shows as they are.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::port_forwards::PortForwardRefusal;
use demi_backend_user_shard::shard::port_forwards::PortForwardError;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::PortForwardId;
use demi_web_api_protocol::port_forwards::{PortForwardAnswer, PortForwardList, PortForwardPatch, refusal};

use super::AppState;
use super::body::{Worded, WordedBody};
use super::error::ApiError;
use super::gate::AuthUser;

impl Worded for PortForwardPatch {
    fn refusal(report: &garde::Report) -> String {
        refusal(report)
    }
}

/// `GET /port-forwards`.
pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<PortForwardList>, ApiError> {
    let forwards = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.port_forward_list().await })
        .await??;
    Ok(Json(PortForwardList { forwards }))
}

/// `POST /port-forwards`: answers 201 with the forward as stored.
pub(super) async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    WordedBody(patch): WordedBody<PortForwardPatch>,
) -> Result<(StatusCode, Json<PortForwardAnswer>), ApiError> {
    let forward = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.create_port_forward(patch).await })
        .await?
        .map_err(refused)?;
    Ok((StatusCode::CREATED, Json(PortForwardAnswer { forward })))
}

/// `PATCH /port-forwards/:id`.
pub(super) async fn patch(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    WordedBody(patch): WordedBody<PortForwardPatch>,
) -> Result<Json<PortForwardAnswer>, ApiError> {
    let id = forward_id(id)?;
    let forward = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.change_port_forward(id, patch).await })
        .await?
        .map_err(refused)?;
    Ok(Json(PortForwardAnswer { forward }))
}

/// `DELETE /port-forwards/:id`.
pub(super) async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = forward_id(id)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.remove_port_forward(id).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /port-forwards/:id/retry`: Try Again.
pub(super) async fn retry(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = forward_id(id)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.retry_port_forward(id).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The forward id a path names; an empty one names none of the caller's.
fn forward_id(id: String) -> Result<PortForwardId, ApiError> {
    PortForwardId::try_from(id).map_err(|_| refused(PortForwardRefusal::NotFound.into()))
}

fn refused(error: PortForwardError) -> ApiError {
    let message = error.to_string();
    match error {
        PortForwardError::Refused(PortForwardRefusal::NotFound) => {
            ApiError::new(StatusCode::NOT_FOUND, ErrorCode::PortForwardNotFound, message)
        }
        PortForwardError::Refused(PortForwardRefusal::NameTaken(name)) => {
            ApiError::new(StatusCode::CONFLICT, ErrorCode::PortForwardNameTaken, message).with_param("name", name)
        }
        PortForwardError::Refused(PortForwardRefusal::NoSuchHost) => {
            ApiError::new(StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound, message)
        }
        PortForwardError::Refused(PortForwardRefusal::NoHost) => ApiError::invalid_body(message),
        PortForwardError::Storage(error) => error.into(),
    }
}
