//! `WS /api/sync` (`web-api.md` § Page synchronization): the page's
//! synchronization channel. It requires the session cookie, which it checks
//! without renewing the session, since only requests renew it; once
//! upgraded, the socket moves into the user's shard, which serves it until
//! it closes.

use axum::extract::State;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::http::StatusCode;
use axum::response::Response;
use axum_extra::extract::CookieJar;
use demi_backend_database::accounts::TokenHash;
use demi_web_api_protocol::error::ErrorCode;

use super::AppState;
use super::body::page_socket;
use super::cookies::SESSION_COOKIE;
use super::error::ApiError;
use super::page_client::PageClient;
use demi_backend_user_shard::shard::page_socket::PageSocketKind;
use demi_backend_user_shard::sync::ChannelSession;

pub(super) async fn channel(
    State(state): State<AppState>,
    jar: CookieJar,
    client: PageClient,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let token = jar
        .get(SESSION_COOKIE)
        .map(|cookie| TokenHash::of(cookie.value()))
        .ok_or_else(ApiError::unauthenticated)?;
    let session = state
        .services
        .sessions
        .check(&token)
        .await?
        .ok_or_else(ApiError::unauthenticated)?;
    let upgrade = upgrade.map_err(|_| {
        ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            ErrorCode::UpgradeRequired,
            "The synchronization channel is a WebSocket",
        )
    })?;
    let session = ChannelSession {
        token,
        user: session.user,
        expires_at: session.expires_at,
    };
    Ok(page_socket(upgrade, move |socket| async move {
        let user = session.user.id.clone();
        let origin = client.origin(PageSocketKind::Sync, user.clone());
        let adopted = state
            .shards
            .of(&user)
            .adopt(move |shard| shard.serve_sync_channel(socket, session, origin))
            .await;
        // A shard that is closing takes no channel: dropping the socket
        // closes it, and the page connects again to the next backend.
        if adopted.is_err() {
            tracing::info!("a synchronization channel arrived while the backend shuts down");
        }
    }))
}
