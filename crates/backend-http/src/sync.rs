//! The page's synchronization channel, a lane of its connection, which the
//! user's shard serves until it ends; and `WS /api/sync`, where a page of a
//! build before the connection opened its channel.
//!
//! The old route tells every page that the product state it holds is stale,
//! and closes: that build reads the state again, which names the web build
//! the backend serves, and reloads on it. No page of that build is served
//! otherwise.

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocketUpgrade, close_code};
use axum::http::StatusCode;
use axum::response::Response;
use demi_backend_user_shard::shard::lane::PageSession;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::state::{SyncEvent, SyncFollowed};

use super::AppState;
use super::connection::Opened;
use super::error::ApiError;

/// The synchronization channel of the session `session`.
pub(super) fn open(state: &AppState, session: PageSession) -> Opened {
    let shards = state.shards.clone();
    Opened::new(move |lane, reader| async move {
        let user = session.user.id.clone();
        // A shard that is closing takes no channel: the lane drops, and the
        // connection closes as the backend does.
        let _ = shards
            .of(&user)
            .adopt(move |shard| shard.serve_sync_channel(lane, reader, session))
            .await;
    })
}

/// `WS /api/sync`: the word that sends a page of an earlier build to read the
/// state, on which it reloads.
pub(super) async fn legacy(
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let upgrade = upgrade.map_err(|_| {
        ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            ErrorCode::UpgradeRequired,
            "The synchronization channel is a WebSocket",
        )
    })?;
    let stale = serde_json::to_string(&SyncEvent::Stale {
        value: SyncFollowed::State,
    })
    .expect("a sync event serializes");
    Ok(upgrade.on_upgrade(move |mut socket| async move {
        // A page gone meanwhile hears nothing.
        if socket.send(Message::Text(stale.into())).await.is_ok() {
            let frame = CloseFrame {
                code: close_code::NORMAL,
                reason: Utf8Bytes::from_static("superseded"),
            };
            let _ = socket.send(Message::Close(Some(frame))).await;
        }
    }))
}
