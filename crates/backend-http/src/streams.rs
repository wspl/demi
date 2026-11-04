//! User streams at the edge.
//!
//! - The open of a declared stream, a lane of the page connection.
//! - The relay of its bytes between the lane and the Host's pipes, taken from
//!   the Host only as fast as the page takes them.
//! - The page's word on whether it watches the stream, a text message of the
//!   user stream's own layer (`crates/runner-protocol/src/direct.rs`,
//!   `StreamWatched`), which decides whether the stream is activity.
//!
//! The relay serves any page side: a lane of the page connection, or a view
//! link's own socket (`browser_views.rs`).

use std::future::Future;

use axum::http::StatusCode;
use bytes::Bytes;
use demi_backend_user_shard::shard::lane::{Lane, LaneClose, LaneEnd, LaneMessage, LaneReader};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::UserId;

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
                    .open_user_stream(&record.id, &binding, &serde_json::Map::new(), &cancel)
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
        ..Opened::new(move |lane, reader| async move {
            let until = async move {
                plugin_off.cancelled().await;
                End::PluginDisabled
            };
            relay(lane, reader, stream, until, |lane, reader, end| lane.end(reader, end.close())).await;
        })
    })
}

/// The page's side of a stream, where the Host's bytes go.
pub(super) trait ToPage {
    /// Writes `bytes` once what went before was written; an error once the
    /// page is gone.
    fn send_bytes(&mut self, bytes: Bytes) -> impl Future<Output = Result<(), ()>>;
}

/// The page's side of a stream, which its messages come from.
pub(super) trait FromPage {
    /// The page's next message; none once it closed.
    fn next(&mut self) -> impl Future<Output = Option<LaneMessage>>;
}

impl ToPage for Lane {
    async fn send_bytes(&mut self, bytes: Bytes) -> Result<(), ()> {
        Lane::send_bytes(self, bytes).await.map_err(|_| ())
    }
}

impl FromPage for LaneReader {
    async fn next(&mut self) -> Option<LaneMessage> {
        LaneReader::next(self).await
    }
}

/// How a stream ended, which its page side closes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum End {
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
    /// The view link the page opened the stream by expired.
    ViewExpired,
    /// The view link the page opened the stream by was removed.
    ViewRemoved,
    /// The view link the page opened the stream by no longer opens.
    ViewNotFound,
}

impl End {
    /// How the page side ends: with the close the page hears, or as the page
    /// left it.
    pub(super) fn close(self) -> LaneEnd {
        let frame = |code, reason| LaneEnd::Close(LaneClose { code, reason });
        match self {
            Self::Completed => frame(1000, "completed"),
            Self::BinaryOnly => frame(1003, "binary_only"),
            Self::HostUnreachable => frame(1011, "host_unreachable"),
            Self::Changed => frame(4000, "conversation_changed"),
            Self::PluginDisabled => frame(4001, "plugin_disabled"),
            Self::ViewExpired => frame(4002, "view_expired"),
            Self::ViewRemoved => frame(4003, "view_removed"),
            Self::ViewNotFound => frame(4004, "view_not_found"),
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
/// - Whichever side ends first ends the stream, and so does `until`, such as
///   once the user turns its plugin off. `close` ends the page side with how
///   it ended; dropping the lease then ends it in the shard.
pub(super) async fn relay<T: ToPage, F: FromPage>(
    mut to_page: T,
    mut from_page: F,
    stream: UserStream,
    until: impl Future<Output = End>,
    close: impl FnOnce(T, F, End),
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
            end = until => end,
            end = deliver => end,
            end = forward => end,
        }
    };
    close(to_page, from_page, end);
    drop(lease);
}
