//! The caller's credentials, which only their owner lists, creates, changes
//! and deletes.
//!
//! - Each route acts in the caller's core.
//! - A change reaches the caller's pages as the `credentials` part, which
//!   carries a visible field's value and never a secret one's. Only
//!   `reveal` answers a secret's value, to the owner whose session's
//!   verification holds, for a reply's card to copy or show.
//! - A change writes values, a secret one's included, and answers none of a
//!   secret's.
//! - Each refusal is in words for the user, which the page shows as they are.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_core_store::credentials::CredentialRefusal;
use demi_core::credentials::CredentialError;
use demi_web_api_protocol::credentials::{
    CredentialAnswer, CredentialList, CredentialPatch, NewCredential, RevealedValue, refusal,
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
        .call(move |core, _| async move { core.credential_list().await })
        .await??;
    Ok(Json(CredentialList { credentials }))
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
        .call(move |core, _| async move { core.create_credential(credential).await })
        .await?
        .map_err(refused)?;
    Ok((StatusCode::CREATED, Json(CredentialAnswer { credential })))
}

/// `PATCH /credentials/:id`: answers the credential as it now is.
pub(super) async fn change(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    WordedBody(patch): WordedBody<CredentialPatch>,
) -> Result<Json<CredentialAnswer>, ApiError> {
    let id = credential_id(id)?;
    let credential = state
        .shards
        .of(&user.id)
        .call(move |core, _| async move { core.change_credential(id, patch).await })
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
        .call(move |core, _| async move { core.remove_credential(id).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /credentials/:id/fields/:field/reveal`: a field's value, for a
/// session verified in the last five minutes, so a browser left signed in
/// shows no secret.
pub(super) async fn reveal(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    session: CurrentSession,
    Path((id, field)): Path<(String, String)>,
) -> Result<Json<RevealedValue>, ApiError> {
    require_verification(&state.services, &session)?;
    let id = credential_id(id)?;
    let value = state
        .shards
        .of(&user.id)
        .call(move |core, _| async move { core.reveal_credential(id, field).await })
        .await?
        .map_err(refused)?;
    Ok(Json(RevealedValue { value }))
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
        CredentialError::Storage(error) => error.into(),
    }
}
