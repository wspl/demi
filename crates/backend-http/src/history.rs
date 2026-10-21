//! A conversation's history as its database holds it.
//!
//! The routes read, each of one agent, the root, a subagent or a side question,
//! named by its node (`docs/decisions/0009-agent-sessions-and-sync.md`):
//!
//! - one page of its transcript at a time, in light form;
//! - a block whole;
//!
//! and where the call that started a command lies.
//!
//! Every route reads a database alone: it wakes no Host and needs no live
//! session.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::StorageError;
use demi_backend_database::tree::{self, BlockRead, Entry, PageQuery, PageRead, Transcript};
use demi_backend_user_shard::conversation::failure_facts;
use demi_conversation_socket_protocol::{index_u32, light};
use demi_shared_types::{Block, BlockId, CommandId, NodeId};
use demi_web_api_protocol::conversations::{CommandCall, TranscriptPage, TranscriptQuery, WholeBlock};
use demi_web_api_protocol::error::ErrorCode;

use super::AppState;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;
use super::query::QueryParams;

/// `GET /conversations/:id/agents/:node/transcript`: one page of an agent's
/// transcript.
pub(super) async fn transcript(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, node)): Path<(String, String)>,
    QueryParams(query): QueryParams<TranscriptQuery>,
) -> Result<Json<TranscriptPage>, ApiError> {
    let services = &state.services;
    let record = owned(services, &user.id, &id).await?;
    let node = NodeId::try_from(node).map_err(|_| no_node())?;
    // The root is the conversation's from its start: before its first turn
    // stored it, its transcript is empty.
    let root = node.as_str() == record.id.as_str();
    let transcript = Transcript::Agent(node);
    let read = services
        .conversations
        .read(&record.id, move |connection| {
            tree::read_page(connection, &transcript, PageQuery::from(&query))
        })
        .await
        .map_err(ApiError::conversation_unreadable)?
        .filter(|read| !(root && matches!(read, PageRead::NoNode)));
    let mut page = page_answer(read, no_node, no_block, outside, changed)?;
    // Error blocks come whole, so their facts read as the stream's.
    page.failures = failure_facts(&services.assembly, &user.id, &page.blocks).await;
    Ok(Json(page))
}

/// A conversation's answer to an index outside its transcript.
fn outside() -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery, "an index outside the transcript")
}

/// A conversation's answer to an edge that is no longer at its index.
fn changed() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        ErrorCode::TranscriptChanged,
        "The transcript changed since the page read it",
    )
}

/// A page read as its route answers it, without failure facts.
///
/// - A conversation without a database yet has an empty transcript.
/// - A node or a block the transcript lacks, an index it does not reach, or an
///   edge no longer at its index, answers `no_node`, `no_block`, `outside` or
///   `changed`.
pub(super) fn page_answer(
    read: Option<PageRead>,
    no_node: fn() -> ApiError,
    no_block: fn() -> ApiError,
    outside: fn() -> ApiError,
    changed: fn() -> ApiError,
) -> Result<TranscriptPage, ApiError> {
    match read {
        None => Ok(TranscriptPage {
            start: 0,
            length: 0,
            blocks: Vec::new(),
            failures: None,
            instructions: Vec::new(),
            summaries: Vec::new(),
            versions: Vec::new(),
        }),
        Some(PageRead::Page {
            start,
            length,
            blocks,
            instructions,
            summaries,
            versions,
        }) => {
            let blocks = blocks
                .into_iter()
                .map(|entry| match entry {
                    Entry::Block(block) => Ok(block),
                    Entry::StepsNotShared(_) => Err(ApiError::conversation_unreadable(StorageError::Corrupt {
                        table: "blocks",
                        column: "block",
                        reason: "an agent's transcript holds a run of steps not shared".to_owned(),
                    })),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TranscriptPage {
                start: index_u32(start),
                length: index_u32(length),
                blocks: light(&blocks),
                failures: None,
                instructions,
                summaries,
                versions,
            })
        }
        Some(PageRead::NoNode) => Err(no_node()),
        Some(PageRead::NoBlock) => Err(no_block()),
        Some(PageRead::Outside) => Err(outside()),
        Some(PageRead::Changed) => Err(changed()),
        Some(PageRead::Invalid(message)) => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidQuery,
            message,
        )),
    }
}

/// `GET /conversations/:id/agents/:node/blocks/:blockId`: a block whole, for
/// a row the page opens.
pub(super) async fn block(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, node, block)): Path<(String, String, String)>,
) -> Result<Json<WholeBlock>, ApiError> {
    let services = &state.services;
    let record = owned(services, &user.id, &id).await?;
    let block = BlockId::try_from(block).map_err(|_| no_block())?;
    let transcript = Transcript::Agent(NodeId::try_from(node).map_err(|_| no_node())?);
    let read = services
        .conversations
        .read(&record.id, move |connection| {
            tree::read_block(connection, &transcript, &block)
        })
        .await
        .map_err(ApiError::conversation_unreadable)?;
    let found = block_answer(read, no_node, no_block)?;
    let failures = failure_facts(&services.assembly, &user.id, std::slice::from_ref(&found)).await;
    Ok(Json(WholeBlock {
        block: found,
        failures,
    }))
}

/// A block read as its route answers it: a conversation without a database
/// yet holds no block.
pub(super) fn block_answer(
    read: Option<BlockRead>,
    no_node: fn() -> ApiError,
    no_block: fn() -> ApiError,
) -> Result<Block, ApiError> {
    match read {
        Some(BlockRead::Block(block)) => Ok(block),
        Some(BlockRead::NoNode) => Err(no_node()),
        Some(BlockRead::NoBlock) | None => Err(no_block()),
    }
}

/// `GET /conversations/:id/commands/:commandId`: where the call that started
/// a command lies.
pub(super) async fn command(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, command)): Path<(String, String)>,
) -> Result<Json<CommandCall>, ApiError> {
    let services = &state.services;
    let record = owned(services, &user.id, &id).await?;
    let no_command = || {
        ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            format!("No command {command} in this conversation"),
        )
    };
    let command_id = CommandId::try_from(command.clone()).map_err(|_| no_command())?;
    let wanted = command_id.clone();
    let (node, call) = services
        .conversations
        .read(&record.id, move |connection| tree::command_call(connection, &wanted))
        .await
        .map_err(ApiError::conversation_unreadable)?
        .flatten()
        .ok_or_else(no_command)?;
    Ok(Json(CommandCall {
        command_id,
        node,
        block_id: call.id,
    }))
}

fn no_node() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
        "No such agent in this conversation",
    )
}

fn no_block() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::BlockNotFound,
        "The transcript holds no such block",
    )
}
