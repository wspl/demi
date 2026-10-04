//! The caller's subagent settings (`web-api.md` § Subagents): the Subagent
//! switch and the profiles. The edge checks a body's name and texts; the
//! caller's shard checks a profile's model against its entry's catalog and
//! writes, and the change reaches the caller's pages as the `subagents`
//! part.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_accounts::subagents::{check_new, check_patch};
use demi_backend_database::subagents::ProfileRefusal as Stored;
use demi_backend_host_access::transition::ChangeRefusal;
use demi_backend_user_shard::shard::subagents::ProfileRefusal;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::ProfileId;
use demi_web_api_protocol::subagents::{NewProfile, ProfileAnswer, ProfilePatch, SubagentSwitch};

use super::AppState;
use super::body::JsonBody;
use super::error::{ApiError, status_of};
use super::gate::AuthUser;

/// `PUT /subagents { enabled }`.
pub(super) async fn switch(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(SubagentSwitch { enabled }): JsonBody<SubagentSwitch>,
) -> Result<StatusCode, ApiError> {
    state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.switch_subagents(enabled).await })
        .await??;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /subagents/profiles`: answers 201 with the profile as stored.
pub(super) async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(profile): JsonBody<NewProfile>,
) -> Result<(StatusCode, Json<ProfileAnswer>), ApiError> {
    check_new(&profile).map_err(|error| ApiError::invalid_body(error.to_string()))?;
    let profile = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.create_profile(profile).await })
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
    check_patch(&patch).map_err(|error| ApiError::invalid_body(error.to_string()))?;
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
