//! The file watch at the edge.
//!
//! It handles two things:
//!
//! - The opens of a conversation's watch and of a device's folder dialog's,
//!   lanes of the page connection.
//! - The relay of the Hosts' reports to the page, and of the page's
//!   `subscribe`, `unsubscribe` and `renew` messages to the shard, which holds
//!   the Hosts' subscriptions.
//!
//! Only a conversation's watch takes a subscription that names a host.

use demi_backend_host_access::file_watch::{FileWatchChannel, OpenedWatch};
use demi_backend_user_shard::shard::lane::{Lane, LaneClose, LaneEnd, LaneMessage, LaneReader};
use demi_web_api_protocol::files::{ConversationWatchRequest, FileWatchMessage, FileWatchRequest};
use demi_web_api_protocol::ids::UserId;
use garde::Validate;

use super::AppState;
use super::connection::Opened;
use super::conversations::owned;
use super::devices::owned_device;
use super::error::ApiError;

/// The watch of the caller's conversation `id`: one that is not the user's,
/// or is archived or changing, is refused; a Host out of reach is answered on
/// the lane, with `offline`, once a subscription names it.
pub(super) async fn open(state: &AppState, user: &UserId, id: &str) -> Result<Opened, ApiError> {
    let record = owned(&state.services, user, id).await?;
    let conversation = record.id.clone();
    let channel = state
        .shards
        .of(user)
        .call(move |shard, _| async move { shard.host_shard().open_file_watch(&record.id).await })
        .await??;
    // A lane that never opens drops the channel, which ends the watch.
    Ok(Opened {
        conversation: Some(conversation),
        ..Opened::new(move |lane, reader| relay(lane, reader, channel, request_of::<ConversationWatchRequest>))
    })
}

/// A device's folder dialog's watch, through device access
/// (`crates/backend-host-access/src/lib.rs`). A device that is not the
/// caller's paired device is refused; one whose runner is not connected is
/// answered on the lane, with `offline`.
pub(super) async fn open_device(state: &AppState, user: &UserId, id: &str) -> Result<Opened, ApiError> {
    // The Cloud's too, for the workflows of a new conversation in a Cloud
    // project; a stopped Cloud is answered `offline`, never woken.
    let device = owned_device(state, user, id, None).await?.id;
    let opened = {
        let device = device.clone();
        state
            .shards
            .of(user)
            .call(move |shard, _| async move { shard.host_shard().open_device_watch(&device).await })
            .await?
    };
    Ok(Opened {
        device: Some(device),
        ..Opened::new(move |lane, reader| async move {
            match opened {
                OpenedWatch::Watching(channel) => {
                    relay(lane, reader, channel, request_of::<FileWatchRequest>).await;
                }
                OpenedWatch::Offline => offline(lane, reader).await,
            }
        })
    })
}

/// The most bytes of one message to the page: a `changed` naming more keys
/// goes in as many as it takes.
///
/// Bounded by the page's own limit on a message
/// ([`demi_web_api_protocol::MAX_PAGE_MESSAGE_BYTES`]).
const MESSAGE_BYTES: usize = 1024 * 1024;

/// How a watch ended, which its lane closes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The Host became unreachable, after `offline`.
    HostUnreachable,
    /// The page sent a message it should not have.
    Refused,
    /// A transition ended the watch.
    Changed,
    /// The page closed the lane, or its connection ended.
    PageClosed,
}

impl End {
    /// How the lane ends: with the close the page hears, or as the page left
    /// it.
    fn lane_end(self) -> LaneEnd {
        let close = |code, reason| LaneEnd::Close(LaneClose { code, reason });
        match self {
            Self::HostUnreachable => close(1011, "host_unreachable"),
            Self::Refused => close(1003, "invalid_message"),
            Self::Changed => close(4000, "conversation_changed"),
            Self::PageClosed => LaneEnd::Gone,
        }
    }
}

/// Tells the page that the Host is out of reach, and closes.
async fn offline(mut page: Lane, page_in: LaneReader) {
    let end = match page.send(text(&FileWatchMessage::Offline)).await {
        Ok(()) => End::HostUnreachable,
        Err(_) => End::PageClosed,
    };
    page.end(page_in, end.lane_end());
}

/// Relays an open watch until either side ends it or a transition does;
/// dropping the lease then ends it in the shard. `read` reads a page's message as
/// the watch's route takes it.
async fn relay(
    mut page: Lane,
    mut page_in: LaneReader,
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
                let Some(message) = to_page.recv().await else {
                    return End::HostUnreachable;
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
                    Some(LaneMessage::Text(message)) => {
                        let Some(request) = read(&message) else {
                            return End::Refused;
                        };
                        if from_page.send(request).await.is_err() {
                            return End::Changed;
                        }
                    }
                    Some(LaneMessage::Binary(_)) => return End::Refused,
                    None => return End::PageClosed,
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
    page.end(page_in, end.lane_end());
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
