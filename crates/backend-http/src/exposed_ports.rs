//! The caller's exposed ports (`crates/backend-user-shard/src/shard/exposed_ports.rs`): each
//! route acts in the caller's shard, and a change reaches the caller's
//! pages as the `exposedPorts` part. Each refusal is in words for the user,
//! which the page shows as they are.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::exposed_ports::ExposedPortRefusal;
use demi_backend_user_shard::shard::exposed_ports::ExposedPortError;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::exposed_ports::{
    ExposedPortAnswer, ExposedPortList, ExposedPortPatch, NewExposedPort, refusal,
};
use demi_web_api_protocol::ids::ExposedPortId;

use super::AppState;
use super::body::{Worded, WordedBody};
use super::error::ApiError;
use super::gate::AuthUser;

impl Worded for NewExposedPort {
    fn refusal(report: &garde::Report) -> String {
        refusal(report)
    }
}

impl Worded for ExposedPortPatch {
    fn refusal(report: &garde::Report) -> String {
        refusal(report)
    }
}

/// `GET /exposed-ports`.
pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<ExposedPortList>, ApiError> {
    let ports = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.exposed_port_list().await })
        .await??;
    Ok(Json(ExposedPortList { ports }))
}

/// `POST /exposed-ports`: answers 201 with the row as stored.
pub(super) async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    WordedBody(new): WordedBody<NewExposedPort>,
) -> Result<(StatusCode, Json<ExposedPortAnswer>), ApiError> {
    let (port, new) = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.create_exposed_port(new).await })
        .await?
        .map_err(refused)?;
    // A port exposed already is the answer too, as it was.
    let status = if new { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(ExposedPortAnswer { port })))
}

/// `PATCH /exposed-ports/:id`.
pub(super) async fn patch(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    WordedBody(patch): WordedBody<ExposedPortPatch>,
) -> Result<Json<ExposedPortAnswer>, ApiError> {
    let id = port_id(id)?;
    let port = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.change_exposed_port(id, patch).await })
        .await?
        .map_err(refused)?;
    Ok(Json(ExposedPortAnswer { port }))
}

/// `DELETE /exposed-ports/:id`.
pub(super) async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = port_id(id)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.remove_exposed_port(id).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The id a path names; an empty one names none of the caller's.
fn port_id(id: String) -> Result<ExposedPortId, ApiError> {
    ExposedPortId::try_from(id).map_err(|_| refused(ExposedPortRefusal::NotFound.into()))
}

fn refused(error: ExposedPortError) -> ApiError {
    let message = error.to_string();
    match error {
        ExposedPortError::Refused(ExposedPortRefusal::NotFound) => {
            ApiError::new(StatusCode::NOT_FOUND, ErrorCode::ExposedPortNotFound, message)
        }
        ExposedPortError::Refused(ExposedPortRefusal::NoSuchHost) => {
            ApiError::new(StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound, message)
        }
        ExposedPortError::Refused(ExposedPortRefusal::PortTaken(port)) => {
            ApiError::new(StatusCode::CONFLICT, ErrorCode::ExposedPortTaken, message).with_param("port", port)
        }
        ExposedPortError::Storage(error) => error.into(),
    }
}
