//! User streams at the edge (`web-api.md` § User streams): the upgrade of a
//! declared stream, and the relay of its bytes between the page's socket
//! and the Host's pipes, taken from the Host only as fast as the page takes
//! them.

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use demi_web_api_protocol::error::ErrorCode;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio_util::sync::CancellationToken;

use super::AppState;
use super::body::page_socket;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;
use demi_backend_host_access::stream::UserStream;

/// `WS /conversations/:id/streams/:name`: everything that can refuse the
/// stream answers before the upgrade, the Host's opening of it included.
pub(super) async fn open(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, name)): Path<(String, String)>,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let unknown = || {
        ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::UnknownStream,
            "No stream has that name",
        )
    };
    let Some(binding) = state.services.user_streams.get(&name).cloned() else {
        return Err(unknown());
    };
    let record = owned(&state.services, &user.id, &id).await?;
    let upgrade = upgrade.map_err(|_| {
        ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            ErrorCode::UpgradeRequired,
            "A user stream is a WebSocket",
        )
    })?;
    // A stream of a plugin the user has off does not exist; one whose
    // plugin is turned off while it is open ends.
    let opened = state
        .shards
        .of(&user.id)
        .call(move |shard, cancel| async move {
            let Some(plugin_off) = shard.plugins().stream_end(&name).await? else {
                return Ok(None);
            };
            let stream = shard
                .host_shard()
                .open_user_stream(&record.id, &binding, &cancel)
                .await?;
            Ok::<_, ApiError>(Some((stream, plugin_off)))
        })
        .await??;
    let (stream, plugin_off) = opened.ok_or_else(unknown)?;
    // An upgrade that never completes drops the stream, which ends it.
    Ok(page_socket(upgrade).on_upgrade(move |socket| relay(socket, stream, plugin_off)))
}

/// How a stream ended, which the page's socket closes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The invocation completed.
    Completed,
    /// The page sent a text message.
    BinaryOnly,
    /// The Host became unreachable, or a pipe to it failed.
    HostUnreachable,
    /// A transition ended the stream.
    Changed,
    /// The user turned off the stream's plugin.
    PluginDisabled,
    /// The page closed its socket, or it failed.
    PageClosed,
}

impl End {
    /// The close code and reason, when the page is still there to hear
    /// them.
    fn close(self) -> Option<(u16, &'static str)> {
        match self {
            Self::Completed => Some((1000, "completed")),
            Self::BinaryOnly => Some((1003, "binary_only")),
            Self::HostUnreachable => Some((1011, "host_unreachable")),
            Self::Changed => Some((4000, "conversation_changed")),
            Self::PluginDisabled => Some((4001, "plugin_disabled")),
            Self::PageClosed => None,
        }
    }
}

/// Relays an open stream: the page's binary messages go to the Host in
/// order, each after the Host took the one before, and the Host's bytes go
/// to the page, each after the page's socket took the one before. Message
/// boundaries mean nothing. Whichever side ends first ends the stream, and
/// so does `plugin_off`, once the user turns its plugin off; dropping the
/// lease then ends it in the shard.
async fn relay(socket: WebSocket, stream: UserStream, plugin_off: CancellationToken) {
    let UserStream {
        mut to_host,
        mut from_host,
        lease,
    } = stream;
    let (mut to_page, mut from_page) = socket.split();
    let end = {
        let deliver = async {
            loop {
                match from_host.next().await {
                    Some(Ok(bytes)) => {
                        if to_page.send(Message::Binary(bytes)).await.is_err() {
                            return End::PageClosed;
                        }
                    }
                    Some(Err(_)) => return End::HostUnreachable,
                    None => return End::Completed,
                }
            }
        };
        let forward = async {
            loop {
                match from_page.next().await {
                    Some(Ok(Message::Binary(bytes))) => {
                        if to_host.write(bytes).await.is_err() {
                            return End::HostUnreachable;
                        }
                    }
                    Some(Ok(Message::Text(_))) => return End::BinaryOnly,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return End::PageClosed,
                }
            }
        };
        // A transition fails the stream's pipes once it ended the lease, so
        // the lease's end is seen first.
        tokio::select! {
            biased;
            () = lease.ended() => End::Changed,
            () = plugin_off.cancelled() => End::PluginDisabled,
            end = deliver => end,
            end = forward => end,
        }
    };
    if let Some((code, reason)) = end.close() {
        let frame = CloseFrame {
            code,
            reason: reason.into(),
        };
        // A page that went meanwhile hears nothing, which is what closing
        // tells it.
        let _ = to_page.send(Message::Close(Some(frame))).await;
    }
    drop(lease);
}
