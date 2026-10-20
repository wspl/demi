//! `GET/PUT /conversations/:id/draft` and `POST .../draft/replaced`: the
//! conversation's draft.
//!
//! - A save always changes the draft, keeping the version it replaced when it
//!   was built on an older revision.
//! - An archived conversation reads its draft and refuses the rest.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::drafts::{DraftRefusal, StagedFile};
use demi_backend_page_sync::Part;
use demi_conversation_socket_protocol::ClientContent;
use demi_web_api_protocol::drafts::{
    ATTACHMENT_MARK, DRAFT_BYTES_MAX, DraftAnswer, DraftSave, DraftWritten, ReplacedDraftAction,
};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::AttachmentId;

use super::body::JsonBody;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;
use demi_backend_user_shard::services::Services;

pub(super) async fn read(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<DraftAnswer>, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    let draft = services.control.draft(record.id).await?;
    Ok(Json(DraftAnswer { draft }))
}

/// Saves the draft: the text with one mark per file, and the files as a
/// frame names them.
pub(super) async fn save(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    body: Result<JsonBody<DraftSave>, ApiError>,
) -> Result<Json<DraftWritten>, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    let JsonBody(save) = body?;
    let files = staged_files(&save.text, save.files)?;
    let saved = services
        .control
        .save_draft(
            record.id.clone(),
            user.id.clone(),
            save.base,
            save.text,
            files,
            save.comments,
        )
        .await?
        .map_err(refused)?;
    services.sync.mark(&user.id, Part::Conversation(record.id));
    Ok(Json(saved))
}

/// Restores or dismisses the replaced version the request names.
pub(super) async fn replaced(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    body: Result<JsonBody<ReplacedDraftAction>, ApiError>,
) -> Result<Json<DraftWritten>, ApiError> {
    let record = owned(&services, &user.id, &id).await?;
    let JsonBody(request) = body?;
    let changed = services
        .control
        .change_replaced_draft(record.id.clone(), request.action, request.revision)
        .await?
        .map_err(refused)?;
    services.sync.mark(&user.id, Part::Conversation(record.id));
    Ok(Json(changed))
}

/// A draft's files as storage takes them, when `text` holds one mark per
/// file; a message saved for later names its files the same way.
pub(super) fn staged_files(
    text: &str,
    files: Vec<ClientContent>,
) -> Result<Vec<StagedFile>, ApiError> {
    let marks = text.chars().filter(|char| *char == ATTACHMENT_MARK).count();
    if marks != files.len() {
        return Err(ApiError::invalid_body(format!(
            "text: holds {marks} attachment marks for {} files",
            files.len()
        )));
    }
    files
        .into_iter()
        .enumerate()
        .map(|(index, file)| staged(index, file))
        .collect()
}

/// A draft's file as storage takes it: an upload or a file on a paired
/// device, the only files a message's text stands for.
fn staged(index: usize, file: ClientContent) -> Result<StagedFile, ApiError> {
    match file {
        ClientContent::Upload { r#ref, file_name } => {
            let id = AttachmentId::try_from(r#ref)
                .map_err(|error| ApiError::invalid_body(format!("files[{index}].ref: {error}")))?;
            Ok(StagedFile::Upload { id, file_name })
        }
        ClientContent::RemoteFile { device_id, path } => Ok(StagedFile::Remote { device_id, path }),
        _ => Err(ApiError::invalid_body(format!(
            "files[{index}]: a draft's file is an upload or a remote file"
        ))),
    }
}

fn refused(refusal: DraftRefusal) -> ApiError {
    match refusal {
        DraftRefusal::Archived => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::ConversationArchived,
            "The conversation is archived",
        ),
        DraftRefusal::UploadNotFound(id) => ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::UploadNotFound,
            format!("No upload {id}"),
        ),
        DraftRefusal::TooLarge => ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::TooLarge,
            format!("The draft is over its {DRAFT_BYTES_MAX}-byte limit"),
        )
        .with_param("limitBytes", DRAFT_BYTES_MAX),
        DraftRefusal::Changed => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::DraftChanged,
            "The draft's replaced version is not that one any more",
        ),
    }
}
