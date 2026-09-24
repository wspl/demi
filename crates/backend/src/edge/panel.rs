//! `GET/PUT /conversations/:id/panel` (`web-api.md` § Work panel state): the
//! work panel's document, saved whole and read back as saved; an archived
//! conversation reads it and refuses a save.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_web_api::error::ErrorCode;
use demi_web_api::panel::{PANEL_BYTES_MAX, WorkPanel};

use super::body::JsonBody;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;
use crate::backend::Services;

pub(super) async fn read(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<WorkPanel>, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    let panel = services.control.panel(record.id).await?;
    Ok(Json(panel.unwrap_or_else(WorkPanel::empty)))
}

/// Replaces the panel. Whose conversation it is and whether it takes a save
/// are answered before the body.
pub(super) async fn save(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    body: Result<JsonBody<WorkPanel>, ApiError>,
) -> Result<StatusCode, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    if record.archived {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::ConversationArchived,
            "The conversation is archived",
        ));
    }
    let JsonBody(panel) = body?;
    // The panel's types serialize their fields as JSON strings, arrays and
    // objects with string keys, which serde_json never refuses.
    let document = serde_json::to_string(&panel).expect("a work panel serializes to JSON");
    if document.len() > PANEL_BYTES_MAX {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::TooLarge,
            format!("The work panel is over its {PANEL_BYTES_MAX}-byte limit"),
        ));
    }
    services.control.save_panel(record.id, document).await?;
    Ok(StatusCode::NO_CONTENT)
}
