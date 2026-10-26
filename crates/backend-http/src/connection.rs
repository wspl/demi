//! `WS /api/connect`: a page's connection, and the lanes it carries
//! (`crates/web-api-protocol/src/connection.rs`).
//!
//! The upgrade checks the session cookie without renewing the session, since
//! only requests renew it. The connection then runs in two tasks:
//!
//! ```text
//!  page --> reader (this task) --- the page's messages ---> owners
//!            |  each open checked in a task of its own
//!            |  owners' messages, closes and credit
//!            v
//!  page <-- writer <-- connection's own, then lanes' pieces in turn
//! ```
//!
//! - Each open is checked as the lane's own route checked its upgrade, with
//!   the session as it is now, and is refused with the status and error that
//!   route answered; an account still to choose its password opens only the
//!   synchronization channel. One open never waits for another.
//! - The page sends a lane's messages behind its open without waiting for
//!   `opened`: what comes while the open is checked is held, within the
//!   lane's credit, and its owner takes it first once the lane opens; a
//!   refused open drops it.
//! - An opened lane is served by its owner: the user's shard serves the
//!   synchronization channel and a conversation's lane, the edge the rest.
//! - The writer writes the connection's own messages and every lane's
//!   opened, refused, close and credit first, then one piece of each lane
//!   with a message to send in turn, a user stream's after the others', so a
//!   large message holds another lane back by one piece at most. Once it has
//!   written nothing for the heartbeat interval it writes a `heartbeat`.
//! - The connection ends with its session, as it is signed out or expires,
//!   with `session_ended`; as the backend shuts down, with
//!   `backend_closing`; and as the page breaks the protocol, with
//!   `protocol_error`. Every lane ends with it at once: what the page sent
//!   that the lane's owner has not taken is dropped, unread.
//!
//! # Logging
//!
//! The connection logs one line as it opens and one as it ends, under the
//! request ID of its upgrade; each lane logs its own (`crates/backend-user-
//! shard/src/shard/lane.rs`) under the request ID of its open, in a span of
//! its own, with the connection's ID beside it.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade, close_code};
use axum::http::StatusCode;
use axum::response::Response;
use axum_extra::extract::CookieJar;
use demi_backend_database::accounts::TokenHash;
use demi_backend_user_shard::services::Services;
use demi_backend_user_shard::shard::lane::{
    Counted, FromConnection, Lane, LaneKind, LaneMessage, LaneOrigin, LanePriority, LaneReader,
    PageSession, ToConnection, open_lane,
};
use demi_shared_types::{RequestId, Timestamp};
use demi_web_api_protocol::MAX_PAGE_MESSAGE_BYTES;
use demi_web_api_protocol::connection::{
    BINARY_FLAG, BackendConnectionMessage, CLOSE_FRAME, CONNECTION_FRAME, CREDIT_FRAME,
    DATA_FRAME, LANE_CREDIT, LANE_PIECE_BYTES, LaneRefusal, LaneTarget, MAX_LANES, MORE_FLAG,
    OPEN_FRAME, OPENED_FRAME, PageConnectionMessage, REFUSED_FRAME,
};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::BoxFuture;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::AppState;
use super::cookies::SESSION_COOKIE;
use super::error::ApiError;
use super::page_client::PageClient;
use super::request_id;
use super::{conversations, direct, file_watch, streams, sync};

/// The most bytes of one message the page sends on the connection: a piece
/// and its header, or an open and its target, with room to spare.
///
/// Bounded because a message is read whole into memory and a client sends
/// without end.
const MAX_CONNECTION_MESSAGE_BYTES: usize = 64 * 1024;

/// The close code of a connection whose session was signed out or expired.
const SESSION_ENDED: u16 = 4002;

/// Cancelled as the backend begins to shut down.
#[derive(Debug, Clone)]
pub(super) struct Closing(pub(super) CancellationToken);

/// What serves an opened lane until it ends.
pub(super) type Serve = Box<dyn FnOnce(Lane, LaneReader) -> BoxFuture<'static, ()> + Send>;

/// A lane its checks let open: what serves it, and what it serves, which its
/// log lines name.
pub(super) struct Opened {
    pub(super) serve: Serve,
    pub(super) conversation: Option<ConversationId>,
    pub(super) device: Option<DeviceId>,
    pub(super) stream: Option<String>,
}

impl Opened {
    pub(super) fn new<F, Fut>(serve: F) -> Self
    where
        F: FnOnce(Lane, LaneReader) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self {
            serve: Box::new(move |lane, reader| Box::pin(serve(lane, reader))),
            conversation: None,
            device: None,
            stream: None,
        }
    }
}

/// `WS /api/connect?page=<id>`: a missing or ended session answers 401
/// before the upgrade.
pub(super) async fn connect(
    State(state): State<AppState>,
    Extension(Closing(closing)): Extension<Closing>,
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
    tracing::Span::current().record("user_id", tracing::field::display(&session.user.id));
    let upgrade = upgrade.map_err(|_| {
        ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            ErrorCode::UpgradeRequired,
            "The page connection is a WebSocket",
        )
    })?;
    let session = PageSession {
        id: session.id,
        token,
        user: session.user,
        expires_at: session.expires_at,
    };
    let span = tracing::Span::current();
    Ok(upgrade
        .max_message_size(MAX_CONNECTION_MESSAGE_BYTES)
        .max_frame_size(MAX_CONNECTION_MESSAGE_BYTES)
        .on_upgrade(move |socket| serve(socket, state, closing, client, session).instrument(span)))
}

/// How a connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// The page closed the connection, or it broke.
    PageGone,
    /// A write to the page failed.
    WriteFailed,
    BackendClosing,
    /// The session was signed out, or expired.
    SessionEnded,
    /// The session could not be read at its expiry.
    SessionUnread,
    /// The page broke the protocol in the way this says.
    Protocol(&'static str),
}

impl Ending {
    /// The close the page hears, none when it hears nothing more.
    fn close(self) -> Option<(u16, &'static str)> {
        match self {
            Self::PageGone | Self::WriteFailed => None,
            Self::BackendClosing => Some((close_code::AWAY, "backend_closing")),
            Self::SessionEnded => Some((SESSION_ENDED, "session_ended")),
            Self::SessionUnread => Some((close_code::ERROR, "internal_error")),
            Self::Protocol(_) => Some((close_code::PROTOCOL, "protocol_error")),
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::PageGone => "page closed",
            Self::WriteFailed => "write failed",
            Self::BackendClosing => "backend closing",
            Self::SessionEnded => "session ended",
            Self::SessionUnread => "session unread",
            Self::Protocol(why) => why,
        }
    }
}

/// A lane of the connection, as the reader holds it.
enum Slot {
    /// Its open is being checked, by this task.
    Opening(Opening),
    Open(OpenLane),
}

/// A lane whose open is being checked, and what the page sent on it
/// meanwhile, which its owner takes first once it opens.
struct Opening {
    task: AbortHandle,
    /// The whole messages the page sent, each with whether it is binary.
    messages: Vec<(bool, Bytes)>,
    /// The message whose pieces are still coming, with whether it is binary.
    assembling: Option<(bool, Vec<u8>)>,
    /// Why the lane closes once it opens: the page sent past its credit or a
    /// message larger than one may be, and what followed was not kept.
    overrun: Option<(u16, &'static str)>,
}

impl Opening {
    /// Keeps a piece the page sent before the open was answered, put
    /// together into its message, within what an open lane would take: at
    /// most the lane's credit of messages, each at most
    /// `MAX_PAGE_MESSAGE_BYTES`, so a page holds no more here than once the
    /// lane opens.
    fn hold(&mut self, flags: u8, piece: Bytes) {
        if self.overrun.is_some() {
            return;
        }
        if self.messages.len() >= usize::from(LANE_CREDIT) {
            self.overrun = Some((close_code::POLICY, "credit_exceeded"));
            return;
        }
        let (binary, mut whole) = self
            .assembling
            .take()
            .unwrap_or_else(|| (flags & BINARY_FLAG != 0, Vec::new()));
        if whole.len() + piece.len() > MAX_PAGE_MESSAGE_BYTES {
            self.overrun = Some((close_code::SIZE, "message_too_big"));
            return;
        }
        whole.extend_from_slice(&piece);
        if flags & MORE_FLAG == 0 {
            self.messages.push((binary, Bytes::from(whole)));
        } else {
            self.assembling = Some((binary, whole));
        }
    }
}

struct OpenLane {
    /// Brings the owner the page's messages and the lane's close.
    to_owner: mpsc::Sender<FromConnection>,
    /// The page's messages the owner has not taken yet, at most the credit.
    held: u16,
    /// The pieces of the page's message that more pieces complete.
    assembling: Option<Vec<u8>>,
    /// Whether the lane closed: the page's messages for it are dropped, and
    /// it waits for its owner to end it.
    closed: bool,
}

/// An open's check, answered.
struct Answer {
    lane: u32,
    kind: LaneKind,
    request: RequestId,
    span: tracing::Span,
    opened: Result<Opened, LaneRefusal>,
}

/// What the reader asks the writer to write.
enum Write {
    /// A message of the connection's own, or a lane's opened, refused or
    /// credit, ahead of every lane's pieces.
    Control(Bytes),
    /// A lane's message, in pieces, whose `written` says once the last went.
    Data {
        lane: u32,
        priority: LanePriority,
        message: LaneMessage,
        written: oneshot::Sender<()>,
    },
    /// A lane's close, which drops what of the lane is still to write.
    Close { lane: u32, frame: Bytes },
    /// Drops what of a lane that closed is still to write, with no frame.
    Forget { lane: u32 },
    /// The connection's close, after which nothing is written.
    Finish { code: u16, reason: &'static str },
}

/// The reader's state.
struct Connection {
    state: AppState,
    session: PageSession,
    client: PageClient,
    lanes: HashMap<u32, Slot>,
    /// The number of the last lane the page opened.
    last_lane: u32,
    to_writer: mpsc::UnboundedSender<Write>,
    /// What every lane's owner tells the connection by.
    to_connection: mpsc::UnboundedSender<ToConnection>,
    /// Cancelled when the connection ends, which ends every lane's reads.
    ended: CancellationToken,
    answers: mpsc::UnboundedSender<Answer>,
    opened: u64,
    refused: u64,
}

/// Serves a connection until it ends, then logs why.
async fn serve(
    socket: WebSocket,
    state: AppState,
    closing: CancellationToken,
    client: PageClient,
    session: PageSession,
) {
    let _counted = Counted::connection();
    let opened_at = Instant::now();
    tracing::info!(
        event = "page_connection.opened",
        request_id = %client.request,
        user = %session.user.id,
        page_id = client.page.as_deref(),
        cf_ray = client.ray.as_deref(),
        client = client.client.map(tracing::field::display),
        user_agent = client.user_agent.as_deref(),
        "a page connection opened"
    );
    let tuning = state.services.pages;
    let (sink, mut stream) = socket.split();
    let (to_writer, writes) = mpsc::unbounded_channel();
    let mut writer = tokio::spawn(write(sink, writes, tuning.heartbeat).in_current_span());
    let (to_connection, mut from_lanes) = mpsc::unbounded_channel();
    let (answers, mut answered) = mpsc::unbounded_channel();
    let ended = CancellationToken::new();
    let _ended = ended.clone().drop_guard();
    let watch = state
        .services
        .sync
        .watch_session(&session.user.id, session.token.clone());
    let expiry = session_expiry(state.services.clone(), session.token.clone(), session.expires_at);
    tokio::pin!(expiry);
    let mut connection = Connection {
        state,
        session,
        client,
        lanes: HashMap::new(),
        last_lane: 0,
        to_writer: to_writer.clone(),
        to_connection,
        ended,
        answers,
        opened: 0,
        refused: 0,
    };
    let ending = loop {
        tokio::select! {
            biased;
            () = closing.cancelled() => break Ending::BackendClosing,
            () = watch.marked() => {
                if watch.take().session_ended {
                    break Ending::SessionEnded;
                }
            }
            ending = &mut expiry => break ending,
            _ = &mut writer => break Ending::WriteFailed,
            message = stream.next() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    if let Err(ending) = connection.read(bytes) {
                        break ending;
                    }
                }
                Some(Ok(Message::Text(_))) => break Ending::Protocol("text message"),
                // Pings are answered by the socket itself.
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_)) | Err(_)) | None => break Ending::PageGone,
            },
            Some(told) = from_lanes.recv() => connection.told(told),
            Some(answer) = answered.recv() => connection.answered(answer),
        }
    };
    connection.ended.cancel();
    let open = connection.lanes.len();
    let (opened, refused) = (connection.opened, connection.refused);
    // The opens still checked stop; each owner hears the end of its lane.
    for slot in connection.lanes.into_values() {
        if let Slot::Opening(opening) = slot {
            opening.task.abort();
        }
    }
    let close = ending.close();
    let mut close_late = false;
    if let Some((code, reason)) = close
        && !writer.is_finished()
    {
        // A page that stopped reading holds the close no longer than its
        // bound, behind at most one piece.
        let _ = to_writer.send(Write::Finish { code, reason });
        close_late = tokio::time::timeout(tuning.close_wait, &mut writer)
            .await
            .is_err();
    }
    writer.abort();
    let (close_code, close_reason) = close.unzip();
    macro_rules! ended {
        ($level:ident) => {
            tracing::$level!(
                event = "page_connection.ended",
                request_id = %connection.client.request,
                user = %connection.session.user.id,
                reason = ending.reason(),
                close_code,
                close_reason,
                close_late,
                life_ms = millis(opened_at.elapsed()),
                lanes_opened = opened,
                lanes_refused = refused,
                lanes_open = open,
                "a page connection ended"
            )
        };
    }
    if matches!(ending, Ending::Protocol(_) | Ending::SessionUnread) || close_late {
        ended!(warn);
    } else {
        ended!(info);
    }
}

impl Connection {
    /// Reads one message of the page's; an error is how the page broke the
    /// protocol, which ends the connection.
    fn read(&mut self, bytes: Bytes) -> Result<(), Ending> {
        let Some(&kind) = bytes.first() else {
            return Err(Ending::Protocol("empty message"));
        };
        match kind {
            CONNECTION_FRAME => {
                let message: PageConnectionMessage = serde_json::from_slice(&bytes[1..])
                    .map_err(|_| Ending::Protocol("connection message not understood"))?;
                match message {
                    PageConnectionMessage::Ping { id } => {
                        self.write(Write::Control(connection_frame(&BackendConnectionMessage::Pong { id })));
                    }
                }
                Ok(())
            }
            OPEN_FRAME => {
                let lane = lane_of(&bytes)?;
                self.open(lane, &bytes[5..])
            }
            DATA_FRAME => {
                let lane = lane_of(&bytes)?;
                let Some(&flags) = bytes.get(5) else {
                    return Err(Ending::Protocol("data without flags"));
                };
                self.data(lane, flags, bytes.slice(6..))
            }
            CLOSE_FRAME => {
                let lane = lane_of(&bytes)?;
                let Some(code) = bytes.get(5..7) else {
                    return Err(Ending::Protocol("close without a code"));
                };
                let code = u16::from_be_bytes([code[0], code[1]]);
                let reason = String::from_utf8_lossy(&bytes[7..]).into_owned();
                self.page_closed(lane, code, reason)
            }
            _ => Err(Ending::Protocol("unknown message kind")),
        }
    }

    fn write(&self, write: Write) {
        // A writer that ended ends the connection, which its task's end says.
        let _ = self.to_writer.send(write);
    }

    /// Checks the open of `lane`, which `target` names, in a task of its own.
    fn open(&mut self, lane: u32, target: &[u8]) -> Result<(), Ending> {
        if lane <= self.last_lane {
            return Err(Ending::Protocol("lane number not new"));
        }
        self.last_lane = lane;
        let target = serde_json::from_slice::<LaneTarget>(target);
        let kind = match &target {
            Ok(LaneTarget::Sync) => LaneKind::Sync,
            Ok(LaneTarget::Conversation { .. }) => LaneKind::Conversation,
            Ok(LaneTarget::FileWatch { .. } | LaneTarget::DeviceWatch { .. }) | Err(_) => {
                LaneKind::FileWatch
            }
            Ok(LaneTarget::Signaling { .. }) => LaneKind::Signaling,
            Ok(LaneTarget::UserStream { .. }) => LaneKind::UserStream,
        };
        let request = request_id::generate();
        let span = request_id::lane_span(
            &request,
            kind.name(),
            self.client.page.as_deref(),
            self.client.ray.as_deref(),
            &self.session.user.id,
        );
        let target = match target {
            _ if self.lanes.len() >= MAX_LANES => Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                ErrorCode::TooManyLanes,
                "The connection holds as many lanes as it may",
            )),
            Err(error) => Err(ApiError::invalid_body(format!("The lane's target: {error}"))),
            Ok(target) => Ok(target),
        };
        let target = match target {
            Ok(target) => target,
            Err(error) => {
                let refusal = span.in_scope(|| refusal(error, &request, &self.client.request, lane, kind));
                self.refused += 1;
                self.write(Write::Control(json_frame(REFUSED_FRAME, lane, &refusal)));
                return Ok(());
            }
        };
        let state = self.state.clone();
        let token = self.session.token.clone();
        let answers = self.answers.clone();
        let connection = self.client.request.clone();
        let task = tokio::spawn(
            request_id::scope(request.clone(), async move {
                let opened = check_open(&state, &token, target)
                    .await
                    .map_err(|error| refusal(error, &request, &connection, lane, kind));
                let span = tracing::Span::current();
                // A connection that ended takes no answer.
                let _ = answers.send(Answer {
                    lane,
                    kind,
                    request,
                    span,
                    opened,
                });
            })
            .instrument(span),
        );
        self.lanes.insert(
            lane,
            Slot::Opening(Opening {
                task: task.abort_handle(),
                messages: Vec::new(),
                assembling: None,
                overrun: None,
            }),
        );
        Ok(())
    }

    /// Opens a lane whose check answered, unless the page closed it
    /// meanwhile, or refuses it.
    fn answered(&mut self, answer: Answer) {
        let Answer {
            lane,
            kind,
            request,
            span,
            opened,
        } = answer;
        let Some(Slot::Opening(opening)) = self.lanes.remove(&lane) else {
            return;
        };
        let opened = match opened {
            Ok(opened) => opened,
            Err(refusal) => {
                self.refused += 1;
                self.write(Write::Control(json_frame(REFUSED_FRAME, lane, &refusal)));
                return;
            }
        };
        let origin = LaneOrigin {
            kind,
            lane,
            request,
            connection: self.client.request.clone(),
            user: self.session.user.id.clone(),
            conversation: opened.conversation,
            device: opened.device,
            stream: opened.stream,
        };
        let (owner, reader, to_owner) = span.in_scope(|| open_lane(origin, self.to_connection.clone(), self.ended.clone()));
        self.lanes.insert(
            lane,
            Slot::Open(OpenLane {
                to_owner,
                held: 0,
                assembling: None,
                closed: false,
            }),
        );
        self.opened += 1;
        // Ahead of anything the owner sends, which comes through this task.
        self.write(Write::Control(lane_frame(OPENED_FRAME, lane, &[])));
        tokio::spawn((opened.serve)(owner, reader).instrument(span));
        // What the page sent while the open was checked, ahead of what it
        // sends next, as an open lane takes it.
        for (binary, message) in opening.messages {
            let _ = self.data(lane, if binary { BINARY_FLAG } else { 0 }, message);
        }
        if let Some((code, reason)) = opening.overrun {
            self.close_lane(lane, code, reason);
        }
    }

    /// Takes a piece of a message of `lane`'s.
    fn data(&mut self, lane: u32, flags: u8, piece: Bytes) -> Result<(), Ending> {
        let open = match self.lanes.get_mut(&lane) {
            Some(Slot::Open(open)) => open,
            Some(Slot::Opening(opening)) => {
                opening.hold(flags, piece);
                return Ok(());
            }
            // A message for a lane that ended meanwhile goes nowhere.
            None if lane <= self.last_lane => return Ok(()),
            None => return Err(Ending::Protocol("data for a lane never opened")),
        };
        if open.closed {
            return Ok(());
        }
        let message = match open.assembling.take() {
            None if flags & MORE_FLAG == 0 => piece,
            assembling => {
                let mut whole = assembling.unwrap_or_default();
                if whole.len() + piece.len() > MAX_PAGE_MESSAGE_BYTES {
                    self.close_lane(lane, close_code::SIZE, "message_too_big");
                    return Ok(());
                }
                whole.extend_from_slice(&piece);
                if flags & MORE_FLAG != 0 {
                    open.assembling = Some(whole);
                    return Ok(());
                }
                Bytes::from(whole)
            }
        };
        let message = if flags & BINARY_FLAG != 0 {
            LaneMessage::Binary(message)
        } else {
            match String::from_utf8(message.into()) {
                Ok(text) => LaneMessage::Text(text),
                Err(_) => {
                    self.close_lane(lane, close_code::INVALID, "not_utf8");
                    return Ok(());
                }
            }
        };
        if open.held >= LANE_CREDIT {
            self.close_lane(lane, close_code::POLICY, "credit_exceeded");
            return Ok(());
        }
        open.held += 1;
        // The channel holds the credit and the close; an owner that ended
        // takes nothing more.
        let _ = open.to_owner.try_send(FromConnection::Message(message));
        Ok(())
    }

    /// Closes `lane` for a message of the page's that broke its rules: the
    /// page and the owner both hear the close.
    fn close_lane(&mut self, lane: u32, code: u16, reason: &'static str) {
        let Some(Slot::Open(open)) = self.lanes.get_mut(&lane) else {
            return;
        };
        open.closed = true;
        open.assembling = None;
        let _ = open.to_owner.try_send(FromConnection::Closed {
            code,
            reason: reason.to_owned(),
            by_page: false,
        });
        let frame = close_frame(lane, code, reason);
        self.write(Write::Close { lane, frame });
    }

    /// The page closed `lane`: an open still checked stops, and an open
    /// lane's owner hears the close and nothing more of it is written.
    fn page_closed(&mut self, lane: u32, code: u16, reason: String) -> Result<(), Ending> {
        match self.lanes.get_mut(&lane) {
            Some(Slot::Opening(opening)) => {
                opening.task.abort();
                self.lanes.remove(&lane);
            }
            Some(Slot::Open(open)) => {
                if !open.closed {
                    open.closed = true;
                    open.assembling = None;
                    let _ = open.to_owner.try_send(FromConnection::Closed {
                        code,
                        reason,
                        by_page: true,
                    });
                    self.write(Write::Forget { lane });
                }
            }
            None if lane <= self.last_lane => {}
            None => return Err(Ending::Protocol("close of a lane never opened")),
        }
        Ok(())
    }

    /// Takes what an owner told the connection.
    fn told(&mut self, told: ToConnection) {
        match told {
            ToConnection::Data {
                lane,
                priority,
                message,
                written,
            } => {
                // A lane that closed sends nothing more: dropping `written`
                // tells its owner.
                if matches!(self.lanes.get(&lane), Some(Slot::Open(open)) if !open.closed) {
                    self.write(Write::Data {
                        lane,
                        priority,
                        message,
                        written,
                    });
                }
            }
            ToConnection::Close { lane, code, reason } => {
                if let Some(Slot::Open(open)) = self.lanes.get_mut(&lane)
                    && !open.closed
                {
                    open.closed = true;
                    let frame = close_frame(lane, code, &reason);
                    self.write(Write::Close { lane, frame });
                }
            }
            ToConnection::Credit { lane } => {
                if let Some(Slot::Open(open)) = self.lanes.get_mut(&lane) {
                    open.held = open.held.saturating_sub(1);
                    if !open.closed {
                        self.write(Write::Control(lane_frame(CREDIT_FRAME, lane, &1u16.to_be_bytes())));
                    }
                }
            }
            ToConnection::Ended { lane } => {
                self.lanes.remove(&lane);
                self.write(Write::Forget { lane });
            }
        }
    }
}

/// Checks the open of a lane of `target` as its route checked its upgrade,
/// with the session `token` names as it is now.
async fn check_open(state: &AppState, token: &TokenHash, target: LaneTarget) -> Result<Opened, ApiError> {
    let session = state
        .services
        .sessions
        .check(token)
        .await?
        .ok_or_else(ApiError::unauthenticated)?;
    let user = session.user;
    if user.chooses_password && target != LaneTarget::Sync {
        return Err(ApiError::password_choice_required());
    }
    match target {
        LaneTarget::Sync => Ok(sync::open(
            state,
            PageSession {
                id: session.id,
                token: token.clone(),
                user,
                expires_at: session.expires_at,
            },
        )),
        LaneTarget::Conversation { conversation } => {
            conversations::open_lane(state, &user.id, &conversation).await
        }
        LaneTarget::FileWatch { conversation } => file_watch::open(state, &user.id, &conversation).await,
        LaneTarget::DeviceWatch { device } => file_watch::open_device(state, &user.id, &device).await,
        LaneTarget::Signaling { device } => direct::open(state, &user.id, &device).await,
        LaneTarget::UserStream {
            conversation,
            stream,
        } => streams::open(state, &user.id, &conversation, stream).await,
    }
}

/// The refusal of a lane's open, with its request's ID, and its line.
fn refusal(error: ApiError, request: &RequestId, connection: &RequestId, lane: u32, kind: LaneKind) -> LaneRefusal {
    let (status, mut error) = error.into_body();
    error.request_id = request.clone();
    tracing::info!(
        event = "lane.refused",
        request_id = %request,
        connection = %connection,
        lane,
        kind = kind.name(),
        status = status.as_u16(),
        code = %error.code,
        "a lane's open was refused"
    );
    LaneRefusal {
        status: status.as_u16(),
        error,
    }
}

/// Resolves as the connection's session expires: at the expiry it holds, it
/// looks at the session again, since a request may have renewed it, and
/// waits for the new expiry if one did.
async fn session_expiry(services: Arc<Services>, token: TokenHash, mut expires_at: Timestamp) -> Ending {
    loop {
        let left = expires_at.to_jiff().duration_since(services.clock.now().to_jiff());
        // An expiry already past is due at once.
        tokio::time::sleep(Duration::try_from(left).unwrap_or(Duration::ZERO)).await;
        match services.sessions.check(&token).await {
            Ok(Some(renewed)) if renewed.expires_at > expires_at => expires_at = renewed.expires_at,
            // A session the clock has not yet passed is checked again at the
            // expiry it holds.
            Ok(Some(_)) => {}
            Ok(None) => return Ending::SessionEnded,
            Err(error) => {
                tracing::error!(
                    event = "page_connection.session_read_failed",
                    error = &error as &dyn std::error::Error,
                    "a page connection's session could not be read"
                );
                return Ending::SessionUnread;
            }
        }
    }
}

/// A lane's message the writer is cutting into pieces.
struct Sending {
    lane: u32,
    priority: LanePriority,
    binary: bool,
    bytes: Bytes,
    /// How much of `bytes` went.
    at: usize,
    written: oneshot::Sender<()>,
}

/// The writer: writes what the reader asks in the order the module says
/// until the reader finishes the connection or a write fails.
async fn write(
    mut sink: SplitSink<WebSocket, Message>,
    mut writes: mpsc::UnboundedReceiver<Write>,
    heartbeat: Duration,
) {
    let mut control: VecDeque<Bytes> = VecDeque::new();
    // The interactive lanes' messages, then the user streams'.
    let mut sending: [VecDeque<Sending>; 2] = Default::default();
    let mut wrote_at = Instant::now();
    loop {
        // Everything asked so far is in its queue before the next piece is
        // chosen.
        let mut finish = None;
        loop {
            match writes.try_recv() {
                Ok(Write::Finish { code, reason }) => {
                    finish = Some((code, reason));
                    break;
                }
                Ok(write) => queue(write, &mut control, &mut sending),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if finish.is_none()
            && control.is_empty()
            && sending.iter().all(VecDeque::is_empty)
        {
            tokio::select! {
                write = writes.recv() => match write {
                    Some(Write::Finish { code, reason }) => finish = Some((code, reason)),
                    Some(write) => {
                        queue(write, &mut control, &mut sending);
                        continue;
                    }
                    None => return,
                },
                () = tokio::time::sleep_until(wrote_at + heartbeat) => {
                    control.push_back(connection_frame(&BackendConnectionMessage::Heartbeat));
                }
            }
        }
        if let Some((code, reason)) = finish {
            let frame = CloseFrame {
                code,
                reason: Utf8Bytes::from_static(reason),
            };
            let _ = sink.send(Message::Close(Some(frame))).await;
            return;
        }
        let (frame, written) = match control.pop_front() {
            Some(frame) => (frame, None),
            None => {
                let Some(next) = sending.iter_mut().find_map(VecDeque::pop_front) else {
                    continue;
                };
                let (frame, rest) = piece(next);
                match rest {
                    Ok(rest) => {
                        let at = usize::from(rest.priority == LanePriority::Bulk);
                        sending[at].push_back(rest);
                        (frame, None)
                    }
                    Err(written) => (frame, Some(written)),
                }
            }
        };
        if sink.send(Message::Binary(frame)).await.is_err() {
            return;
        }
        wrote_at = Instant::now();
        if let Some(written) = written {
            // An owner that stopped waiting no longer asks.
            let _ = written.send(());
        }
    }
}

/// Puts what the reader asked in its queue.
fn queue(write: Write, control: &mut VecDeque<Bytes>, sending: &mut [VecDeque<Sending>; 2]) {
    match write {
        Write::Control(frame) => control.push_back(frame),
        Write::Data {
            lane,
            priority,
            message,
            written,
        } => {
            let (binary, bytes) = match message {
                LaneMessage::Text(text) => (false, Bytes::from(text)),
                LaneMessage::Binary(bytes) => (true, bytes),
            };
            let at = usize::from(priority == LanePriority::Bulk);
            sending[at].push_back(Sending {
                lane,
                priority,
                binary,
                bytes,
                at: 0,
                written,
            });
        }
        Write::Close { lane, frame } => {
            for queue in sending.iter_mut() {
                queue.retain(|message| message.lane != lane);
            }
            control.push_back(frame);
        }
        Write::Forget { lane } => {
            for queue in sending.iter_mut() {
                queue.retain(|message| message.lane != lane);
            }
        }
        Write::Finish { .. } => unreachable!("the writer finishes before it queues"),
    }
}

/// The next piece of `message`, and the message with what is left of it, or,
/// when the piece is its last, what tells its owner it went.
fn piece(mut message: Sending) -> (Bytes, Result<Sending, oneshot::Sender<()>>) {
    let end = (message.at + LANE_PIECE_BYTES).min(message.bytes.len());
    let more = end < message.bytes.len();
    let mut flags = 0;
    if message.binary {
        flags |= BINARY_FLAG;
    }
    if more {
        flags |= MORE_FLAG;
    }
    let mut frame = Vec::with_capacity(6 + end - message.at);
    frame.push(DATA_FRAME);
    frame.extend_from_slice(&message.lane.to_be_bytes());
    frame.push(flags);
    frame.extend_from_slice(&message.bytes[message.at..end]);
    message.at = end;
    let rest = if more { Ok(message) } else { Err(message.written) };
    (Bytes::from(frame), rest)
}

/// The lane number of a lane's message, after its kind byte.
fn lane_of(bytes: &[u8]) -> Result<u32, Ending> {
    let lane = bytes
        .get(1..5)
        .ok_or(Ending::Protocol("message without a lane"))?;
    Ok(u32::from_be_bytes([lane[0], lane[1], lane[2], lane[3]]))
}

fn lane_frame(kind: u8, lane: u32, body: &[u8]) -> Bytes {
    let mut frame = Vec::with_capacity(5 + body.len());
    frame.push(kind);
    frame.extend_from_slice(&lane.to_be_bytes());
    frame.extend_from_slice(body);
    Bytes::from(frame)
}

fn json_frame(kind: u8, lane: u32, body: &impl serde::Serialize) -> Bytes {
    // The connection's types serialize their fields as JSON strings, numbers
    // and objects with string keys, which serde_json never refuses.
    lane_frame(kind, lane, &serde_json::to_vec(body).expect("a lane's message serializes"))
}

fn close_frame(lane: u32, code: u16, reason: &str) -> Bytes {
    let mut body = code.to_be_bytes().to_vec();
    body.extend_from_slice(reason.as_bytes());
    lane_frame(CLOSE_FRAME, lane, &body)
}

fn connection_frame(message: &BackendConnectionMessage) -> Bytes {
    let mut frame = vec![CONNECTION_FRAME];
    frame.extend_from_slice(&serde_json::to_vec(message).expect("a connection message serializes"));
    Bytes::from(frame)
}

/// A duration as the integer `_ms` fields of the log hold it.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
