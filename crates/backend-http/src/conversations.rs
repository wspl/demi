//! `/api/conversations`.
//!
//! The routes:
//!
//! - creating a conversation under the id the web app chose;
//! - listing the caller's conversations;
//! - changing their fields one patch at a time or in a batch;
//! - forking one, acknowledging its output and deleting it;
//! - its socket, `WS /conversations/:id/stream`, which moves into the caller's
//!   shard once upgraded.
//!
//! A conversation the caller does not own answers like a missing one.

use axum::Json;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use demi_backend_database::conversation_index::{ConversationRecord, Creation};
use demi_backend_database::conversation_attachments::{self, AttachmentNumber, AttachmentRow};
use demi_backend_database::tree;
use demi_backend_page_sync::Part;
use demi_host_interface::{FileKind, FileStat, HostFs};
use demi_web_api_protocol::conversations::{
    BatchAnswer, BatchResult, ConversationBatch, ConversationPatch, CreatedConversation,
    ConversationUpdate, Conversations, ConversationsQuery, CreateConversation, FieldResult,
    ForkAnswer, ForkRequest, ReadRequest,
};
use demi_web_api_protocol::attachments::{AttachmentSourceDto, ConversationAttachment, ConversationAttachmentQuery};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, UserId};

use super::AppState;
use super::body::{JsonBody, page_socket};
use super::error::ApiError;
use super::gate::{AuthUser, CurrentSession};
use super::hosts;
use super::page_client::PageClient;
use super::query::QueryParams;
use demi_backend_user_shard::shard::page_socket::{PageSocketKind, PageSocketOrigin};
use demi_backend_user_shard::conversation::ForkRefusal;
use demi_backend_user_shard::services::Services;
use demi_backend_user_shard::sync::ChannelSession;

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::ConversationNotFound,
        "No such conversation",
    )
}

/// The caller's conversation the path names; one of another user answers
/// like a missing one, so a request only ever reaches its owner's shard.
pub(super) async fn owned(
    services: &Services,
    user: &UserId,
    id: &str,
) -> Result<ConversationRecord, ApiError> {
    let id = ConversationId::try_from(id).map_err(|_| not_found())?;
    services
        .control
        .conversation(id)
        .await?
        .filter(|record| record.owner == *user)
        .ok_or_else(not_found)
}

/// `GET /conversations?archived=`: the caller's conversations that are
/// archived, or that are not, in sidebar order.
pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    QueryParams(query): QueryParams<ConversationsQuery>,
) -> Result<Json<Conversations>, ApiError> {
    let archived = query.archived.0;
    let conversations = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.conversation_summaries(archived).await })
        .await??;
    Ok(Json(Conversations { conversations }))
}

/// `POST /conversations { id, ... }`: creates a conversation.
///
/// - A new conversation with what the request names answers 201.
/// - The caller's conversation of that id again applies nothing and answers
///   200.
/// - Another user's conversation holding the id, or a Fork reserving it,
///   answers 409 `id_unavailable`.
///
/// A part of the request that is refused answers its own status and code, and
/// nothing is created. Both successes answer the conversation.
pub(super) async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(request): JsonBody<CreateConversation>,
) -> Result<(StatusCode, Json<CreatedConversation>), ApiError> {
    let creation = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.create_conversation(request).await })
        .await?
        .map_err(hosts::refused)?;
    let (status, record) = match creation {
        Creation::Created(record) => (StatusCode::CREATED, record),
        Creation::Existing(record) => (StatusCode::OK, record),
        Creation::Unavailable => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                ErrorCode::IdUnavailable,
                "Conversation id is unavailable",
            ));
        }
        Creation::Unselectable(reason) => return Err(hosts::refused(reason.into())),
    };
    let conversation = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.conversation_summary(record).await })
        .await??;
    Ok((status, Json(CreatedConversation { conversation })))
}

/// `GET /conversations/:id/attachments/:attachment`: an attachment of the
/// conversation, by its number such as `a3`, from the conversation's database
/// alone; it wakes no Host.
///
/// With `?node=`, only one that node's replies show
/// (`conversation_attachments::read_shown`). A number the conversation does not
/// have, or those replies do not show, answers 404 `not_found`.
pub(super) async fn attachment(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, attachment)): Path<(String, String)>,
    QueryParams(query): QueryParams<ConversationAttachmentQuery>,
) -> Result<Json<ConversationAttachment>, ApiError> {
    let services = &state.services;
    let record = owned(services, &user.id, &id).await?;
    let no_attachment = || {
        ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            format!("No attachment {attachment} in this conversation"),
        )
    };
    let number: AttachmentNumber = attachment.parse().map_err(|_| no_attachment())?;
    let row = services
        .conversations
        .read(&record.id, move |connection| match &query.node {
            Some(node) => conversation_attachments::read_shown(connection, number, node),
            None => conversation_attachments::read(connection, number),
        })
        .await
        .map_err(ApiError::conversation_unreadable)?
        .flatten()
        .ok_or_else(no_attachment)?;
    Ok(Json(attachment_answer(row)))
}

/// `GET /conversations/:id/attachments/:attachment/file`: whether the file the
/// attachment was uploaded from is there to open in its copy's place
/// (`docs/decisions/0003-attachment-opens-its-file.md`), asked once each time the
/// page opens the attachment.
///
/// It answers 204 while the file is a regular file on the conversation's
/// primary Host, and that Host is the device the attachment was uploaded
/// from. It answers 404 `not_found` for an attachment the conversation does
/// not have or that names no file, for a file that is not there, and while
/// the conversation runs on another Host. It is a look, admitted as a user
/// stream is (`look_at_host_on`): a stopped Cloud answers 409 `host_stopped`
/// and is not woken, and an offline device answers as it does for any look.
pub(super) async fn attachment_file(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, attachment)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let services = &state.services;
    let record = owned(services, &user.id, &id).await?;
    let no_file = || {
        ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            format!("The file attachment {attachment} was uploaded from is not on the conversation's Host"),
        )
    };
    let number: AttachmentNumber = attachment.parse().map_err(|_| no_file())?;
    let source = services
        .conversations
        .read(&record.id, move |connection| conversation_attachments::read(connection, number))
        .await
        .map_err(ApiError::conversation_unreadable)?
        .flatten()
        .and_then(|row| row.source)
        .ok_or_else(no_file)?;
    let conversation = record.id;
    let present = state
        .shards
        .of(&user.id)
        .call(move |shard, cancel| async move {
            shard
                .host_shard()
                .look_at_host_on(&conversation, &source.host, &cancel, async |host| {
                    matches!(
                        host.host.stat(&source.path).await,
                        Ok(FileStat { kind: FileKind::File, .. })
                    )
                })
                .await
                .map_err(ApiError::from)
        })
        .await??;
    match present {
        Some(true) => Ok(StatusCode::NO_CONTENT),
        Some(false) | None => Err(no_file()),
    }
}

/// An attachment's row as an attachment route answers it.
pub(super) fn attachment_answer(row: AttachmentRow) -> ConversationAttachment {
    ConversationAttachment {
        id: row.number.to_string(),
        name: row.name,
        media_type: row.media_type,
        size: row.size,
        width: row.pixels.map(|pixels| pixels.width),
        height: row.pixels.map(|pixels| pixels.height),
        blob: row.blob,
        source: row.source.map(|source| AttachmentSourceDto {
            host: source.host,
            path: source.path,
        }),
    }
}

/// `PATCH /conversations/:id`: each field applied on its own, the archive
/// first. All applied answers 200 with the conversation and each field's
/// result; a patch of one field that is refused answers that field's status
/// and code; any other refusal among several fields answers 207.
pub(super) async fn patch(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(patch): JsonBody<ConversationPatch>,
) -> Result<(StatusCode, Json<ConversationUpdate>), ApiError> {
    let id = ConversationId::try_from(id).map_err(|_| not_found())?;
    let update = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.apply_patch(&id, patch).await })
        .await??
        .ok_or_else(not_found)?;
    let failures: Vec<&FieldResult> = update
        .results
        .iter()
        .filter(|result| matches!(result, FieldResult::Failed { .. }))
        .collect();
    if let (
        [
            FieldResult::Failed {
                code,
                message,
                http_status,
                ..
            },
        ],
        1,
    ) = (failures.as_slice(), update.results.len())
    {
        let status =
            StatusCode::from_u16(*http_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return Err(ApiError::new(status, *code, message.clone()));
    }
    let status = if failures.is_empty() {
        StatusCode::OK
    } else {
        StatusCode::MULTI_STATUS
    };
    Ok((status, Json(update)))
}

/// `DELETE /conversations/:id`: deletes the conversation, archived or not,
/// 204. Nothing refuses it: its work stops first. A conversation the caller
/// does not have, one whose deletion has begun included, answers 404.
pub(super) async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = ConversationId::try_from(id).map_err(|_| not_found())?;
    let deleted = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.delete_conversation(&id).await })
        .await??;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(not_found())
    }
}

/// `POST /conversations/batch { items }`: up to 100 patches, each answered
/// on its own, 207 always; a conversation the caller does not have is its
/// item's refusal.
pub(super) async fn batch(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(ConversationBatch { items }): JsonBody<ConversationBatch>,
) -> Result<(StatusCode, Json<BatchAnswer>), ApiError> {
    let results = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            let mut results = Vec::with_capacity(items.len());
            for item in items {
                let result = match shard.apply_patch(&item.id, item.patch).await? {
                    Some(ConversationUpdate {
                        conversation,
                        results,
                    }) => BatchResult::Updated {
                        id: item.id,
                        conversation: Box::new(conversation),
                        results,
                    },
                    None => BatchResult::Refused {
                        id: item.id,
                        code: ErrorCode::ConversationNotFound,
                        message: "No such conversation".into(),
                    },
                };
                results.push(result);
            }
            Ok::<_, demi_backend_database::StorageError>(results)
        })
        .await??;
    Ok((StatusCode::MULTI_STATUS, Json(BatchAnswer { results })))
}

/// `POST /conversations/:id/fork { id, blockId }`: the conversation `id`
/// with this conversation's history through the assistant text `blockId`,
/// 201; the same attempt again, 200.
pub(super) async fn fork(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(source): Path<String>,
    JsonBody(ForkRequest { id, block_id }): JsonBody<ForkRequest>,
) -> Result<(StatusCode, Json<ForkAnswer>), ApiError> {
    let source = ConversationId::try_from(source).map_err(|_| not_found())?;
    let (forked, conversation) = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move {
            let forked = shard.fork(source, id, block_id).await?;
            let conversation = shard.conversation_summary(forked.record.clone()).await?;
            Ok::<_, ForkRefusal>((forked, conversation))
        })
        .await?
        .map_err(fork_refused)?;
    let status = if forked.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(ForkAnswer { conversation })))
}

fn fork_refused(refusal: ForkRefusal) -> ApiError {
    let message = refusal.to_string();
    match refusal {
        ForkRefusal::SourceNotFound => not_found(),
        ForkRefusal::Conflict => {
            ApiError::new(StatusCode::CONFLICT, ErrorCode::ForkConflict, message)
        }
        ForkRefusal::Unavailable => {
            ApiError::new(StatusCode::CONFLICT, ErrorCode::IdUnavailable, message)
        }
        ForkRefusal::Target(_) => ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidForkTarget,
            message,
        ),
        ForkRefusal::Storage(error) => error.into(),
        ForkRefusal::Failed(_) => ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::InternalError,
            message,
        ),
    }
}

/// `POST /conversations/:id/read { revision }`: acknowledges the output the
/// page showed, 204. The read revision only moves forward; a revision
/// beyond the conversation's output answers 409 `invalid_revision`.
pub(super) async fn read(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(ReadRequest { revision }): JsonBody<ReadRequest>,
) -> Result<StatusCode, ApiError> {
    let services = &state.services;
    let record = owned(services, &user.id, &id).await?;
    let facts = services
        .conversations
        .read(&record.id, tree::summary)
        .await
        .map_err(ApiError::conversation_unreadable)?
        .unwrap_or(tree::SummaryFacts::EMPTY);
    if revision > facts.revision {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::InvalidRevision,
            "Cannot read beyond current output",
        ));
    }
    services
        .control
        .mark_conversation_read(record.id.clone(), revision)
        .await?;
    // Opening a run's conversation marks the run read.
    if services.control.read_automation_runs(record.id.clone()).await? {
        services.sync.mark(&user.id, Part::Automations);
    }
    services.sync.mark(&user.id, Part::Conversation(record.id));
    Ok(StatusCode::NO_CONTENT)
}

/// `WS /conversations/:id/stream`: the conversation's frames. The socket is
/// upgraded only for the caller's conversation that is not archived, and
/// then moves into the caller's shard, which serves it until it closes.
pub(super) async fn stream(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    session: CurrentSession,
    Path(id): Path<String>,
    client: PageClient,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    if record.archived {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::ConversationArchived,
            "Use the transcript for an archived conversation's history",
        ));
    }
    let upgrade = upgrade.map_err(|_| {
        ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            ErrorCode::UpgradeRequired,
            "The conversation stream is a WebSocket",
        )
    })?;
    let origin = PageSocketOrigin {
        conversation: Some(record.id.clone()),
        ..client.origin(PageSocketKind::Conversation, user.id.clone())
    };
    let session = ChannelSession {
        id: session.id,
        token: session.token,
        user: user.clone(),
        expires_at: session.expires_at,
    };
    Ok(page_socket(upgrade, move |socket| async move {
        let adopted = state
            .shards
            .of(&user.id)
            .adopt(move |shard| shard.serve_conversation_socket(record, socket, origin, session))
            .await;
        // A shard that is closing takes no socket: dropping it closes it.
        if adopted.is_err() {
            tracing::info!("a conversation socket arrived while the backend shuts down");
        }
    }))
}
