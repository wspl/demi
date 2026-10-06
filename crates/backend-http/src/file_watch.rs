//! The file watch at the edge (`web-api.md` § File watch): the upgrade, and
//! the relay of the Host's reports to the page and of the page's `paths`
//! messages to the shard, which follows the Host's watches.

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use demi_backend_host_access::file_watch::{FileWatchChannel, OpenedWatch};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::files::{FileWatchMessage, FileWatchRequest, FileWatchState};
use futures_util::{SinkExt as _, StreamExt as _};
use garde::Validate as _;

use super::AppState;
use super::body::page_socket;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;

/// `WS /conversations/:id/fs/watch`: a conversation that is not the user's,
/// or is archived or changing, answers before the upgrade; a Host out of
/// reach answers after it, with `offline`.
pub(super) async fn open(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    let upgrade = upgrade.map_err(|_| {
        ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            ErrorCode::UpgradeRequired,
            "The file watch is a WebSocket",
        )
    })?;
    let opened = state
        .shards
        .of(&user.id)
        .call(move |shard, cancel| async move {
            shard
                .host_shard()
                .open_file_watch(&record.id, &cancel)
                .await
        })
        .await??;
    // An upgrade that never completes drops the channel, which ends the
    // watch.
    Ok(page_socket(upgrade).on_upgrade(move |socket| async move {
        match opened {
            OpenedWatch::Watching(channel) => relay(socket, channel).await,
            OpenedWatch::Offline => offline(socket).await,
        }
    }))
}

/// How a watch ended, which the page's socket closes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The Host became unreachable, after `offline`.
    HostUnreachable,
    /// The page sent a message that is not a `paths` message.
    Refused,
    /// A transition ended the watch.
    Changed,
    /// The page closed its socket, or it failed.
    PageClosed,
}

impl End {
    fn close(self) -> Option<(u16, &'static str)> {
        match self {
            Self::HostUnreachable => Some((1011, "host_unreachable")),
            Self::Refused => Some((1003, "invalid_message")),
            Self::Changed => Some((4000, "conversation_changed")),
            Self::PageClosed => None,
        }
    }
}

/// Tells the page that the Host is out of reach, and closes.
async fn offline(mut socket: WebSocket) {
    let message = FileWatchMessage::State {
        state: FileWatchState::Offline,
        reason: None,
    };
    if socket.send(text(&message)).await.is_ok() {
        // A page that went meanwhile hears nothing, which is what closing
        // tells it.
        let _ = socket.send(close(End::HostUnreachable)).await;
    }
}

/// Relays an open watch until either side ends it or a transition does;
/// dropping the lease then ends it in the shard.
async fn relay(socket: WebSocket, channel: FileWatchChannel) {
    let FileWatchChannel {
        mut to_page,
        from_page,
        lease,
    } = channel;
    let (mut page_out, mut page_in) = socket.split();
    let end = {
        let deliver = async {
            while let Some(message) = to_page.recv().await {
                if page_out.send(text(&message)).await.is_err() {
                    return End::PageClosed;
                }
            }
            End::HostUnreachable
        };
        let forward = async {
            loop {
                match page_in.next().await {
                    Some(Ok(Message::Text(message))) => {
                        let Some(paths) = paths_of(message.as_str()) else {
                            return End::Refused;
                        };
                        if from_page.send(paths).await.is_err() {
                            return End::Changed;
                        }
                    }
                    Some(Ok(Message::Binary(_))) => return End::Refused,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return End::PageClosed,
                }
            }
        };
        tokio::select! {
            biased;
            () = lease.ended() => End::Changed,
            end = deliver => end,
            end = forward => end,
        }
    };
    if end.close().is_some() {
        // As above, a page that went hears nothing.
        let _ = page_out.send(close(end)).await;
    }
    drop(lease);
}

/// The paths of a valid `paths` message.
fn paths_of(message: &str) -> Option<Vec<String>> {
    let request: FileWatchRequest = serde_json::from_str(message).ok()?;
    request.validate().ok()?;
    let FileWatchRequest::Paths { paths } = request;
    Some(paths)
}

fn text(message: &FileWatchMessage) -> Message {
    let json = serde_json::to_string(message).expect("a watch message serializes");
    Message::Text(json.into())
}

fn close(end: End) -> Message {
    let (code, reason) = end.close().expect("an end the page hears");
    Message::Close(Some(CloseFrame {
        code,
        reason: reason.into(),
    }))
}
