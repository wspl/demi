//! The file watch at the edge (`web-api.md` § File watch): the upgrade, and
//! the relay of the Host's reports to the page and of the page's `paths`
//! messages to the shard, which follows the Host's watches.

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocketUpgrade};
use demi_backend_user_shard::shard::page_socket::{
    PageEnd, PageSocket, PageSocketKind, PageSocketOrigin, PageStream, open_page_socket,
};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use demi_backend_host_access::file_watch::{FileWatchChannel, OpenedWatch};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::files::{FileWatchMessage, FileWatchRequest, FileWatchState};
use futures_util::StreamExt as _;
use garde::Validate as _;

use super::AppState;
use super::body::page_socket;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;
use super::page_client::PageClient;

/// `WS /conversations/:id/fs/watch`: a conversation that is not the user's,
/// or is archived or changing, answers before the upgrade; a Host out of
/// reach answers after it, with `offline`.
pub(super) async fn open(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    client: PageClient,
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
    let origin = PageSocketOrigin {
        conversation: Some(record.id.clone()),
        ..client.origin(PageSocketKind::FileWatch, user.id.clone())
    };
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
    let tuning = state.services.pages;
    // An upgrade that never completes drops the channel, which ends the
    // watch.
    Ok(page_socket(upgrade).on_upgrade(move |socket| async move {
        let (to_page, from_page) = open_page_socket(socket, tuning, origin);
        match opened {
            OpenedWatch::Watching(channel) => relay(to_page, from_page, channel).await,
            OpenedWatch::Offline => offline(to_page, from_page).await,
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
    /// How the socket ends: with the close frame the page hears, or as the
    /// page left it.
    fn page_end(self) -> PageEnd {
        let frame = |code, reason: &'static str| {
            PageEnd::Close(CloseFrame {
                code,
                reason: reason.into(),
            })
        };
        match self {
            Self::HostUnreachable => frame(1011, "host_unreachable"),
            Self::Refused => frame(1003, "invalid_message"),
            Self::Changed => frame(4000, "conversation_changed"),
            Self::PageClosed => PageEnd::Gone,
        }
    }
}

/// Tells the page that the Host is out of reach, and closes.
async fn offline(mut page: PageSocket, page_in: PageStream) {
    let message = FileWatchMessage::State {
        state: FileWatchState::Offline,
        reason: None,
    };
    let end = match page.send(text(&message)).await {
        Ok(()) => End::HostUnreachable,
        Err(_) => End::PageClosed,
    };
    page.end(page_in, end.page_end()).await;
}

/// Relays an open watch until either side ends it or a transition does,
/// with a `heartbeat` after 30 seconds without another message; dropping
/// the lease then ends it in the shard.
async fn relay(mut page: PageSocket, mut page_in: PageStream, channel: FileWatchChannel) {
    let FileWatchChannel {
        mut to_page,
        from_page,
        lease,
    } = channel;
    let end = {
        let deliver = async {
            loop {
                let message = tokio::select! {
                    message = to_page.recv() => match message {
                        Some(message) => message,
                        None => return End::HostUnreachable,
                    },
                    () = page.silent() => FileWatchMessage::Heartbeat,
                };
                if page.send(text(&message)).await.is_err() {
                    return End::PageClosed;
                }
            }
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
    page.end(page_in, end.page_end()).await;
    drop(lease);
}

/// The paths of a valid `paths` message.
fn paths_of(message: &str) -> Option<Vec<String>> {
    let request: FileWatchRequest = serde_json::from_str(message).ok()?;
    request.validate().ok()?;
    let FileWatchRequest::Paths { paths } = request;
    Some(paths)
}

fn text(message: &FileWatchMessage) -> String {
    serde_json::to_string(message).expect("a watch message serializes")
}
