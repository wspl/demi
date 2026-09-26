//! `/api/exposes` (`web-api.md` § Exposes): the caller's exposes, a new one
//! on a device of the caller's that is connected, a renewal and a removal.
//! A request for an expose hostname never reaches these routes: the public
//! relay (`edge::expose`) answers it before routing.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_web_api::exposes::{CreateExpose, ExposeAnswer, Exposes};
use demi_web_api::ids::ExposeId;

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use crate::expose::ExposeError;

pub(super) async fn list(State(state): State<AppState>, AuthUser(user): AuthUser) -> Result<Json<Exposes>, ApiError> {
    let exposes = state
        .shards
        .of(&user.id)
        .call(|shard, _| async move { shard.list_exposes().await })
        .await??;
    Ok(Json(Exposes { exposes }))
}

pub(super) async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(CreateExpose { device_id, address }): JsonBody<CreateExpose>,
) -> Result<(StatusCode, Json<ExposeAnswer>), ApiError> {
    let expose = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.add_expose(&device_id, address).await })
        .await??;
    Ok((StatusCode::CREATED, Json(ExposeAnswer { expose })))
}

pub(super) async fn renew(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ExposeAnswer>, ApiError> {
    let id = expose_id(id)?;
    let expose = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.renew_expose(&id).await })
        .await??;
    Ok(Json(ExposeAnswer { expose }))
}

pub(super) async fn remove(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = expose_id(id)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.remove_expose(&id).await })
        .await??;
    Ok(StatusCode::NO_CONTENT)
}

/// The expose a path names; a text that is no expose id names none.
fn expose_id(text: String) -> Result<ExposeId, ExposeError> {
    ExposeId::try_from(text.as_str()).map_err(|_| ExposeError::NotFound(text))
}
