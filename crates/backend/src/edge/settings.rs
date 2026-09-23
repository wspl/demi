//! The instance's settings and the caller's preferences (`web-api.md` §
//! User preferences). Preferences belong to the signed-in user alone, in
//! both instance modes.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use demi_web_api::settings::{PreferencesPatch, Settings, UserPreferences};

use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use crate::backend::Services;
use crate::settings;

pub(super) async fn settings(State(services): State<Arc<Services>>) -> Json<Settings> {
    Json(Settings { mode: services.mode })
}

pub(super) async fn preferences(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<Json<UserPreferences>, ApiError> {
    let preferences = services.control.preferences(user.id).await?;
    Ok(Json(UserPreferences { preferences }))
}

/// Merges the patch into the caller's preferences; a reported locale must
/// name a known time zone and well-formed language tags.
pub(super) async fn patch_preferences(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(patch): JsonBody<PreferencesPatch>,
) -> Result<Json<UserPreferences>, ApiError> {
    let patch = settings::check(patch).map_err(|error| ApiError::invalid_body(error.to_string()))?;
    let preferences = services.control.patch_preferences(user.id, patch).await?;
    Ok(Json(UserPreferences { preferences }))
}
