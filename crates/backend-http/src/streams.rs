//! User streams at the edge.
//!
//! - The open of a declared stream, a lane of the page connection.
//! - The relay of its bytes between the lane and the Host's pipes, taken from
//!   the Host only as fast as the page takes them.
//! - The page's word on whether it watches the stream, a text message of the
//!   user stream's own layer (`crates/runner-protocol/src/direct.rs`,
//!   `StreamWatched`), which decides whether the stream is activity.

use axum::http::StatusCode;
use demi_backend_user_shard::shard::lane::{Lane, LaneClose, LaneEnd, LaneMessage, LaneReader};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::UserId;
use tokio_util::sync::CancellationToken;

use super::AppState;
use super::connection::Opened;
use super::conversations::owned;
use super::error::ApiError;
use demi_backend_host_access::stream::UserStream;
use demi_runner_protocol::direct::StreamWatched;

/// The stream `name` of the caller's conversation `id`: everything that can
/// refuse it refuses the open, the Host's opening of it included.
pub(super) async fn open(state: &AppState, user: &UserId, id: &str, name: String) -> Result<Opened, ApiError> {
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
    let record = owned(&state.services, user, id).await?;
    let conversation = record.id.clone();
    // A stream of a plugin the user has off does not exist; one whose
    // plugin is turned off while it is open ends.
    let opened = {
        let name = name.clone();
        state
            .shards
            .of(user)
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
            .await??
    };
    let (stream, plugin_off) = opened.ok_or_else(unknown)?;
    // A lane that never opens drops the stream, which ends it.
    Ok(Opened {
        conversation: Some(conversation),
        stream: Some(name),
        ..Opened::new(move |lane, reader| relay(lane, reader, stream, plugin_off))
    })
}

/// How a stream ended, which its lane closes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The invocation completed.
    Completed,
    /// The page sent a text message that says nothing of the user stream's
    /// layer: the stream's own bytes are binary.
    BinaryOnly,
    /// The Host became unreachable, or a pipe to it failed.
    HostUnreachable,
    /// A transition ended the stream.
    Changed,
    /// The user turned off the stream's plugin.
    PluginDisabled,
    /// The page closed the lane, or its connection ended.
    PageClosed,
}

impl End {
    /// How the lane ends: with the close the page hears, or as the page left
    /// it.
    fn lane_end(self) -> LaneEnd {
        let frame = |code, reason| LaneEnd::Close(LaneClose { code, reason });
        match self {
            Self::Completed => frame(1000, "completed"),
            Self::BinaryOnly => frame(1003, "binary_only"),
            Self::HostUnreachable => frame(1011, "host_unreachable"),
            Self::Changed => frame(4000, "conversation_changed"),
            Self::PluginDisabled => frame(4001, "plugin_disabled"),
            Self::PageClosed => LaneEnd::Gone,
        }
    }
}

/// Relays an open stream.
///
/// - The page's binary messages go to the Host in order, each after the Host
///   took the one before.
/// - The Host's bytes go to the page, each after the connection wrote the one
///   before.
/// - Message boundaries mean nothing.
/// - The page's text message says whether it watches the stream.
/// - Whichever side ends first ends the stream, and so does `plugin_off`, once
///   the user turns its plugin off; dropping the lease then ends it in the
///   shard.
async fn relay(
    mut to_page: Lane,
    mut from_page: LaneReader,
    stream: UserStream,
    plugin_off: CancellationToken,
) {
    let UserStream {
        mut to_host,
        mut from_host,
        watched,
        lease,
    } = stream;
    let end = {
        let deliver = async {
            loop {
                match from_host.next().await {
                    Some(Ok(bytes)) => {
                        if to_page.send_bytes(bytes).await.is_err() {
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
                    Some(LaneMessage::Binary(bytes)) => {
                        if to_host.write(bytes).await.is_err() {
                            return End::HostUnreachable;
                        }
                    }
                    Some(LaneMessage::Text(text)) => {
                        match serde_json::from_str::<StreamWatched>(&text) {
                            Ok(said) => {
                                watched.send_replace(said.watched);
                            }
                            Err(_) => return End::BinaryOnly,
                        }
                    }
                    None => return End::PageClosed,
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
    to_page.end(from_page, end.lane_end());
    drop(lease);
}
