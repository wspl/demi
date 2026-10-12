//! The caller's subagent settings (`web-api.md` § Subagents): the Subagents
//! switch, the limits and the profiles. The caller's shard checks a
//! profile's name, texts and model, as it does for `demi agent profiles`,
//! and writes, and the change reaches the caller's pages as the `subagents`
//! part.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::subagents::ProfileRefusal as Stored;
use demi_backend_host_access::transition::ChangeRefusal;
use demi_backend_user_shard::shard::subagents::ProfileRefusal;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::ProfileId;
use demi_web_api_protocol::subagents::{NewProfile, ProfileAnswer, ProfilePatch, SubagentsPatch};

use super::AppState;
use super::body::JsonBody;
use super::error::{ApiError, status_of};
use super::gate::AuthUser;

/// `PUT /subagents`: any of `{ enabled, maxLive, maxDepth }`; a limit
/// that is neither a whole number from 1 nor null fails the body's decoding.
pub(super) async fn switch(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(patch): JsonBody<SubagentsPatch>,
) -> Result<StatusCode, ApiError> {
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.patch_subagents(patch).await })
        .await??;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /subagents/profiles`: answers 201 with the profile as stored.
pub(super) async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(profile): JsonBody<NewProfile>,
) -> Result<(StatusCode, Json<ProfileAnswer>), ApiError> {
    let profile = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.create_profile(profile, true).await })
        .await?
        .map_err(refused)?;
    Ok((StatusCode::CREATED, Json(ProfileAnswer { profile })))
}

/// `PATCH /subagents/profiles/:id`.
pub(super) async fn patch(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(patch): JsonBody<ProfilePatch>,
) -> Result<Json<ProfileAnswer>, ApiError> {
    let id = profile_id(id)?;
    let profile = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.patch_profile(id, patch).await })
        .await?
        .map_err(refused)?;
    Ok(Json(ProfileAnswer { profile }))
}

/// `DELETE /subagents/profiles/:id`.
pub(super) async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = profile_id(id)?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.delete_profile(id).await })
        .await?
        .map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The profile id a path names; an empty one names none of the caller's.
fn profile_id(id: String) -> Result<ProfileId, ApiError> {
    ProfileId::try_from(id).map_err(|_| refused(Stored::NotFound.into()))
}

fn refused(refusal: ProfileRefusal) -> ApiError {
    let message = refusal.to_string();
    match refusal {
        ProfileRefusal::Text(_) => ApiError::invalid_body(message),
        ProfileRefusal::Stored(Stored::NotFound) => {
            ApiError::new(StatusCode::NOT_FOUND, ErrorCode::ProfileNotFound, message)
        }
        ProfileRefusal::Stored(Stored::Exists) => {
            ApiError::new(StatusCode::CONFLICT, ErrorCode::ProfileExists, message)
        }
        ProfileRefusal::Model(ChangeRefusal::Storage(error)) | ProfileRefusal::Storage(error) => {
            error.into()
        }
        ProfileRefusal::Model(refusal) => {
            let (code, status) = refusal.code();
            ApiError::new(status_of(status), code, message)
        }
    }
}
