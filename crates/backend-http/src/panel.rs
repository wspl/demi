//! The work panel's routes (`web-api.md` § Work panel state): the read of a
//! conversation's tabs, and a request of changes, which the user's shard
//! applies whole or not at all and answers with the panel's revision. An archived
//! conversation reads its tabs and refuses every change.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_plugins::PanelError;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use demi_web_api_protocol::panel::{PanelAnswer, PanelChanges, WorkPanel};

use super::AppState;
use super::body::JsonBody;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;
use demi_backend_user_shard::services::Services;

/// `GET /conversations/:id/panel`.
pub(super) async fn read(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<WorkPanel>, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    Ok(Json(services.control.panel(record.id).await?))
}

/// `POST /conversations/:id/panel/changes { changes }`: applied in order,
/// all or none, as one change of the panel.
pub(super) async fn change(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    body: Result<JsonBody<PanelChanges>, ApiError>,
) -> Result<Json<PanelAnswer>, ApiError> {
    let conversation = changeable(&state, &user.id, &id).await?;
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

/// The caller's conversation `id`, which takes a change: whose
/// conversation it is and whether it is archived are answered before the
/// body.
async fn changeable(state: &AppState, user: &UserId, id: &str) -> Result<ConversationId, ApiError> {
    let record = owned(&state.services, user, id).await?;
    if record.archived {
        return Err(archived());
    }
    Ok(record.id)
}

fn archived() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        ErrorCode::ConversationArchived,
        "The conversation is archived",
    )
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
        }
        PanelError::Archived => archived(),
        PanelError::Missing => ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::ConversationNotFound,
            message,
        ),
        PanelError::Storage(error) => error.into(),
    }
}
