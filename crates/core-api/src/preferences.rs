//! The caller's preferences, which belong to the core's principal alone.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use demi_core::services::CoreServices;
use demi_core::settings;
use demi_core_sync::Part;
use demi_web_api_protocol::settings::{PreferencesPatch, UserPreferences};

use crate::body::JsonBody;
use crate::caller::Principal;
use crate::error::ApiError;

pub(crate) async fn preferences(
    State(services): State<Arc<CoreServices>>,
    Principal(user): Principal,
) -> Result<Json<UserPreferences>, ApiError> {
    let preferences = services.vault.stores().store_of(&user).await?.preferences().await?;
    Ok(Json(UserPreferences { preferences }))
}

/// Merges the patch into the caller's preferences; a reported locale must
/// name a known time zone and well-formed language tags.
pub(crate) async fn patch_preferences(
    State(services): State<Arc<CoreServices>>,
    Principal(user): Principal,
    JsonBody(patch): JsonBody<PreferencesPatch>,
) -> Result<Json<UserPreferences>, ApiError> {
    let patch =
        settings::check(patch).map_err(|error| ApiError::invalid_body(error.to_string()))?;
    let merge = move |saved| settings::merge(saved, patch);
    let preferences = services.vault.stores().store_of(&user).await?.patch_preferences(merge)
        .await?;
    services.sync.mark(&user, Part::Preferences);
    Ok(Json(UserPreferences { preferences }))
}
