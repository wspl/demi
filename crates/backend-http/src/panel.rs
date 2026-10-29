//! The work panel's route: a request of changes, which the user's shard applies
//! whole or not at all and answers with the panel's revision; the
//! conversation's entry in the list carries the tabs.
//!
//! The backend applies a conversation's panel changes one at a time, the page's
//! and the plugin's alike. A plugin whose work on one conversation's tabs takes
//! several port operations, such as reading the browser's tabs and then adding
//! the missing ones, does that work one piece at a time per conversation, so a
//! change it makes is never based on what it read before its own earlier
//! change.
//!
//! The user creates a tab of a plugin's kind only through an intent the kind
//! opens; otherwise the plugin creates them.


use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_plugin_host::PanelError;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::panel::{PANEL_BYTES_MAX, PanelAnswer, PanelChanges};

use super::AppState;
use super::body::JsonBody;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;

/// `POST /conversations/:id/panel/changes { changes }`: applied in order,
/// all or none, as one change of the panel.
pub(super) async fn change(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    body: Result<JsonBody<PanelChanges>, ApiError>,
) -> Result<Json<PanelAnswer>, ApiError> {
    let conversation = owned(&state.services, &user.id, &id).await?.id;
    let JsonBody(PanelChanges { changes }) = body?;
    let (revision, changed) = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            shard.plugins().change_panel(conversation, changes).await
        })
        .await?
        .map_err(refused)?;
    Ok(Json(PanelAnswer { revision, changed }))
}

fn refused(error: PanelError) -> ApiError {
    let message = error.to_string();
    match error {
        PanelError::UnknownKind(_) => ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::UnknownPanelKind,
            message,
        ),
        PanelError::Full => ApiError::new(StatusCode::CONFLICT, ErrorCode::PanelFull, message),
        PanelError::TooLarge => {
            ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::TooLarge, message)
                .with_param("limitBytes", PANEL_BYTES_MAX)
        }
        PanelError::Missing => ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::ConversationNotFound,
            message,
        ),
        PanelError::Storage(error) => error.into(),
    }
}
