//! The caller's credentials, which only their owner lists, creates, changes
//! and deletes.
//!
//! - Each route acts in the caller's core.
//! - A change reaches the caller's pages as the `credentials` part, which
//!   carries a visible field's value and never a secret one's. Only
//!   `reveal` answers a secret's value, to a caller who proved itself
//!   lately, for a reply's card to copy or show.
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

use crate::api::Api;
use crate::body::{Worded, WordedBody};
use crate::error::ApiError;
use crate::caller::{CallerOf, Principal};

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
pub(crate) async fn list(
    State(state): State<Api>,
    Principal(user): Principal,
) -> Result<Json<CredentialList>, ApiError> {
    let credentials = state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.credential_list().await })
        .await??;
    Ok(Json(CredentialList { credentials }))
}

/// `POST /credentials`: answers 201 with the credential as stored.
pub(crate) async fn create(
    State(state): State<Api>,
    Principal(user): Principal,
    WordedBody(credential): WordedBody<NewCredential>,
) -> Result<(StatusCode, Json<CredentialAnswer>), ApiError> {
    let credential = state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.create_credential(credential).await })
        .await?
        .map_err(refused)?;
    Ok((StatusCode::CREATED, Json(CredentialAnswer { credential })))
}

/// `PATCH /credentials/:id`: answers the credential as it now is.
pub(crate) async fn change(
    State(state): State<Api>,
    Principal(user): Principal,
    Path(id): Path<String>,
    WordedBody(patch): WordedBody<CredentialPatch>,
) -> Result<Json<CredentialAnswer>, ApiError> {
    let id = credential_id(id)?;
    let credential = state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.change_credential(id, patch).await })
        .await?
        .map_err(refused)?;
    Ok(Json(CredentialAnswer { credential }))
}

/// `DELETE /credentials/:id`.
pub(crate) async fn delete(
    State(state): State<Api>,
    Principal(user): Principal,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = credential_id(id)?;
    state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.remove_credential(id).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /credentials/:id/fields/:field/reveal`: a field's value, for a
/// caller who proved itself lately, as a server's session verified in the
/// last five minutes, so a browser left signed in shows no secret.
pub(crate) async fn reveal(
    State(state): State<Api>,
    CallerOf(caller): CallerOf,
    Path((id, field)): Path<(String, String)>,
) -> Result<Json<RevealedValue>, ApiError> {
    caller.proved_lately()?;
    let id = credential_id(id)?;
    let value = state
        .shards
        .of(caller.principal())
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
