//! `GET/POST /conversations/:id/saved`, `POST .../saved/from-draft`,
//! `POST .../saved/:messageId/edit` and `DELETE .../saved/:messageId`
//! (`web-api.md` § Messages saved for later): the conversation's messages
//! saved for later. A save of the composer's message empties the
//! conversation's draft and an edit exchanges a message with the draft, each
//! in one transaction, while a plain save, Undo's, leaves the draft alone; an archived conversation reads its list and refuses
//! the rest, as it does its draft. Sending one is the conversation socket's
//! `send_saved` frame. A save names its message's id; one sent again with the
//! same id and content answers as the first did.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::saved::SavedRefusal;
use demi_backend_page_sync::Part;
use demi_backend_user_shard::services::Services;
use demi_web_api_protocol::drafts::{DRAFT_BYTES_MAX, DraftWritten};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::saved::{
    FromDraftAnswer, SAVED_MAX, SavedAnswer, SavedEdit, SavedFromDraft, SavedList, SavedRemoved,
    SavedSave,
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
    services.sync.mark(&user.id, Part::Conversation(record.id));
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
    services.sync.mark(&user.id, Part::Conversation(record.id));
    Ok((StatusCode::CREATED, Json(answer)))
}

/// Moves the message into the draft, and the draft into the list; the edit's
/// answer is kept by its `op` for a day.
pub(super) async fn edit(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path((id, message)): Path<(String, String)>,
    body: Result<JsonBody<SavedEdit>, ApiError>,
) -> Result<Json<DraftWritten>, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    let JsonBody(edit) = body?;
    let answer = services
        .control
        .edit_saved(record.id.clone(), edit.op, message, edit.id)
        .await?
        .map_err(refused)?;
    services.sync.mark(&user.id, Part::Conversation(record.id));
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
    services.sync.mark(&user.id, Part::Conversation(record.id));
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
