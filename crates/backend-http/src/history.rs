//! A conversation's history as its database holds it (`web-api.md`
//! § Conversation history): one page of an agent's transcript at a time,
//! in light form, a block whole, the conversation's subagents, and the
//! record of an ended command. Every route reads the database alone: it
//! wakes no Host and needs no live session.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_database::tree;
use demi_backend_host_access::root_of;
use demi_backend_user_shard::conversation::failure_facts;
use demi_conversation_socket_protocol::{index_u32, light};
use demi_shared_types::{BlockId, CommandId};
use demi_web_api_protocol::conversations::{
    CommandRecord, NodeQuery, Subagents, TranscriptPage, TranscriptQuery, WholeBlock,
};
use demi_web_api_protocol::error::ErrorCode;

use super::AppState;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;
use super::query::QueryParams;

/// `GET /conversations/:id/transcript`: one page of an agent's transcript
/// (`web-api.md` § Pages).
pub(super) async fn transcript(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(query): QueryParams<TranscriptQuery>,
) -> Result<Json<TranscriptPage>, ApiError> {
    let services = &state.services;
    let record = owned(services, &user.id, &id).await?;
    let node = query.node.clone().unwrap_or_else(|| root_of(&record.id));
    let read = services
        .conversations
        .read(&record.id, move |connection| {
            tree::read_page(connection, &node, &query)
        })
        .await?;
    match read {
        // A conversation with no database yet has an empty transcript.
        None => Ok(Json(TranscriptPage {
            start: 0,
            length: 0,
            blocks: Vec::new(),
            failures: None,
            instructions: Vec::new(),
            summaries: Vec::new(),
        })),
        Some(tree::PageRead::Page {
            start,
            length,
            blocks,
            instructions,
            summaries,
        }) => {
            // Error blocks come whole, so their facts read as the stream's.
            let failures = failure_facts(&services.assembly, &blocks).await;
            Ok(Json(TranscriptPage {
                start: index_u32(start),
                length: index_u32(length),
                blocks: blocks.iter().map(light).collect(),
                failures,
                instructions,
                summaries,
            }))
        }
        Some(tree::PageRead::NoNode) => Err(no_node()),
        Some(tree::PageRead::NoBlock) => Err(no_block()),
        Some(tree::PageRead::Changed) => Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::TranscriptChanged,
            "The transcript changed since the page read it",
        )),
        Some(tree::PageRead::Invalid(message)) => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidQuery,
            message,
        )),
    }
}

/// `GET /conversations/:id/transcript/blocks/:blockId`: a block whole, for
/// a row the page opens (`web-api.md` § Light form).
pub(super) async fn block(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, block)): Path<(String, String)>,
    QueryParams(query): QueryParams<NodeQuery>,
) -> Result<Json<WholeBlock>, ApiError> {
    let services = &state.services;
    let record = owned(services, &user.id, &id).await?;
    let block = BlockId::try_from(block).map_err(|_| no_block())?;
    let node = query.node.unwrap_or_else(|| root_of(&record.id));
    let found = services
        .conversations
        .read(&record.id, move |connection| {
            let Some(length) = tree::transcript_length(connection, &node)? else {
                return Ok(Err(no_node()));
            };
            let Some(index) = tree::block_index(connection, &node, &block, length)? else {
                return Ok(Err(no_block()));
            };
            let mut blocks = tree::blocks_in(connection, &node, index..index + 1)?;
            Ok(Ok(blocks.remove(0)))
        })
        .await?
        .ok_or_else(no_block)??;
    let failures = failure_facts(&services.assembly, std::slice::from_ref(&found)).await;
    Ok(Json(WholeBlock {
        block: found,
        failures,
    }))
}

/// `GET /conversations/:id/subagents`: every subagent the conversation has
/// had, in the order they started.
pub(super) async fn subagents(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Subagents>, ApiError> {
    let services = &state.services;
    let record = owned(services, &user.id, &id).await?;
    let nodes = services
        .conversations
        .read(&record.id, tree::subagents)
        .await?
        .unwrap_or_default();
    Ok(Json(Subagents {
        subagents: nodes.iter().filter_map(|node| node.job()).collect(),
    }))
}

/// `GET /conversations/:id/commands/:commandId`: an ended command, from the
/// call that started it (`web-api.md` § Subagents and commands).
pub(super) async fn command(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, command)): Path<(String, String)>,
) -> Result<Json<CommandRecord>, ApiError> {
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
    let root = root_of(&record.id);
    let wanted = command_id.clone();
    let (node, call) = services
        .conversations
        .read(&record.id, move |connection| tree::command_call(connection, &wanted))
        .await?
        .flatten()
        .ok_or_else(no_command)?;
    // The input is the JSON the provider supplied for a call the session
    // ran, so it parses; one that did not would leave the title and script
    // empty, as the terminal then shows only the output.
    let input: serde_json::Value = serde_json::from_str(&call.input).unwrap_or_default();
    let text = |field: &str| {
        input
            .get(field)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Ok(Json(CommandRecord {
        command_id,
        subagent_id: (node != root).then_some(node),
        title: text("description"),
        script: text("script"),
        started_at: call.created_at,
        view: call.view,
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
