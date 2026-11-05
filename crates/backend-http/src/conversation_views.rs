//! A view link's page of a conversation (`/v/:token` of a link that names no
//! tab): the routes it reads the conversation by, under `/api/views/:token`,
//! and its page connection.
//!
//! Each route finds the link, then answers as the conversation's own route
//! answers its owner, by the same functions:
//!
//! - `GET /conversation`: the conversation with its agents
//!   (`conversations.rs`).
//! - `GET /agents/:node/transcript` and `GET /agents/:node/blocks/:block`
//!   (`history.rs`).
//! - `GET /cost/recent` (`cost.rs`).
//! - `GET /blobs/:sha256`: a blob the conversation's transcript or its
//!   attachments show.
//! - `GET /plugins/:plugin/blocks/:tag/:id`: a reply block's document,
//!   through a link of the block alone (`blocks.rs`).
//! - `WS /connect`: a page connection whose only lane is the conversation's,
//!   which watches (`connection.rs`); it ends as the link does.
//!
//! The page reads nothing else of the account's: no draft, saved messages,
//! workflows, shares, Host or settings. Every miss answers the same 404
//! `view_not_found`, as a tab's link's routes do (`view_links.rs`).

use axum::Json;
use axum::extract::ws::WebSocketUpgrade;
use axum::response::IntoResponse;
use axum::extract::{Path, Query, RawQuery, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::Extension;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_database::references::{conversation_shows_blob, conversation_shows_block};
use demi_backend_user_shard::conversation::LaneAccess;
use demi_shared_types::{BlobRef, BlockId, NodeId};
use demi_web_api_protocol::connection::LaneTarget;
use demi_backend_user_shard::conversation::runs_on;
use demi_web_api_protocol::conversations::{RecentCost, TranscriptPage, TranscriptQuery, WholeBlock};
use demi_web_api_protocol::views::ViewedConversation;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use axum::http::StatusCode;

use super::AppState;
use super::blobs::blob_answer;
use super::connection::{Closing, Ending, Opened, Principal, ViewGrant, upgraded};
use super::cost::RecentQuery;
use super::error::ApiError;
use super::page_client::PageClient;
use super::query::QueryParams;
use super::view_links::{Found, ended, found, not_found};
use super::{conversations, history};

/// A link to a conversation that opens, and the conversation.
pub(super) struct Viewed {
    pub(super) found: Found,
    pub(super) record: ConversationRecord,
}

/// The link of `token` to a conversation, which opens, and its conversation.
pub(super) async fn viewed(state: &AppState, token: &str) -> Result<Viewed, ApiError> {
    let found = found(state, token).await?;
    if found.link.tab.is_some() {
        return Err(not_found());
    }
    let record = state
        .services
        .control
        .conversation(found.link.conversation.clone())
        .await?
        .filter(|record| record.owner == found.owner)
        .ok_or_else(not_found)?;
    Ok(Viewed { found, record })
}

/// `GET /api/views/:token/conversation`.
pub(super) async fn detail(State(state): State<AppState>, Path(token): Path<String>) -> Result<Json<ViewedConversation>, ApiError> {
    let viewed = viewed(&state, &token).await?;
    let owner = viewed.found.owner;
    let model = viewed.record.model.clone();
    let conversation = conversations::conversation_detail(&state, &owner, viewed.record).await?;
    let runs_on = match model {
        Some(model) => runs_on(&state.services, &owner, &[model]).await.pop(),
        None => None,
    };
    Ok(Json(ViewedConversation { conversation, runs_on }))
}

/// `GET /api/views/:token/agents/:node/transcript`.
pub(super) async fn transcript(
    State(state): State<AppState>,
    Path((token, node)): Path<(String, String)>,
    QueryParams(query): QueryParams<TranscriptQuery>,
) -> Result<Json<TranscriptPage>, ApiError> {
    let viewed = viewed(&state, &token).await?;
    let node = NodeId::try_from(node).map_err(|_| not_found())?;
    history::read_transcript(&state, &viewed.found.owner, viewed.record, node, query).await.map(Json)
}

/// `GET /api/views/:token/agents/:node/blocks/:block`.
pub(super) async fn block(
    State(state): State<AppState>,
    Path((token, node, block)): Path<(String, String, String)>,
) -> Result<Json<WholeBlock>, ApiError> {
    let viewed = viewed(&state, &token).await?;
    let node = NodeId::try_from(node).map_err(|_| not_found())?;
    let block = BlockId::try_from(block).map_err(|_| not_found())?;
    history::whole_block(&state, &viewed.record, node, block).await.map(Json)
}

/// `GET /api/views/:token/cost/recent`.
pub(super) async fn recent(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Query(query): Query<RecentQuery>,
) -> Result<Json<RecentCost>, ApiError> {
    let viewed = viewed(&state, &token).await?;
    let agents = state.services.control.conversation_recent(viewed.record.id, query.window).await?;
    Ok(Json(RecentCost { agents }))
}

/// `GET /api/views/:token/blobs/:sha256?type=`: a blob the conversation's
/// transcript or its attachments show.
pub(super) async fn blob(
    State(state): State<AppState>,
    Path((token, name)): Path<(String, String)>,
    RawQuery(query): RawQuery,
    request: HeaderMap,
) -> Result<Response, ApiError> {
    let viewed = viewed(&state, &token).await?;
    let blob = BlobRef::try_from(name).map_err(|_| not_found())?;
    let wanted = blob.clone();
    let shown = state
        .services
        .conversations
        .read(&viewed.record.id, move |connection| conversation_shows_blob(connection, &wanted))
        .await
        .map_err(ApiError::conversation_unreadable)?
        .unwrap_or(false);
    if !shown {
        return Err(not_found());
    }
    let bytes = state
        .services
        .blobs
        .for_user(&viewed.found.owner)
        .get(&blob)
        .await?
        .ok_or_else(not_found)?;
    Ok(blob_answer(&bytes, query.as_deref(), &request))
}

/// The document of `plugin`'s reply block `id` of `tag`, as the owner of the
/// conversation the link `token` shows has it, for a block a text of the
/// conversation names; none for a block with no document, and anything else
/// is not found.
pub(super) async fn block_document(
    state: &AppState,
    token: &str,
    plugin: String,
    tag: String,
    id: String,
) -> Result<Option<String>, ApiError> {
    let viewed = viewed(state, token).await?;
    let (named_tag, named_id) = (tag.clone(), id.clone());
    let shown = state
        .services
        .conversations
        .read(&viewed.record.id, move |connection| conversation_shows_block(connection, &named_tag, &named_id))
        .await
        .map_err(ApiError::conversation_unreadable)?
        .unwrap_or(false);
    if !shown {
        return Err(not_found());
    }
    state
        .shards
        .of(&viewed.found.owner)
        .call(move |shard, cancel| async move { shard.plugins().reply_block_document(&plugin, &tag, id, cancel).await })
        .await?
        .map_err(super::plugins::refused)
}

/// `WS /api/views/:token/connect`: the page's connection. A miss closes it
/// at once with `view_not_found`, as the page cannot read an upgrade's
/// refusal.
pub(super) async fn connect(
    State(state): State<AppState>,
    Extension(Closing(closing)): Extension<Closing>,
    Path(token): Path<String>,
    client: PageClient,
    upgrade: WebSocketUpgrade,
) -> Response {
    // A link that no longer opens refuses the upgrade, as an ended session's
    // does: the page learns it from the failed open and reads the link again.
    let viewed = match viewed(&state, &token).await {
        Ok(viewed) => viewed,
        Err(error) => return error.into_response(),
    };
    let Found { link, integration, owner } = viewed.found;
    // Registered before the open of its lane finds the link again, so a
    // removal that open misses ends the connection.
    let open = state.services.views.open(&link.id, integration, link.conversation.clone(), false);
    let until = ended(&state, &link, &open);
    let ended = Box::pin(async move {
        let end = until.await;
        drop(open);
        Ending::View(end)
    });
    let grant = ViewGrant {
        token,
        conversation: link.conversation,
        owner,
        ended,
    };
    upgraded(upgrade, state, closing, client, Principal::View(grant))
}

/// Checks the open of a lane of `target` on a view link's connection: the
/// lane of its conversation alone, while the link `token` still opens.
pub(super) async fn check_open(
    state: &AppState,
    token: &str,
    conversation: &ConversationId,
    owner: &UserId,
    target: LaneTarget,
) -> Result<Opened, ApiError> {
    let LaneTarget::Conversation { conversation: named } = target else {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            ErrorCode::ViewOnly,
            "A view link's page opens its conversation's lane alone",
        ));
    };
    if named != conversation.as_str() {
        return Err(not_found());
    }
    let viewed = viewed(state, token).await?;
    conversations::lane_of(state, owner, viewed.record, LaneAccess::Watch)
}
