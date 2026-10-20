//! The conversation's messages saved for later: `GET/POST
//! /conversations/:id/saved`, `POST .../saved/from-draft`, and `PUT` and
//! `DELETE .../saved/:messageId`.
//!
//! - A save of the composer's message empties the conversation's draft in one
//!   transaction. A plain save, Undo's, and the write of an edit's Save, which
//!   keeps the message in its place, leave the draft alone.
//! - An archived conversation reads its list and refuses the rest, as it does
//!   its draft.
//! - A save names its message's id; one sent again with the same id and content
//!   answers as the first did.
//!
//! Sending one, as it is or as an edit changed it, is the conversation socket's
//! `send_saved` frame.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::saved::SavedRefusal;
use demi_backend_page_sync::Part;
use demi_backend_user_shard::services::Services;
use demi_web_api_protocol::drafts::DRAFT_BYTES_MAX;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::saved::{
    FromDraftAnswer, SAVED_MAX, SavedAnswer, SavedFromDraft, SavedList, SavedRemoved, SavedSave,
    SavedWrite,
};

use super::body::JsonBody;
use super::conversations::owned;
use super::drafts::staged_files;
use super::error::ApiError;
use super::gate::AuthUser;

pub(super) async fn list(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<SavedList>, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    Ok(Json(services.control.saved_messages(record.id).await?))
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
        .save_from_draft(record.id.clone(), user.id.clone(), save.id, save.base, save.text, files)
        .await?
        .map_err(refused)?;
    services.sync.mark(&user.id, Part::Metadata(record.id));
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
    services.sync.mark(&user.id, Part::Metadata(record.id));
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
    services.sync.mark(&user.id, Part::Metadata(record.id));
    Ok(Json(answer))
}

/// Removes the message; one already gone answers the same.
pub(super) async fn remove(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path((id, message)): Path<(String, String)>,
) -> Result<Json<SavedRemoved>, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    let revisions = services
        .control
        .delete_saved(record.id.clone(), message)
        .await?
        .map_err(refused)?;
    services.sync.mark(&user.id, Part::Metadata(record.id));
    Ok(Json(SavedRemoved { revisions }))
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
        SavedRefusal::TooLarge => ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::TooLarge,
            format!("The message is over its {DRAFT_BYTES_MAX}-byte limit"),
        )
        .with_param("limitBytes", DRAFT_BYTES_MAX),
        SavedRefusal::Full => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::SavedFull,
            format!("The conversation keeps {SAVED_MAX} messages saved for later"),
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
