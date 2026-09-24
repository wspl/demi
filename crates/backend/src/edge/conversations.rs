//! `/api/conversations` (`web-api.md` § Conversation creation and Fork,
//! § Sidebar mutations, read state and page synchronization): creating a
//! conversation under the id the browser chose, listing the caller's
//! conversations, changing their fields one patch at a time or in a batch,
//! forking one, reading one's history as its database holds it,
//! acknowledging its output, and its socket, `WS /conversations/:id/stream`,
//! which moves into the caller's shard once upgraded. A conversation the
//! caller does not own answers like a missing one.

use axum::Json;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use demi_web_api::conversations::{
    BatchAnswer, BatchResult, ConversationAnswer, ConversationBatch, ConversationPatch, ConversationUpdate, Conversations,
    ConversationsQuery, CreateConversation, FieldResult, ForkAnswer, ForkRequest, ReadRequest, SubagentHistory,
    Transcript,
};
use demi_web_api::error::ErrorCode;
use demi_web_api::ids::{ConversationId, UserId};

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use super::query::QueryParams;
use crate::backend::Services;
use crate::conversation::{ForkRefusal, failure_facts};
use crate::storage::conversation_index::{ConversationRecord, Creation};
use crate::storage::tree;

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound, "No such conversation")
}

/// The caller's conversation the path names; one of another user answers
/// like a missing one, so a request only ever reaches its owner's shard.
pub(super) async fn owned(services: &Services, user: &UserId, id: &str) -> Result<ConversationRecord, ApiError> {
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

/// `POST /conversations { id }`: a new conversation on the Cloud, 201; the
/// caller's conversation of that id again, 200; 409 `id_unavailable` when
/// another user's conversation holds the id or a Fork reserved it.
pub(super) async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(CreateConversation { id }): JsonBody<CreateConversation>,
) -> Result<(StatusCode, Json<ConversationAnswer>), ApiError> {
    let creation = state.services.control.create_conversation(user.id.clone(), id).await?;
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
    };
    let conversation = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.conversation_summary(record).await })
        .await??;
    Ok((status, Json(ConversationAnswer { conversation })))
}

/// `GET /conversations/:id/transcript`: the history as the conversation's
/// database holds it, with the failure facts of its error blocks. It reads
/// without a live session and wakes nothing.
pub(super) async fn transcript(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Transcript>, ApiError> {
    let services = &state.services;
    let record = owned(services, &user.id, &id).await?;
    let history = services
        .conversations
        .read(&record.id, tree::history)
        .await?
        .unwrap_or_default();
    let failures = failure_facts(&services.assembly, &history.blocks).await;
    let mut subagents = Vec::with_capacity(history.subagents.len());
    for (node, blocks) in history.subagents {
        let Some(subagent) = node.job() else {
            unreachable!("a subagent's record names its parent");
        };
        let failures = failure_facts(&services.assembly, &blocks).await;
        subagents.push(SubagentHistory {
            subagent,
            blocks,
            failures,
        });
    }
    Ok(Json(Transcript {
        blocks: history.blocks,
        failures,
        subagents,
    }))
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
    if let ([FieldResult::Failed { code, message, http_status, .. }], 1) = (failures.as_slice(), update.results.len()) {
        let status = StatusCode::from_u16(*http_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return Err(ApiError::new(status, *code, message.clone()));
    }
    let status = if failures.is_empty() {
        StatusCode::OK
    } else {
        StatusCode::MULTI_STATUS
    };
    Ok((status, Json(update)))
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
                    Some(ConversationUpdate { conversation, results }) => BatchResult::Updated {
                        id: item.id,
                        conversation,
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
            Ok::<_, crate::storage::StorageError>(results)
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
    let status = if forked.created { StatusCode::CREATED } else { StatusCode::OK };
    Ok((
        status,
        Json(ForkAnswer {
            conversation,
            model: forked.model,
        }),
    ))
}

fn fork_refused(refusal: ForkRefusal) -> ApiError {
    let message = refusal.to_string();
    match refusal {
        ForkRefusal::SourceNotFound => not_found(),
        ForkRefusal::Conflict => ApiError::new(StatusCode::CONFLICT, ErrorCode::ForkConflict, message),
        ForkRefusal::Unavailable => ApiError::new(StatusCode::CONFLICT, ErrorCode::IdUnavailable, message),
        ForkRefusal::Target(_) => ApiError::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidForkTarget, message),
        ForkRefusal::Storage(error) => error.into(),
        ForkRefusal::Failed(_) => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError, message),
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
        .await?
        .unwrap_or(tree::SummaryFacts::EMPTY);
    if revision > facts.revision {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::InvalidRevision,
            "Cannot read beyond current output",
        ));
    }
    services.control.mark_conversation_read(record.id, revision).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `WS /conversations/:id/stream`: the conversation's frames. The socket is
/// upgraded only for the caller's conversation that is not archived, and
/// then moves into the caller's shard, which serves it until it closes.
pub(super) async fn stream(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
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
    Ok(upgrade.on_upgrade(move |socket| async move {
        let adopted = state
            .shards
            .of(&user.id)
            .adopt(move |shard| shard.serve_conversation_socket(record, socket))
            .await;
        // A shard that is closing takes no socket: dropping it closes it.
        if adopted.is_err() {
            tracing::info!("a conversation socket arrived while the backend shuts down");
        }
    }))
}
