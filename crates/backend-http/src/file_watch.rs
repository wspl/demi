//! The file watch at the edge (`web-api-hosts.md` § File watch): the upgrades of
//! a conversation's watch and of a device's folder dialog's, and the relay
//! of the Hosts' reports to the page and of the page's `subscribe`,
//! `unsubscribe` and `renew` messages to the shard, which holds the Hosts'
//! subscriptions. Only a conversation's watch takes a subscription that
//! names a host.

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
use demi_web_api_protocol::files::{ConversationWatchRequest, FileWatchMessage, FileWatchRequest};
use futures_util::StreamExt as _;
use garde::Validate;

use super::AppState;
use super::body::page_socket;
use super::conversations::owned;
use super::devices::owned_device;
use super::error::ApiError;
use super::gate::AuthUser;
use super::page_client::PageClient;

/// `WS /conversations/:id/fs/watch`: a conversation that is not the user's,
/// or is archived or changing, answers before the upgrade; a Host out of
/// reach answers after it, with `offline`, once a subscription names it.
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
    let channel = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.host_shard().open_file_watch(&record.id).await })
        .await??;
    let tuning = state.services.pages;
    // An upgrade that never completes drops the channel, which ends the
    // watch.
    Ok(page_socket(upgrade, move |socket| async move {
        let (to_page, from_page) = open_page_socket(socket, tuning, origin);
        relay(to_page, from_page, channel, request_of::<ConversationWatchRequest>).await;
    }))
}

/// `WS /devices/:deviceId/fs/watch`: a device's folder dialog's watch,
/// through device access (`sessions-and-targets.md` § Every way to a
/// Host). A device that is not the caller's paired device answers before
/// the upgrade; one whose runner is not connected answers after it, with
/// `offline`.
pub(super) async fn open_device(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    client: PageClient,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    // The Cloud's too, for the workflows of a new conversation in a Cloud
    // project; a stopped Cloud is answered `offline`, never woken.
    let device = owned_device(&state, &user.id, &id, None).await?;
    let upgrade = upgrade.map_err(|_| {
        ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            ErrorCode::UpgradeRequired,
            "The file watch is a WebSocket",
        )
    })?;
    let origin = PageSocketOrigin {
        device: Some(device.id.clone()),
        ..client.origin(PageSocketKind::FileWatch, user.id.clone())
    };
    let device = device.id;
    let opened = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.host_shard().open_device_watch(&device) })
        .await?;
    let tuning = state.services.pages;
    Ok(page_socket(upgrade, move |socket| async move {
        let (to_page, from_page) = open_page_socket(socket, tuning, origin);
        match opened {
            OpenedWatch::Watching(channel) => {
                relay(to_page, from_page, channel, request_of::<FileWatchRequest>).await;
            }
            OpenedWatch::Offline => offline(to_page, from_page).await,
        }
    }))
}

/// The most bytes of one message to the page: a `changed` naming more keys
/// goes in as many as it takes.
const MESSAGE_BYTES: usize = 1024 * 1024;

/// How a watch ended, which the page's socket closes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The Host became unreachable, after `offline`.
    HostUnreachable,
    /// The page sent a message it should not have.
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
    let end = match page.send(text(&FileWatchMessage::Offline)).await {
        Ok(()) => End::HostUnreachable,
        Err(_) => End::PageClosed,
    };
    page.end(page_in, end.page_end()).await;
}

/// Relays an open watch until either side ends it or a transition does,
/// with a `heartbeat` after 30 seconds without another message; dropping
/// the lease then ends it in the shard. `read` reads a page's message as
/// the watch's route takes it.
async fn relay(
    mut page: PageSocket,
    mut page_in: PageStream,
    channel: FileWatchChannel,
    read: fn(&str) -> Option<ConversationWatchRequest>,
) {
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
                for text in message.into_texts(MESSAGE_BYTES) {
                    if page.send(text).await.is_err() {
                        return End::PageClosed;
                    }
                }
            }
        };
        let forward = async {
            loop {
                match page_in.next().await {
                    Some(Ok(Message::Text(message))) => {
                        let Some(request) = read(message.as_str()) else {
                            return End::Refused;
                        };
                        if from_page.send(request).await.is_err() {
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

/// A valid message of the page's, as `R`, the route's message, takes it.
fn request_of<R>(message: &str) -> Option<ConversationWatchRequest>
where
    R: serde::de::DeserializeOwned + Validate<Context = ()> + Into<ConversationWatchRequest>,
{
    let request: R = serde_json::from_str(message).ok()?;
    request.validate().ok()?;
    Some(request.into())
}

fn text(message: &FileWatchMessage) -> String {
    serde_json::to_string(message).expect("a watch message serializes")
}
