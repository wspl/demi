//! A conversation's history as its database holds it.
//!
//! The routes read, each of one agent, the root, a subagent or a side question,
//! named by its node:
//!
//! - one page of its transcript at a time, in light form, with the failure
//!   facts of its error blocks and the attachments its replies name, and its
//!   latest page, read from the window of its live session when it has one,
//!   with the transcript's version and the view of its live commands;
//! - a block whole;
//!
//! and where the call that started a command lies.
//!
//! No route wakes a Host, and none needs a live session: a node that does not
//! run is read from its database.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::StorageError;
use demi_backend_database::tree::{self, BlockRead, Entry, PageQuery, PageRead, Transcript};
use demi_backend_user_shard::conversation::{attachment_facts, failure_facts, reply_blocks};
use demi_conversation_lane_protocol::{PAGE_BLOCKS, STORED_EPOCH, Version, index_u32, light};
use demi_shared_types::{Block, BlockId, CommandId, NodeId};
use demi_web_api_protocol::conversations::{CommandCall, TranscriptPage, TranscriptQuery, WholeBlock};
use demi_web_api_protocol::error::ErrorCode;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::query::StrictBool;

use super::AppState;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;
use super::query::QueryParams;

/// `GET /conversations/:id/agents/:node/transcript`: one page of an agent's
/// transcript. The latest page of a node a live session runs is read from its
/// window, at the session's version; any other is read from the store, at the
/// version the store holds, and no read restores a tree.
pub(super) async fn transcript(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, node)): Path<(String, String)>,
    QueryParams(query): QueryParams<TranscriptQuery>,
) -> Result<Json<TranscriptPage>, ApiError> {
    let node = NodeId::try_from(node).map_err(|_| no_node())?;
    let record = owned(&state.services, &user.id, &id).await?;
    read_transcript(&state, &user.id, record, node, query).await.map(Json)
}

/// The latest page of the transcript of the agent `node` of `user`'s
/// conversation `record`, as the web app's latest read answers it: what an
/// integration's transcript route answers too
/// (`crates/backend-http/src/integration_edge.rs`), which checked whose the
/// conversation is.
pub(super) async fn latest_page(
    state: &AppState,
    user: &UserId,
    record: ConversationRecord,
    node: NodeId,
) -> Result<TranscriptPage, ApiError> {
    let query = TranscriptQuery {
        latest: StrictBool(true),
        ..TranscriptQuery::default()
    };
    read_transcript(state, user, record, node, query).await
}

/// A page of the transcript of the agent `node` of `user`'s conversation
/// `record`, as `query` asks.
async fn read_transcript(
    state: &AppState,
    user: &UserId,
    record: ConversationRecord,
    node: NodeId,
    query: TranscriptQuery,
) -> Result<TranscriptPage, ApiError> {
    let services = &state.services;
    let latest = query.latest.0;
    let count = match query.blocks.map(usize::try_from) {
        None => PAGE_BLOCKS,
        Some(Ok(count)) if (1..=PAGE_BLOCKS).contains(&count) => count,
        Some(_) => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                ErrorCode::InvalidQuery,
                format!("`blocks` takes 1 to {PAGE_BLOCKS}"),
            ));
        }
    };
    let live = if latest {
        let conversation = record.id.clone();
        let wanted = node.clone();
        state
            .shards
            .of(user)
            .call(move |shard, _| async move {
                shard
                    .live_transcript(&conversation, &wanted, count)
                    .await
                    .map_err(ApiError::conversation_unreadable)
            })
            .await??
    } else {
        None
    };
    let source = if live.is_some() { "live" } else { "store" };
    let mut page = match live {
        Some(page) => page,
        None => {
            // The root is the conversation's from its start: before its first
            // turn stored it, its transcript is empty.
            let root = node.as_str() == record.id.as_str();
            let latest = query.latest.0;
            let transcript = Transcript::Agent(node.clone());
            let saved_node = node.clone();
            let (read, saved) = services
                .conversations
                .read(&record.id, move |connection| {
                    let read = tree::read_page(connection, &transcript, PageQuery::from(&query))?;
                    let saved = tree::saved_versions(connection)?.nodes.get(&saved_node).copied();
                    Ok((read, saved))
                })
                .await
                .map_err(ApiError::conversation_unreadable)?
                .map_or((None, None), |(read, saved)| (Some(read), saved));
            let read = read.filter(|read| !(root && matches!(read, PageRead::NoNode)));
            let whole_last = read.as_ref().and_then(|read| match read {
                PageRead::Page { blocks, .. } if latest => blocks.last().and_then(|entry| match entry {
                    Entry::Block(block) => Some(block.clone()),
                    Entry::StepsNotShared(_) => None,
                }),
                _ => None,
            });
            let mut page = page_answer(read, no_node, no_block, outside, changed)?;
            // The latest page's last block comes whole, as a live window's
            // does, since the page holds the end of the transcript.
            if let (Some(last), Some(slot)) = (whole_last, page.blocks.last_mut()) {
                *slot = last;
            }
            // What the store holds changes with every save of the node, and
            // its version with it, so a page follows what it read only while
            // the store still holds it.
            page.version = Some(stored_version(saved.unwrap_or(0)));
            // A closed child's shells that still stop are in no stored page.
            if latest {
                let conversation = record.id.clone();
                let wanted = node.clone();
                let (commands, cache_chart) = state
                    .shards
                    .of(user)
                    .call(move |shard, _| async move {
                        let commands = shard.stopping_commands(&conversation, &wanted);
                        let chart = shard.cache_chart(&conversation, &wanted).await;
                        (commands, chart)
                    })
                    .await?;
                page.commands = commands;
                page.cache_chart = cache_chart.map_err(|error| {
                    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::ConversationUnreadable, error.to_string())
                })?;
            }
            page
        }
    };
    // Error blocks come whole, so their facts read as the stream's.
    page.failures = failure_facts(&services.vault, &services.provider_registry, user, &page.blocks).await;
    page.attachments = attachment_facts(&services.conversations, &record.id, reply_blocks(&page.blocks))
        .await
        .map_err(ApiError::conversation_unreadable)?;
    tracing::info!(
        event = "transcript.read",
        conversation = %record.id,
        node = %node,
        latest,
        source,
        blocks = page.blocks.len(),
        revision = page.version.as_ref().map(|version| version.revision),
        "a page of a transcript was read"
    );
    Ok(page)
}

/// The version of a transcript read from the store: an epoch no live
/// session has, so a patch of a node that runs is never taken as its next,
/// at the revision the store holds it at.
fn stored_version(revision: u64) -> Version {
    Version {
        epoch: STORED_EPOCH.to_owned(),
        revision,
    }
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

/// A page read as its route answers it, without failure facts or
/// attachments.
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
            attachments: Vec::new(),
            version: None,
            commands: Vec::new(),
            request_output: None,
            cache_chart: None,
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
                attachments: Vec::new(),
                version: None,
                commands: Vec::new(),
                request_output: None,
                cache_chart: None,
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
    let failures = failure_facts(&services.vault, &services.provider_registry, &user.id, std::slice::from_ref(&found)).await;
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
