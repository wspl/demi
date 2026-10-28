//! The conversation's messages saved for later: `POST
//! /conversations/:id/saved`, `POST .../saved/from-draft`, and `GET`, `PUT` and
//! `DELETE .../saved/:messageId`; `GET .../saved` lists them, which the page reads
//! when the saved menu opens.
//!
//! - A save of the composer's message empties the conversation's draft in one
//!   transaction. A plain save, Undo's, and the write of an edit's Save, which
//!   keeps the message in its place, leave the draft alone.
//! - An archived conversation reads its messages and refuses the rest, as it
//!   does its draft.
//! - A save names its message's id; one sent again with the same id and content
//!   answers as the first did.
//!
//! Sending one, as it is or as an edit changed it, is the conversation lane's
//! `send_saved` frame.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::saved::{SavedRefusal, SavedSharers};
use demi_backend_page_sync::Part;
use demi_backend_user_shard::services::Services;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use demi_web_api_protocol::saved::{
    FromDraftAnswer, SAVED_MAX, SavedAnswer, SavedFromDraft, SavedListing, SavedMessage, SavedRemoved, SavedSave,
    SavedWrite,
};

use super::body::JsonBody;
use super::conversations::owned;
use super::drafts::staged_files;
use super::error::ApiError;
use super::gate::AuthUser;

/// `GET /conversations/:id/saved`: the messages saved for later, each with
/// its opening, under the list's revision.
pub(super) async fn list(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<SavedListing>, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    Ok(Json(services.control.saved_listing(record.id).await?))
}

/// `GET /conversations/:id/saved/:messageId`: the message whole, which the
/// page reads as it uses it; the metadata lists each with its opening alone.
pub(super) async fn read(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path((id, message)): Path<(String, String)>,
) -> Result<Json<SavedMessage>, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    services
        .control
        .saved_message(record.id, message)
        .await?
        .map(Json)
        .ok_or_else(|| refused(SavedRefusal::NotFound))
}

/// Saves the composer's message for later and empties the draft.
pub(super) async fn from_draft(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    body: Result<JsonBody<SavedFromDraft>, ApiError>,
) -> Result<(StatusCode, Json<FromDraftAnswer>), ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    let JsonBody(save) = body?;
    let files = staged_files(&save.text, save.files)?;
    let answer = services
        .control
        .save_from_draft(record.id.clone(), user.id.clone(), save.id, save.text, files)
        .await?
        .map_err(refused)?;
    mark_sharers(&services, &user.id, record.id).await;
    Ok((StatusCode::CREATED, Json(answer)))
}

/// Adds a message for later, leaving the draft as it is.
pub(super) async fn save(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    body: Result<JsonBody<SavedSave>, ApiError>,
) -> Result<(StatusCode, Json<SavedAnswer>), ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    let JsonBody(save) = body?;
    let files = staged_files(&save.text, save.files)?;
    let answer = services
        .control
        .add_saved(record.id.clone(), user.id.clone(), save.id, save.text, files)
        .await?
        .map_err(refused)?;
    mark_sharers(&services, &user.id, record.id).await;
    Ok((StatusCode::CREATED, Json(answer)))
}

/// Writes the message's new content in its place, leaving the draft as it
/// is; a message gone, as another page deleted or sent it, is refused.
pub(super) async fn write(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path((id, message)): Path<(String, String)>,
    body: Result<JsonBody<SavedWrite>, ApiError>,
) -> Result<Json<SavedAnswer>, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    let JsonBody(write) = body?;
    let files = staged_files(&write.text, write.files)?;
    let answer = services
        .control
        .write_saved(record.id.clone(), user.id.clone(), message, write.text, files)
        .await?
        .map_err(refused)?;
    mark_sharers(&services, &user.id, record.id).await;
    Ok(Json(answer))
}

/// Removes the message; one already gone answers the same, without a message.
pub(super) async fn remove(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path((id, message)): Path<(String, String)>,
) -> Result<Json<SavedRemoved>, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    let (revisions, message) = services
        .control
        .delete_saved(record.id.clone(), message)
        .await?
        .map_err(refused)?;
    mark_sharers(&services, &user.id, record.id).await;
    Ok(Json(SavedRemoved { revisions, message }))
}

fn refused(refusal: SavedRefusal) -> ApiError {
    match refusal {
        SavedRefusal::Archived => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::ConversationArchived,
            "The conversation is archived",
        ),
        SavedRefusal::UploadNotFound(id) => ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::UploadNotFound,
            format!("No upload {id}"),
        ),
        SavedRefusal::Full => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::SavedFull,
            format!("The list keeps {SAVED_MAX} messages saved for later"),
        )
        .with_param("limit", SAVED_MAX),
        SavedRefusal::IdTaken => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::IdTaken,
            "A message saved for later has that id already",
        ),
        SavedRefusal::NotFound => ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::SavedNotFound,
            "No such message saved for later",
        ),
    }
}

/// Tells every page that shows a conversation whose list `conversation`
/// shows, its project's conversations when the project shares one, that
/// the list changed, and the projects' record its count when it is a
/// project's. A failed read tells the conversation's own pages.
async fn mark_sharers(services: &Services, user: &UserId, conversation: ConversationId) {
    let sharers = match services.control.saved_sharers(conversation.clone()).await {
        Ok(sharers) => sharers,
        Err(error) => {
            tracing::warn!(
                event = "saved_message.sharers_unread",
                %conversation,
                error = &error as &dyn std::error::Error,
                "the conversations that share a saved messages list were not read"
            );
            SavedSharers { key: conversation.to_string(), conversations: vec![conversation], project: false }
        }
    };
    for sharer in sharers.conversations {
        services.sync.mark(user, Part::Metadata(sharer));
    }
    if sharers.project {
        services.sync.mark(user, Part::Projects);
    }
}
