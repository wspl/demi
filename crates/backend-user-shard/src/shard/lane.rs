//! A lane of a page connection, as its owner holds it
//! (`crates/web-api-protocol/src/connection.rs`).
//!
//! The edge serves the connection and opens each lane, then hands it to its
//! owner: the shard serves the synchronization channel and a conversation's
//! lane; the edge relays a file watch, a device's signaling and a user stream.
//!
//! ```text
//!  connection (edge)                         owner
//!    reader --- FromConnection (credit) ---> LaneReader::next
//!    writer <-- ToConnection --------------- Lane::send, Lane::end
//! ```
//!
//! - A lane has the page's messages and its own, and nothing of the
//!   connection's: its heartbeat and the page's ping are the connection's.
//! - The connection holds as many of the page's messages for a lane as its
//!   credit allows, `LANE_CREDIT`, and grants the page one more each time the
//!   owner takes one, so an owner that is busy never stops the connection's
//!   other lanes.
//! - An owner's send resolves once its message is written to the connection,
//!   so a page that stops reading stops each owner as it sends.
//!
//! # Logging
//!
//! Each lane is logged here, and only here, once as it opens and once as it
//! ends, under the request ID of its open, beside the ID of its connection's
//! upgrade, so that a drop a user saw can be traced afterwards. The lines never
//! carry what the lane carried.

use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Bytes;
use demi_backend_database::accounts::TokenHash;
use demi_shared_types::{RequestId, Timestamp};
use demi_web_api_protocol::auth::UserDto;
use demi_web_api_protocol::sessions::SessionId;
use demi_web_api_protocol::connection::LANE_CREDIT;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// The session a page's connection opened with: its token's hash, its user,
/// and when it expires unless a request renews it. The connection ends with
/// it, every lane with the connection.
#[derive(Debug, Clone)]
pub struct PageSession {
    pub id: SessionId,
    pub token: TokenHash,
    pub user: UserDto,
    pub expires_at: Timestamp,
}

/// What a lane is for, as its log lines name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneKind {
    Sync,
    Conversation,
    FileWatch,
    Signaling,
    UserStream,
}

impl LaneKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Conversation => "conversation",
            Self::FileWatch => "file_watch",
            Self::Signaling => "signaling",
            Self::UserStream => "user_stream",
        }
    }

    /// Whose pieces the connection writes first: a user stream's bytes go
    /// after every other lane's messages, which are what the page waits on.
    pub fn priority(self) -> LanePriority {
        match self {
            Self::UserStream => LanePriority::Bulk,
            _ => LanePriority::Interactive,
        }
    }
}

/// The order in which the connection writes the lanes' pieces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanePriority {
    Interactive,
    Bulk,
}

/// Who opened a lane and what it serves, as its lines say.
#[derive(Debug, Clone)]
pub struct LaneOrigin {
    pub kind: LaneKind,
    /// The lane's number in its connection.
    pub lane: u32,
    /// The request ID of the lane's open, which the lane keeps for its whole
    /// life.
    pub request: RequestId,
    /// The request ID of the connection's upgrade.
    pub connection: RequestId,
    pub user: UserId,
    /// The conversation the lane serves, when it serves one.
    pub conversation: Option<ConversationId>,
    /// The device a signaling lane or a device's file watch reaches.
    pub device: Option<DeviceId>,
    /// The user stream's name.
    pub stream: Option<String>,
}

/// One message of a lane, whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneMessage {
    Text(String),
    Binary(Bytes),
}

/// What a lane tells its connection.
#[derive(Debug)]
pub enum ToConnection {
    /// A message to write, whose `written` says once it went.
    Data {
        lane: u32,
        priority: LanePriority,
        message: LaneMessage,
        written: oneshot::Sender<()>,
    },
    /// The owner closes the lane with this code and reason.
    Close { lane: u32, code: u16, reason: String },
    /// The owner took one of the page's messages: the page may send one more.
    Credit { lane: u32 },
    /// The lane ended: the connection forgets it.
    Ended { lane: u32 },
}

/// What the connection brings a lane.
#[derive(Debug)]
pub enum FromConnection {
    Message(LaneMessage),
    /// The lane closed with this code and reason: the page closed it, or,
    /// when not `by_page`, the connection did for a message of the page's
    /// that broke the lane's rules, such as one past its credit.
    Closed { code: u16, reason: String, by_page: bool },
}

/// The sending half of a lane, which knows when it last sent.
pub struct Lane {
    origin: LaneOrigin,
    to_connection: mpsc::UnboundedSender<ToConnection>,
    /// When the lane last sent a message, or was opened.
    sent_at: Instant,
    /// The messages it sent.
    sent: u64,
    opened_at: Instant,
    /// Whether the owner ended it; one dropped without an end ends as it
    /// drops, as one a shard that is closing never adopted.
    ended: bool,
    /// Counts the lane among the open ones until it is dropped.
    _open: Counted,
}

/// The receiving half of a lane, which notes when the page last sent and how
/// the page closed it.
pub struct LaneReader {
    lane: u32,
    from_connection: mpsc::Receiver<FromConnection>,
    to_connection: mpsc::UnboundedSender<ToConnection>,
    /// When the page last sent a message, or the lane opened.
    received_at: Instant,
    /// The lane's close, once it came: its code and reason, and whether the
    /// page sent it.
    closed: Option<(u16, String, bool)>,
    /// Cancelled when the connection ends: what the page sent that the owner
    /// has not taken is dropped, so nothing of a session that ended runs.
    connection_ended: CancellationToken,
}

/// A send failed: the page closed the lane, or the connection ended.
#[derive(Debug)]
pub struct LaneGone;

/// A close the backend ends a lane with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneClose {
    pub code: u16,
    pub reason: &'static str,
}

/// How a lane ended, as its owner tells [`Lane::end`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneEnd {
    /// The page's side ended: it closed the lane, or the connection ended,
    /// which the reader tells apart.
    Gone,
    /// The backend closes the lane.
    Close(LaneClose),
    /// The backend closes the lane for a failure, which warns: a page that
    /// fell behind, or the backend's own error.
    Fail(LaneClose),
    /// The lane ends for this reason with no word of its own, which the page
    /// hears as a lane that broke.
    Dropped(&'static str),
}

/// The close a page hears of a lane that ended with no word of its own, as
/// it hears a WebSocket that broke.
const ABNORMAL: u16 = 1006;

/// The lanes open now, in every connection, which the backend's resource
/// sample reads.
static OPEN_LANES: AtomicU64 = AtomicU64::new(0);
/// The page connections open now.
static OPEN_CONNECTIONS: AtomicU64 = AtomicU64::new(0);

/// How many lanes are open now.
pub fn open_lanes() -> u64 {
    OPEN_LANES.load(Ordering::Relaxed)
}

/// How many page connections are open now.
pub fn open_connections() -> u64 {
    OPEN_CONNECTIONS.load(Ordering::Relaxed)
}

/// Counts one of what its counter counts until it is dropped.
pub struct Counted(&'static AtomicU64);

impl Counted {
    fn new(counter: &'static AtomicU64) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }

    /// Counts a page connection while it is open.
    pub fn connection() -> Self {
        Self::new(&OPEN_CONNECTIONS)
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Opens the lane `origin` names on its connection, which `to_connection`
/// reaches and `connection_ended` says the end of, and logs that it opened.
/// The connection keeps the sender it answers, which brings the lane the
/// page's messages and close.
pub fn open_lane(
    origin: LaneOrigin,
    to_connection: mpsc::UnboundedSender<ToConnection>,
    connection_ended: CancellationToken,
) -> (Lane, LaneReader, mpsc::Sender<FromConnection>) {
    tracing::info!(
        event = "lane.opened",
        request_id = %origin.request,
        connection = %origin.connection,
        lane = origin.lane,
        kind = origin.kind.name(),
        user = %origin.user,
        conversation = origin.conversation.as_ref().map(tracing::field::display),
        host_id = origin.device.as_ref().map(tracing::field::display),
        stream = origin.stream.as_deref(),
        "a lane opened"
    );
    // One more than the credit, for the page's close behind a full credit.
    let (from_page, from_connection) = mpsc::channel(usize::from(LANE_CREDIT) + 1);
    let now = Instant::now();
    let reader = LaneReader {
        lane: origin.lane,
        from_connection,
        to_connection: to_connection.clone(),
        received_at: now,
        closed: None,
        connection_ended,
    };
    let lane = Lane {
        origin,
        to_connection,
        sent_at: now,
        sent: 0,
        opened_at: now,
        ended: false,
        _open: Counted::new(&OPEN_LANES),
    };
    (lane, reader, from_page)
}

impl Lane {
    /// The request ID of the lane's open.
    pub fn request(&self) -> &RequestId {
        &self.origin.request
    }

    /// Sends one JSON text message.
    pub async fn send(&mut self, text: String) -> Result<(), LaneGone> {
        self.send_message(LaneMessage::Text(text)).await
    }

    /// Sends one binary message.
    pub async fn send_bytes(&mut self, bytes: Bytes) -> Result<(), LaneGone> {
        self.send_message(LaneMessage::Binary(bytes)).await
    }

    async fn send_message(&mut self, message: LaneMessage) -> Result<(), LaneGone> {
        let (written, went) = oneshot::channel();
        self.to_connection
            .send(ToConnection::Data {
                lane: self.origin.lane,
                priority: self.origin.kind.priority(),
                message,
                written,
            })
            .map_err(|_| LaneGone)?;
        // A connection that ended drops what it had yet to write.
        went.await.map_err(|_| LaneGone)?;
        self.sent_at = Instant::now();
        self.sent += 1;
        Ok(())
    }

    /// Ends the lane as `end` says, and logs why.
    pub fn end(mut self, reader: LaneReader, end: LaneEnd) {
        self.finish(Some(&reader), end);
    }

    fn finish(&mut self, reader: Option<&LaneReader>, end: LaneEnd) {
        self.ended = true;
        let lane = self.origin.lane;
        let (reason, code, close_reason) = match end {
            LaneEnd::Gone => match reader.and_then(|reader| reader.closed.as_ref()) {
                Some((code, reason, true)) => ("page closed", Some(*code), Some(reason.as_str())),
                Some((code, reason, false)) => ("connection closed", Some(*code), Some(reason.as_str())),
                None => ("connection ended", None, None),
            },
            LaneEnd::Close(close) | LaneEnd::Fail(close) => {
                ("backend closed", Some(close.code), Some(close.reason))
            }
            LaneEnd::Dropped(why) => (why, None, None),
        };
        let told = match end {
            LaneEnd::Gone => None,
            LaneEnd::Close(close) | LaneEnd::Fail(close) => Some((close.code, close.reason)),
            LaneEnd::Dropped(_) => Some((ABNORMAL, "")),
        };
        // A connection that ended hears nothing more.
        if let Some((code, reason)) = told {
            let _ = self.to_connection.send(ToConnection::Close {
                lane,
                code,
                reason: reason.to_owned(),
            });
        }
        let _ = self.to_connection.send(ToConnection::Ended { lane });
        let since_received_ms = reader.map(|reader| millis(reader.received_at.elapsed()));
        macro_rules! ended {
            ($level:ident) => {
                tracing::$level!(
                    event = "lane.ended",
                    request_id = %self.origin.request,
                    connection = %self.origin.connection,
                    lane,
                    kind = self.origin.kind.name(),
                    user = %self.origin.user,
                    conversation = self.origin.conversation.as_ref().map(tracing::field::display),
                    host_id = self.origin.device.as_ref().map(tracing::field::display),
                    reason,
                    close_code = code,
                    close_reason,
                    life_ms = millis(self.opened_at.elapsed()),
                    since_sent_ms = millis(self.sent_at.elapsed()),
                    since_received_ms,
                    sent = self.sent,
                    "a lane ended"
                )
            };
        }
        // A lane its owner never ended is a defect, or a shard that closed
        // before it took the lane.
        if matches!(end, LaneEnd::Fail(_)) || reader.is_none() {
            ended!(warn);
        } else {
            ended!(info);
        }
    }
}

impl Drop for Lane {
    fn drop(&mut self) {
        if !self.ended {
            self.finish(None, LaneEnd::Dropped("dropped without an end"));
        }
    }
}

impl LaneReader {
    /// The page's next message; none once the page closed the lane or the
    /// connection ended.
    pub async fn next(&mut self) -> Option<LaneMessage> {
        if self.closed.is_some() || self.connection_ended.is_cancelled() {
            return None;
        }
        let received = tokio::select! {
            biased;
            () = self.connection_ended.cancelled() => return None,
            received = self.from_connection.recv() => received?,
        };
        match received {
            FromConnection::Message(message) => {
                self.received_at = Instant::now();
                // A connection that ended takes no credit.
                let _ = self.to_connection.send(ToConnection::Credit { lane: self.lane });
                Some(message)
            }
            FromConnection::Closed { code, reason, by_page } => {
                self.received_at = Instant::now();
                self.closed = Some((code, reason, by_page));
                None
            }
        }
    }

    /// The page's next message; while `busy`, as while a conversation's lane
    /// handles the last one, none: the page's next messages wait in the
    /// lane's credit for the reads once it is not busy.
    pub async fn next_unless_busy(&mut self, busy: bool) -> Option<LaneMessage> {
        if busy {
            return std::future::pending().await;
        }
        self.next().await
    }
}

/// A duration as the integer `_ms` fields of the log hold it.
fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
