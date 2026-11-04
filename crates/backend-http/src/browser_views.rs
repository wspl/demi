//! View links: one tab of a conversation's browser, shown to whoever holds
//! the link.
//!
//! - The integration's routes make, list and remove a conversation's links
//!   (`/v1/conversations/:conversation/browser/...`).
//! - The public routes under `/api/views/:token` take no session and ignore
//!   one: `GET` answers the view's tab, mode and expiry to its page
//!   (`/v/:token`), and `GET /stream` is the page's socket, which opens the
//!   browser's user stream within the view's scope and relays it as the
//!   work panel's lane is relayed (`streams.rs`).
//! - Every miss of a public route, an unknown token or a view that expired
//!   or was removed, answers the same 404 `view_not_found`, and the socket
//!   closes with `view_not_found`.
//! - A view's expiry or removal ends each page open on it, and so does its
//!   integration's browser turned off or the integration's deletion
//!   (`crates/backend-user-shard/src/open_views.rs`).

use axum::Json;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use demi_backend_database::accounts::TokenHash;
use demi_backend_database::browser_views::BrowserViewRow;
use demi_backend_user_shard::integrations::IntegrationRefusal;
use demi_backend_user_shard::shard::lane::{LaneEnd, LaneMessage};
use demi_command_package_browser_protocol::browser::TabId;
use demi_command_package_browser_protocol::live::{LiveInput, STREAM};
use demi_integration_api_protocol as api;
use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::IntegrationId;
use demi_web_api_protocol::error::ErrorCode;
use futures_util::{SinkExt, StreamExt};
use demi_web_api_protocol::views::ViewLink;

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::integration_edge::{authenticated, owned_conversation, refused};
use super::streams::{End, FromPage, ToPage, relay};

fn view_answer(row: BrowserViewRow, url: Option<String>) -> api::BrowserView {
    api::BrowserView {
        id: row.id,
        tab: row.tab,
        read_only: row.read_only,
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
    let expires_at = i64::try_from(request.expires_in)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1000))
        .and_then(|millis| now.as_millisecond().checked_add(millis))
        .and_then(|millis| Timestamp::from_millisecond(millis).ok())
        .ok_or_else(|| refused(IntegrationRefusal::Invalid("`expiresIn` is beyond the times Demi keeps".into())))?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let row = BrowserViewRow {
        id: uuid::Uuid::new_v4().to_string(),
        conversation: conversation.id,
        tab: tab.to_string(),
        read_only: request.read_only,
        created_at: now,
        expires_at,
    };
    if !state.services.control.insert_browser_view(TokenHash::of(&token), row.clone()).await? {
        return Err(refused(IntegrationRefusal::NotFound("conversation")));
    }
    let url = state
        .services
        .public_url
        .get()
        .and_then(|public| public.url().join(&format!("v/{token}")).ok())
        .map(|url| url.to_string());
    Ok((StatusCode::CREATED, Json(view_answer(row, url))))
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
        .browser_views(conversation.id, state.services.clock.now())
        .await?;
    Ok(Json(api::BrowserViewList {
        views: views.into_iter().map(|row| view_answer(row, None)).collect(),
    }))
}

/// `DELETE /v1/conversations/:conversation/browser/views/:view`: ends each
/// page open on it.
pub(super) async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, view)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let record = authenticated(&state, &headers).await?;
    let conversation = owned_conversation(&state, record, conversation).await?;
    if !state.services.control.delete_browser_view(conversation.id, view.clone()).await? {
        return Err(refused(IntegrationRefusal::NotFound("view")));
    }
    state.services.views.remove(&view);
    Ok(StatusCode::NO_CONTENT)
}

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::ViewNotFound, "No view link answers that token")
}

/// The view of `token` and its integration, which still has its browser on.
async fn found(state: &AppState, token: &str) -> Result<(BrowserViewRow, IntegrationId), ApiError> {
    let services = &state.services;
    let view = services
        .control
        .browser_view(TokenHash::of(token), services.clock.now())
        .await?
        .ok_or_else(not_found)?;
    let integration = services
        .control
        .integration_conversation(view.conversation.clone())
        .await?
        .ok_or_else(not_found)?;
    let integration = services.control.integration(integration.integration).await?.ok_or_else(not_found)?;
    if !integration.capabilities.browser {
        return Err(not_found());
    }
    Ok((view, integration.id))
}

/// `GET /api/views/:token`.
pub(super) async fn read(State(state): State<AppState>, Path(token): Path<String>) -> Result<Json<ViewLink>, ApiError> {
    let (view, _) = found(&state, &token).await?;
    Ok(Json(ViewLink {
        tab: view.tab,
        read_only: view.read_only,
        expires_at: view.expires_at,
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
    let Ok((view, integration)) = found(&state, &token).await else {
        let _ = sink.send(close(4004, "view_not_found")).await;
        return;
    };
    // Registered, then read again: a removal the second read misses
    // cancels this page.
    let open = state.services.views.open(&view.id, integration);
    if found(&state, &token).await.is_err() {
        let _ = sink.send(close(4004, "view_not_found")).await;
        return;
    }
    let Some(binding) = state.services.user_streams.get(STREAM).cloned() else {
        let _ = sink.send(close(4004, "view_not_found")).await;
        return;
    };
    let Ok(tab) = TabId::try_from(view.tab.clone()) else {
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
    let Ok(Some(owner)) = state.services.control.conversation(view.conversation.clone()).await.map(|record| record.map(|record| record.owner)) else {
        let _ = sink.send(close(4004, "view_not_found")).await;
        return;
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
    let left = view.expires_at.to_jiff().duration_since(state.services.clock.now().to_jiff());
    let left = std::time::Duration::try_from(left).unwrap_or_default();
    let removed = open.removed.clone();
    let withdrawn = open.withdrawn.clone();
    let until = async move {
        tokio::select! {
            () = tokio::time::sleep(left) => End::ViewExpired,
            () = removed.cancelled() => End::ViewRemoved,
            () = withdrawn.cancelled() => End::ViewNotFound,
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
