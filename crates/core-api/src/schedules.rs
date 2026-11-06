//! A conversation's schedules, as the web contract reaches them.
//!
//! - The user pauses, resumes or deletes a schedule of a conversation they
//!   own, in the caller's core; the change reaches the conversation's pages
//!   in its metadata. The agent does the rest, with `demi schedule`.
//! - A schedule's URL, `/hooks/<id>`, is authenticated by no session: its id
//!   and its secret are its only keys.

use std::time::Instant;

use axum::extract::{Path, Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use demi_core::schedules::{CALLS_PER_MINUTE, HookAnswer, HookCall, ScheduleError};
use demi_shared_types::ScheduleId;
use demi_web_api_protocol::error::ErrorCode;
use demi_core_store::StorageError;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use futures_util::future::BoxFuture;
use demi_web_api_protocol::schedules::{ScheduleChange, SchedulePatch};

use crate::api::Api;
use crate::body::{JsonBody, read};
use crate::error::ApiError;
use crate::caller::Principal;
use crate::page_client::client_address;

/// The one answer an unknown address and a wrong secret share.
pub(crate) fn unauthorized() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        ErrorCode::Unauthenticated,
        "The call carries no valid secret for this address",
    )
}

fn too_many() -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::RateLimited,
        format!("This address takes {CALLS_PER_MINUTE} calls a minute"),
    )
}

/// Which core of the process holds a schedule, which its URL's call
/// reaches: the server keeps a record of each schedule's owner, a program's
/// process looks among the cores it has open.
pub trait ScheduleOwners: Send + Sync {
    /// The principal whose core holds the schedule `id`; none for an id no
    /// open core holds.
    fn owner<'a>(&'a self, id: &'a ScheduleId) -> BoxFuture<'a, Result<Option<UserId>, StorageError>>;
}

/// `POST /hooks/:id`: a call of a schedule's URL. An unknown address gets
/// the answer a wrong secret gets.
pub async fn hook(
    State(state): State<Api>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let limits = &state.services.hook_limits;
    let now = Instant::now();
    let (parts, body) = request.into_parts();
    let client = client_address(&parts);
    let id = ScheduleId::try_from(id).ok();
    let owner = match &id {
        Some(id) => state.schedule_owners.owner(id).await?,
        None => None,
    };
    let admitted = match (&id, &owner) {
        (Some(id), Some(_)) => limits.admit_known(id, now),
        _ => limits.admit_unknown(client, now),
    };
    if !admitted {
        return Err(too_many());
    }
    let (Some(id), Some(owner), true) = (id, owner, parts.method == Method::POST) else {
        return Err(unauthorized());
    };
    let method = parts.method.to_string();
    let headers = parts
        .headers
        .iter()
        .filter_map(|(name, value)| Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned())))
        .collect();
    let body = read(Request::from_parts(parts, body), &state).await?;
    let call = HookCall { method, headers, body };
    let answer = state
        .shards
        .of(&owner)
        .call(move |core, _| async move { core.hook_call(&id, call).await })
        .await??;
    match answer {
        HookAnswer::Taken => Ok(StatusCode::OK.into_response()),
        HookAnswer::Refused => Err(unauthorized()),
    }
}

/// `PATCH /conversations/:id/schedules/:schedule`: the user pauses or resumes
/// one of the conversation's schedules, 204.
pub(crate) async fn change(
    State(state): State<Api>,
    Principal(user): Principal,
    Path((conversation, id)): Path<(String, String)>,
    JsonBody(change): JsonBody<ScheduleChange>,
) -> Result<StatusCode, ApiError> {
    let conversation = ConversationId::try_from(conversation).map_err(|_| refused(ScheduleError::NotFound))?;
    let id = ScheduleId::try_from(id).map_err(|_| refused(ScheduleError::NotFound))?;
    let patch = SchedulePatch { paused: Some(change.paused), ..SchedulePatch::default() };
    state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.change_schedule(&conversation, &id, patch, false).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /conversations/:id/schedules/:schedule`: the user deletes one of
/// the conversation's schedules, 204.
pub(crate) async fn delete(
    State(state): State<Api>,
    Principal(user): Principal,
    Path((conversation, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let conversation = ConversationId::try_from(conversation).map_err(|_| refused(ScheduleError::NotFound))?;
    let id = ScheduleId::try_from(id).map_err(|_| refused(ScheduleError::NotFound))?;
    state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.delete_schedule(&conversation, &id).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

fn refused(error: ScheduleError) -> ApiError {
    let message = error.to_string();
    let (status, code) = match error {
        ScheduleError::NotFound => (StatusCode::NOT_FOUND, ErrorCode::ScheduleNotFound),
        ScheduleError::Storage(error) => return error.into(),
        // The rest are an agent's, which `demi schedule` words.
        other => return ApiError::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidBody, other.to_string()),
    };
    ApiError::new(status, code, message)
}
