//! The caller's subagent settings: the Subagents switch, the limits and the
//! profiles.
//!
//! The caller's core checks a profile's name, texts and model, as it does for
//! `demi agent profiles`, and writes. The change reaches the caller's pages as
//! the `subagents` part.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_core_store::subagents::ProfileRefusal as Stored;
use demi_core_host_access::transition::ChangeRefusal;
use demi_core::subagents::ProfileRefusal;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::ProfileId;
use demi_web_api_protocol::subagents::{NewProfile, ProfileAnswer, ProfilePatch, SubagentSettings, SubagentsPatch};

use crate::api::Api;
use crate::body::JsonBody;
use crate::error::{ApiError, status_of};
use crate::caller::Principal;

/// `GET /settings/subagents`: the switch, the limits and the profiles.
pub(crate) async fn read(
    State(state): State<Api>,
    Principal(user): Principal,
) -> Result<Json<SubagentSettings>, ApiError> {
    Ok(Json(state.store(&user).await?.subagent_settings().await?))
}

/// `PUT /subagents`: any of `{ enabled, maxLive, maxDepth }`; a limit
/// that is neither a whole number from 1 nor null fails the body's decoding.
pub(crate) async fn switch(
    State(state): State<Api>,
    Principal(user): Principal,
    JsonBody(patch): JsonBody<SubagentsPatch>,
) -> Result<StatusCode, ApiError> {
    state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.patch_subagents(patch).await })
        .await??;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /subagents/profiles`: answers 201 with the profile as stored.
pub(crate) async fn create(
    State(state): State<Api>,
    Principal(user): Principal,
    JsonBody(profile): JsonBody<NewProfile>,
) -> Result<(StatusCode, Json<ProfileAnswer>), ApiError> {
    let profile = state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.create_profile(profile, true).await })
        .await?
        .map_err(refused)?;
    Ok((StatusCode::CREATED, Json(ProfileAnswer { profile })))
}

/// `PATCH /subagents/profiles/:id`.
pub(crate) async fn patch(
    State(state): State<Api>,
    Principal(user): Principal,
    Path(id): Path<String>,
    JsonBody(patch): JsonBody<ProfilePatch>,
) -> Result<Json<ProfileAnswer>, ApiError> {
    let id = profile_id(id)?;
    let profile = state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.patch_profile(id, patch).await })
        .await?
        .map_err(refused)?;
    Ok(Json(ProfileAnswer { profile }))
}

/// `DELETE /subagents/profiles/:id`.
pub(crate) async fn delete(
    State(state): State<Api>,
    Principal(user): Principal,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = profile_id(id)?;
    state
        .shards
        .of(&user)
        .call(move |core, _| async move { core.delete_profile(id).await })
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
        ProfileRefusal::Text(_) | ProfileRefusal::Context(_) => ApiError::invalid_body(message),
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
