//! The caller's credentials, which only their owner reads and changes: each route acts
//! in the caller's shard, and a change reaches the caller's pages as the `credentials`
//! part, which carries a visible field's value and never a secret one's. Only the reveal
//! answers every value. Each refusal is in words for the user, which the page shows as
//! they are. The reveal, which is the only route that answers a secret's value, takes
//! the session's confirmation that it is the user, as a handoff code does.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::credentials::CredentialRefusal;
use demi_backend_user_shard::shard::credentials::CredentialError;
use demi_web_api_protocol::credentials::{
    CredentialAnswer, CredentialPatch, CredentialList, CredentialValues, NewCredential, refusal,
};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::UserCredentialId;

use super::AppState;
use super::body::{Worded, WordedBody};
use super::error::ApiError;
use super::gate::{AuthUser, CurrentSession};
use super::sessions::require_verification;

impl Worded for NewCredential {
    fn refusal(report: &garde::Report) -> String {
        refusal(report)
    }
}

impl Worded for CredentialPatch {
    fn refusal(report: &garde::Report) -> String {
        refusal(report)
    }
}

/// `GET /credentials`.
pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<CredentialList>, ApiError> {
    let credentials = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.credential_list().await })
        .await??;
    Ok(Json(CredentialList { credentials }))
}

/// `GET /credentials/:id/values`, for a session verified in the last five
/// minutes.
pub(super) async fn values(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    session: CurrentSession,
    Path(id): Path<String>,
) -> Result<Json<CredentialValues>, ApiError> {
    require_verification(&state.services, &session)?;
    let id = credential_id(id)?;
    let values = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.credential_values(id).await })
        .await?
        .map_err(refused)?;
    Ok(Json(values))
}

/// `POST /credentials`: answers 201 with the credential as stored.
pub(super) async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    WordedBody(credential): WordedBody<NewCredential>,
) -> Result<(StatusCode, Json<CredentialAnswer>), ApiError> {
    let credential = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.create_credential(credential).await })
        .await?
        .map_err(refused)?;
    Ok((StatusCode::CREATED, Json(CredentialAnswer { credential })))
}

/// `PATCH /credentials/:id`.
pub(super) async fn patch(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    WordedBody(patch): WordedBody<CredentialPatch>,
) -> Result<Json<CredentialAnswer>, ApiError> {
    let id = credential_id(id)?;
    let credential = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.change_credential(id, patch).await })
        .await?
        .map_err(refused)?;
    Ok(Json(CredentialAnswer { credential }))
}

/// `DELETE /credentials/:id`.
pub(super) async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = credential_id(id)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.remove_credential(id).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The credential id a path names; an empty one names none of the
/// caller's.
fn credential_id(id: String) -> Result<UserCredentialId, ApiError> {
    UserCredentialId::try_from(id).map_err(|_| refused(CredentialRefusal::NotFound.into()))
}

fn refused(error: CredentialError) -> ApiError {
    let message = error.to_string();
    match error {
        CredentialError::Refused(CredentialRefusal::NotFound) => {
            ApiError::new(StatusCode::NOT_FOUND, ErrorCode::CredentialNotFound, message)
        }
        CredentialError::Refused(CredentialRefusal::NameTaken(_)) => {
            ApiError::new(StatusCode::CONFLICT, ErrorCode::CredentialNameTaken, message)
        }
        CredentialError::Refused(CredentialRefusal::MissingValue(_)) => ApiError::invalid_body(message),
        CredentialError::Storage(error) => error.into(),
    }
}
