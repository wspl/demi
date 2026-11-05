//! View links: a tab of a conversation's browser, or the conversation
//! itself, shown to whoever holds the link.
//!
//! - The integration's routes make, list and remove a conversation's links,
//!   of its browser's tabs (`/v1/conversations/:conversation/browser/...`)
//!   and of the conversation (`/v1/conversations/:conversation/views`).
//! - The public routes under `/api/views/:token` take no session and ignore
//!   one: `GET` answers what the link shows to its page (`/v/:token`).
//!   `GET /stream` is a tab's page's socket, which opens the browser's user
//!   stream within the link's scope and relays it as the work panel's lane
//!   is relayed (`streams.rs`). The conversation's page reads and follows
//!   the conversation by its own routes (`conversation_views.rs`).
//! - Every miss of a public route, an unknown token or a link that expired
//!   or was removed, answers the same 404 `view_not_found`, and a socket
//!   closes with `view_not_found`.
//! - A link's expiry or removal ends each page open on it, and so does its
//!   conversation's deletion, a tab's integration's browser turned off, or
//!   the integration's deletion
//!   (`crates/backend-user-shard/src/open_views.rs`).

use axum::Json;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use demi_backend_database::accounts::TokenHash;
use demi_backend_database::view_links::ViewLinkRow;
use demi_backend_user_shard::integrations::IntegrationRefusal;
use demi_backend_user_shard::open_views::OpenView;
use demi_backend_user_shard::shard::lane::{LaneEnd, LaneMessage};
use demi_command_package_browser_protocol::browser::TabId;
use demi_command_package_browser_protocol::live::{LiveInput, STREAM};
use demi_integration_api_protocol as api;
use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::{IntegrationId, UserId};
use demi_web_api_protocol::error::ErrorCode;
use futures_util::{SinkExt, StreamExt};
use demi_web_api_protocol::views::ViewLink;

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::integration_edge::{authenticated, owned_conversation, refused};
use super::streams::{End, FromPage, ToPage, relay};

fn tab_answer(row: ViewLinkRow, url: Option<String>) -> api::BrowserView {
    api::BrowserView {
        id: row.id,
        tab: row.tab.unwrap_or_default(),
        read_only: row.read_only,
        created_at: row.created_at,
        expires_at: row.expires_at,
        url,
    }
}

fn conversation_answer(row: ViewLinkRow, url: Option<String>) -> api::ConversationView {
    api::ConversationView {
        id: row.id,
        created_at: row.created_at,
        expires_at: row.expires_at,
        url,
    }
}

/// Refuses a request of an integration whose browser capability is off.
fn browser_on(capabilities: &demi_shared_types::IntegrationCapabilities) -> Result<(), ApiError> {
    if capabilities.browser {
        Ok(())
    } else {
        Err(refused(IntegrationRefusal::Refused("The integration's browser capability is off".into())))
    }
}

/// When a link made at `now` that opens for `seconds` expires; none for a
/// link that never does.
fn expiry(now: Timestamp, seconds: Option<u64>) -> Result<Option<Timestamp>, ApiError> {
    let Some(seconds) = seconds else {
        return Ok(None);
    };
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1000))
        .and_then(|millis| now.as_millisecond().checked_add(millis))
        .and_then(|millis| Timestamp::from_millisecond(millis).ok())
        .map(Some)
        .ok_or_else(|| refused(IntegrationRefusal::Invalid("`expiresIn` is beyond the times Demi keeps".into())))
}

/// Stores a new link `row` of its conversation, with its token, and answers
/// the URL of its page.
async fn store(state: &AppState, row: &ViewLinkRow) -> Result<Option<String>, ApiError> {
    let token = uuid::Uuid::new_v4().simple().to_string();
    if !state.services.control.insert_view_link(TokenHash::of(&token), row.clone()).await? {
        return Err(refused(IntegrationRefusal::NotFound("conversation")));
    }
    Ok(state
        .services
        .public_url
        .get()
        .and_then(|public| public.url().join(&format!("v/{token}")).ok())
        .map(|url| url.to_string()))
}

/// `POST /v1/conversations/:conversation/browser/tabs/:tab/views`: a link
/// to the tab, with its URL, which only this answer carries.
pub(super) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, tab)): Path<(String, String)>,
    JsonBody(request): JsonBody<api::CreateBrowserView>,
) -> Result<(StatusCode, Json<api::BrowserView>), ApiError> {
    let record = authenticated(&state, &headers).await?;
    browser_on(&record.capabilities)?;
    let conversation = owned_conversation(&state, record, conversation).await?;
    let tab = TabId::try_from(tab).map_err(|error| refused(IntegrationRefusal::Invalid(error.to_string())))?;
    let now = state.services.clock.now();
    let row = ViewLinkRow {
        id: uuid::Uuid::new_v4().to_string(),
        conversation: conversation.id,
        tab: Some(tab.to_string()),
        read_only: request.read_only,
        created_at: now,
        expires_at: expiry(now, request.expires_in)?,
    };
    let url = store(&state, &row).await?;
    Ok((StatusCode::CREATED, Json(tab_answer(row, url))))
}

/// `GET /v1/conversations/:conversation/browser/views`.
pub(super) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
) -> Result<Json<api::BrowserViewList>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let conversation = owned_conversation(&state, record, conversation).await?;
    let views = state
        .services
        .control
        .view_links(conversation.id, true, state.services.clock.now())
        .await?;
    Ok(Json(api::BrowserViewList {
        views: views.into_iter().map(|row| tab_answer(row, None)).collect(),
    }))
}

/// `DELETE /v1/conversations/:conversation/browser/views/:view`: ends each
/// page open on it.
pub(super) async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, view)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    removed(&state, &headers, conversation, view, true).await
}

/// `POST /v1/conversations/:conversation/views`: a link to the
/// conversation, with its URL, which only this answer carries.
pub(super) async fn create_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
    JsonBody(request): JsonBody<api::CreateConversationView>,
) -> Result<(StatusCode, Json<api::ConversationView>), ApiError> {
    let record = authenticated(&state, &headers).await?;
    let conversation = owned_conversation(&state, record, conversation).await?;
    let now = state.services.clock.now();
    let row = ViewLinkRow {
        id: uuid::Uuid::new_v4().to_string(),
        conversation: conversation.id,
        tab: None,
        read_only: true,
        created_at: now,
        expires_at: expiry(now, request.expires_in)?,
    };
    let url = store(&state, &row).await?;
    Ok((StatusCode::CREATED, Json(conversation_answer(row, url))))
}

/// `GET /v1/conversations/:conversation/views`.
pub(super) async fn list_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation): Path<String>,
) -> Result<Json<api::ConversationViewList>, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let conversation = owned_conversation(&state, record, conversation).await?;
    let views = state
        .services
        .control
        .view_links(conversation.id, false, state.services.clock.now())
        .await?;
    Ok(Json(api::ConversationViewList {
        views: views.into_iter().map(|row| conversation_answer(row, None)).collect(),
    }))
}

/// `DELETE /v1/conversations/:conversation/views/:view`: ends each page
/// open on it.
pub(super) async fn remove_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, view)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    removed(&state, &headers, conversation, view, false).await
}

/// Removes the conversation's link `view`, of its browser's tabs or of the
/// conversation as `tabs` says, and ends each page open on it.
async fn removed(state: &AppState, headers: &HeaderMap, conversation: String, view: String, tabs: bool) -> Result<StatusCode, ApiError> {
    let record = authenticated(state, headers).await?;
    let conversation = owned_conversation(state, record, conversation).await?;
    if !state.services.control.delete_view_link(conversation.id, tabs, view.clone()).await? {
        return Err(refused(IntegrationRefusal::NotFound("view")));
    }
    state.services.views.remove(&view);
    Ok(StatusCode::NO_CONTENT)
}

pub(super) fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::ViewNotFound, "No view link answers that token")
}

/// A link that opens, as its page found it.
pub(super) struct Found {
    pub(super) link: ViewLinkRow,
    pub(super) integration: IntegrationId,
    /// The conversation's owner, the integration's user.
    pub(super) owner: UserId,
}

/// The link of `token`, which opens: a tab's while its integration has its
/// browser on.
pub(super) async fn found(state: &AppState, token: &str) -> Result<Found, ApiError> {
    let services = &state.services;
    let link = services
        .control
        .view_link(TokenHash::of(token), services.clock.now())
        .await?
        .ok_or_else(not_found)?;
    let integration = services
        .control
        .integration_conversation(link.conversation.clone())
        .await?
        .ok_or_else(not_found)?;
    let integration = services.control.integration(integration.integration).await?.ok_or_else(not_found)?;
    if link.tab.is_some() && !integration.capabilities.browser {
        return Err(not_found());
    }
    Ok(Found {
        link,
        integration: integration.id,
        owner: integration.user,
    })
}

/// Why a page open on a view link ends, a tab's socket or a conversation's
/// page connection alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ViewEnd {
    /// The link expired.
    Expired,
    /// The link was removed, or its conversation deleted.
    Removed,
    /// The link no longer opens: its integration went.
    NotFound,
}

impl ViewEnd {
    /// The close the page hears.
    pub(super) fn close(self) -> (u16, &'static str) {
        match self {
            Self::Expired => (4002, "view_expired"),
            Self::Removed => (4003, "view_removed"),
            Self::NotFound => (4004, "view_not_found"),
        }
    }
}

/// Resolves as the page `open` holds on `link` ends, with why.
pub(super) fn ended(state: &AppState, link: &ViewLinkRow, open: &OpenView) -> impl Future<Output = ViewEnd> + Send + 'static {
    let expired = expired(state, link);
    let removed = open.removed.clone();
    let withdrawn = open.withdrawn.clone();
    async move {
        tokio::select! {
            () = expired => ViewEnd::Expired,
            () = removed.cancelled() => ViewEnd::Removed,
            () = withdrawn.cancelled() => ViewEnd::NotFound,
        }
    }
}

/// Resolves as `link` expires, for a page open on it; never for a link that
/// never does.
fn expired(state: &AppState, link: &ViewLinkRow) -> impl Future<Output = ()> + Send + 'static {
    let left = link.expires_at.map(|expires_at| {
        let left = expires_at.to_jiff().duration_since(state.services.clock.now().to_jiff());
        std::time::Duration::try_from(left).unwrap_or_default()
    });
    async move {
        match left {
            Some(left) => tokio::time::sleep(left).await,
            None => std::future::pending().await,
        }
    }
}

/// `GET /api/views/:token`.
pub(super) async fn read(State(state): State<AppState>, Path(token): Path<String>) -> Result<Json<ViewLink>, ApiError> {
    let found = found(&state, &token).await?;
    let link = found.link;
    Ok(Json(match link.tab {
        Some(tab) => ViewLink::Tab {
            tab,
            read_only: link.read_only,
            expires_at: link.expires_at,
        },
        None => ViewLink::Conversation {
            expires_at: link.expires_at,
        },
    }))
}

/// `GET /api/views/:token/stream`: the page's socket. A miss closes it at
/// once with `view_not_found`, as the page cannot read an upgrade's refusal.
pub(super) async fn stream(
    State(state): State<AppState>,
    Path(token): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |socket| serve(socket, state, token))
}

async fn serve(socket: WebSocket, state: AppState, token: String) {
    let (mut sink, source) = socket.split();
    let close = |code: u16, reason: &str| Message::Close(Some(CloseFrame { code, reason: reason.into() }));
    // A link to the conversation has no tab to stream.
    let Ok(Found { link: view, integration, owner }) = found(&state, &token).await else {
        let _ = sink.send(close(4004, "view_not_found")).await;
        return;
    };
    let Some(tab) = view.tab.clone() else {
        let _ = sink.send(close(4004, "view_not_found")).await;
        return;
    };
    // Registered, then read again: a removal the second read misses
    // cancels this page.
    let open = state.services.views.open(&view.id, integration, view.conversation.clone(), true);
    if found(&state, &token).await.is_err() {
        let _ = sink.send(close(4004, "view_not_found")).await;
        return;
    }
    let Some(binding) = state.services.user_streams.get(STREAM).cloned() else {
        let _ = sink.send(close(4004, "view_not_found")).await;
        return;
    };
    let Ok(tab) = TabId::try_from(tab) else {
        let _ = sink.send(close(4004, "view_not_found")).await;
        return;
    };
    let scope = LiveInput {
        tab: Some(tab),
        read_only: view.read_only,
    };
    let Ok(serde_json::Value::Object(args)) = serde_json::to_value(&scope) else {
        unreachable!("a view's scope serializes to an object");
    };
    let conversation = view.conversation.clone();
    let opened = state
        .shards
        .of(&owner)
        .call(move |shard, cancel| async move {
            let Some(plugin_off) = shard.plugins().stream_end(STREAM).await? else {
                return Ok(None);
            };
            let stream = shard.host_shard().open_user_stream(&conversation, &binding, &args, &cancel).await?;
            Ok::<_, ApiError>(Some((stream, plugin_off)))
        })
        .await;
    // A refusal's code is the close's reason, as a lane's refusal is the
    // work panel's: the page shows a stopped Cloud as the panel does, and
    // opens again.
    let opened = opened.map_err(ApiError::from).and_then(|opened| opened);
    let (stream, plugin_off) = match opened {
        Ok(Some(opened)) => opened,
        Ok(None) => {
            let _ = sink.send(close(4001, "plugin_disabled")).await;
            return;
        }
        Err(error) => {
            let (_, body) = error.into_body();
            let _ = sink.send(close(4005, &body.code.to_string())).await;
            return;
        }
    };
    let ended = ended(&state, &view, &open);
    let until = async move {
        tokio::select! {
            end = ended => End::View(end),
            () = plugin_off.cancelled() => End::PluginDisabled,
        }
    };
    relay(SocketOut(sink), SocketIn(source), stream, until, |SocketOut(mut sink), _, end| {
        if let LaneEnd::Close(frame) = end.close() {
            tokio::spawn(async move {
                let _ = sink.send(close(frame.code, frame.reason)).await;
            });
        }
    })
    .await;
    drop(open);
}

struct SocketOut(futures_util::stream::SplitSink<WebSocket, Message>);

impl ToPage for SocketOut {
    async fn send_bytes(&mut self, bytes: Bytes) -> Result<(), ()> {
        self.0.send(Message::Binary(bytes)).await.map_err(|_| ())
    }
}

struct SocketIn(futures_util::stream::SplitStream<WebSocket>);

impl FromPage for SocketIn {
    async fn next(&mut self) -> Option<LaneMessage> {
        loop {
            match self.0.next().await {
                Some(Ok(Message::Binary(bytes))) => return Some(LaneMessage::Binary(bytes)),
                Some(Ok(Message::Text(text))) => return Some(LaneMessage::Text(text.as_str().to_owned())),
                Some(Ok(Message::Close(_)) | Err(_)) | None => return None,
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            }
        }
    }
}
