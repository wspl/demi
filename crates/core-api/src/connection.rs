//! `WS /api/connect`: a page's connection, and the lanes it carries
//! (`crates/web-api-protocol/src/connection.rs`).
//!
//! The mount resolves the caller of the upgrade: the server from the session
//! cookie, without renewing the session, since only requests renew it. The
//! connection then runs in two tasks:
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
//!   the caller as it is now, and is refused with the status and error that
//!   route answered; an account still to choose its password opens only the
//!   synchronization channel. One open never waits for another.
//! - The page sends a lane's messages behind its open without waiting for
//!   `opened`: what comes while the open is checked is held, within the
//!   lane's credit, and its owner takes it first once the lane opens; a
//!   refused open drops it.
//! - An opened lane is served by its owner: the user's core serves the
//!   synchronization channel and a conversation's lane, the edge the rest.
//! - The writer writes the connection's own messages and every lane's
//!   opened, refused, close and credit first, then one piece of each lane
//!   with a message to send in turn, a user stream's after the others', so a
//!   large message holds another lane back by one piece at most. Once it has
//!   written nothing for the heartbeat interval it writes a `heartbeat`.
//! - The connection ends with its caller, as a server's session is signed
//!   out or expires, with `session_ended`; as the backend shuts down, with
//!   `backend_closing`; and as the page breaks the protocol, with
//!   `protocol_error`. Every lane ends with it at once: what the page sent
//!   that the lane's owner has not taken is dropped, unread.
//!
//! # Logging
//!
//! The connection logs one line as it opens and one as it ends, under the
//! request ID of its upgrade; each lane logs its own (`crates/backend-user-
//! core/src/shard/lane.rs`) under the request ID of its open, in a span of
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
use demi_core::lane::{
    Counted, FromConnection, Lane, LaneKind, LaneMessage, LaneOrigin, LanePriority, LaneReader,
    ToConnection, open_lane,
};
use demi_shared_types::RequestId;
use demi_web_api_protocol::MAX_PAGE_MESSAGE_BYTES;
use demi_shared_lanes::{Assembly, AssemblyError, Frame};
use demi_web_api_protocol::connection::{
    BackendConnectionMessage, LANE_CREDIT, LaneRefusal, LaneTarget, MAX_LANES, PageConnectionMessage,
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

use crate::api::Api;
use crate::caller::{Caller, CallerOf, Gone};
use crate::error::ApiError;
use crate::page_client::PageClient;
use crate::request_id;
use crate::{conversations, direct, file_watch, streams, sync};

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
pub struct Closing(pub CancellationToken);

/// How the process times a page's connection. Tests shorten the times.
#[derive(Debug, Clone, Copy)]
pub struct PageTuning {
    /// A connection that has written nothing for this long writes a
    /// heartbeat, so that its page can tell it from a dead one.
    pub heartbeat: Duration,
    /// How long a connection's close frame waits for a page that does not
    /// read; a page that has not taken it by then loses the connection
    /// without it (`crates/backend/src/backend.rs`).
    pub close_wait: Duration,
}

impl Default for PageTuning {
    fn default() -> Self {
        Self {
            heartbeat: Duration::from_secs(30),
            close_wait: Duration::from_secs(1),
        }
    }
}

/// What serves an opened lane until it ends.
pub(crate) type Serve = Box<dyn FnOnce(Lane, LaneReader) -> BoxFuture<'static, ()> + Send>;

/// A lane its checks let open: what serves it, and what it serves, which its
/// log lines name.
pub(crate) struct Opened {
    pub(crate) serve: Serve,
    pub(crate) conversation: Option<ConversationId>,
    pub(crate) device: Option<DeviceId>,
    pub(crate) stream: Option<String>,
}

impl Opened {
    pub(crate) fn new<F, Fut>(serve: F) -> Self
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

/// `WS /api/connect?page=<id>`: a request with no caller, as one whose
/// session is missing or ended, answers 401 before the upgrade.
pub async fn connect(
    State(state): State<Api>,
    Extension(Closing(closing)): Extension<Closing>,
    CallerOf(caller): CallerOf,
    client: PageClient,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    tracing::Span::current().record("user_id", tracing::field::display(caller.principal()));
    let upgrade = upgrade.map_err(|_| {
        ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            ErrorCode::UpgradeRequired,
            "The page connection is a WebSocket",
        )
    })?;
    let span = tracing::Span::current();
    Ok(upgrade
        .max_message_size(MAX_CONNECTION_MESSAGE_BYTES)
        .max_frame_size(MAX_CONNECTION_MESSAGE_BYTES)
        .on_upgrade(move |socket| serve(socket, state, closing, client, caller).instrument(span)))
}

/// How a connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// The page closed the connection, or it broke.
    PageGone,
    /// A write to the page failed.
    WriteFailed,
    BackendClosing,
    /// The caller left, as a server's session was signed out or expired.
    SessionEnded,
    /// What the caller is could not be read, as a server's session at its
    /// expiry.
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
    /// The whole messages the page sent.
    messages: Vec<LaneMessage>,
    /// The message whose pieces are still coming.
    assembling: Assembly,
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
        match self.assembling.take(flags, piece, MAX_PAGE_MESSAGE_BYTES) {
            Ok(Some(message)) => self.messages.push(lane_message(message)),
            Ok(None) => {}
            Err(error) => self.overrun = Some(assembly_close(error)),
        }
    }
}

fn lane_message(message: demi_shared_lanes::Message) -> LaneMessage {
    match message {
        demi_shared_lanes::Message::Text(text) => LaneMessage::Text(text),
        demi_shared_lanes::Message::Binary(bytes) => LaneMessage::Binary(bytes),
    }
}

/// The lane's close for a message of the page's that could not be put
/// together.
fn assembly_close(error: AssemblyError) -> (u16, &'static str) {
    match error {
        AssemblyError::TooBig => (close_code::SIZE, "message_too_big"),
        AssemblyError::NotUtf8 => (close_code::INVALID, "not_utf8"),
    }
}

struct OpenLane {
    /// Brings the owner the page's messages and the lane's close.
    to_owner: mpsc::Sender<FromConnection>,
    /// The page's messages the owner has not taken yet, at most the credit.
    held: u16,
    /// The pieces of the page's message that more pieces complete.
    assembling: Assembly,
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
    state: Api,
    caller: Arc<dyn Caller>,
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
    state: Api,
    closing: CancellationToken,
    client: PageClient,
    caller: Arc<dyn Caller>,
) {
    let _counted = Counted::connection();
    let opened_at = Instant::now();
    tracing::info!(
        event = "page_connection.opened",
        request_id = %client.request,
        user = %caller.principal(),
        page_id = client.page.as_deref(),
        cf_ray = client.ray.as_deref(),
        client = client.client.map(tracing::field::display),
        user_agent = client.user_agent.as_deref(),
        "a page connection opened"
    );
    let tuning = state.pages;
    let (sink, mut stream) = socket.split();
    let (to_writer, writes) = mpsc::unbounded_channel();
    let mut writer = tokio::spawn(write(sink, writes, tuning.heartbeat).in_current_span());
    let (to_connection, mut from_lanes) = mpsc::unbounded_channel();
    let (answers, mut answered) = mpsc::unbounded_channel();
    let ended = CancellationToken::new();
    let _ended = ended.clone().drop_guard();
    let gone = caller.gone();
    tokio::pin!(gone);
    let mut connection = Connection {
        state,
        caller,
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
            gone = &mut gone => break match gone {
                Gone::Ended => Ending::SessionEnded,
                Gone::Unread => Ending::SessionUnread,
            },
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
                user = %connection.caller.principal(),
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
        match Frame::decode(bytes).map_err(Ending::Protocol)? {
            Frame::Connection(json) => {
                let message: PageConnectionMessage = serde_json::from_slice(&json)
                    .map_err(|_| Ending::Protocol("connection message not understood"))?;
                match message {
                    PageConnectionMessage::Ping { id } => {
                        self.write(Write::Control(connection_frame(&BackendConnectionMessage::Pong { id })));
                    }
                }
                Ok(())
            }
            Frame::Open { lane, target } => self.open(lane, &target),
            Frame::Data { lane, flags, piece } => self.data(lane, flags, piece),
            Frame::Close { lane, code, reason } => self.page_closed(lane, code, reason),
            Frame::Opened { .. } | Frame::Refused { .. } | Frame::Credit { .. } => {
                Err(Ending::Protocol("unknown message kind"))
            }
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
            Ok(LaneTarget::Starter) => LaneKind::Starter,
        };
        let request = request_id::generate();
        let span = request_id::lane_span(
            &request,
            kind.name(),
            self.client.page.as_deref(),
            self.client.ray.as_deref(),
            self.caller.principal(),
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
                self.write(Write::Control(refused_frame(lane, &refusal)));
                return Ok(());
            }
        };
        let state = self.state.clone();
        let caller = self.caller.clone();
        let answers = self.answers.clone();
        let connection = self.client.request.clone();
        let task = tokio::spawn(
            request_id::scope(request.clone(), async move {
                let opened = check_open(&state, &caller, target)
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
                assembling: Assembly::default(),
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
                self.write(Write::Control(refused_frame(lane, &refusal)));
                return;
            }
        };
        let origin = LaneOrigin {
            kind,
            lane,
            request,
            connection: self.client.request.clone(),
            user: self.caller.principal().clone(),
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
                assembling: Assembly::default(),
                closed: false,
            }),
        );
        self.opened += 1;
        // Ahead of anything the owner sends, which comes through this task.
        self.write(Write::Control(Frame::Opened { lane }.encode()));
        tokio::spawn((opened.serve)(owner, reader).instrument(span));
        // What the page sent while the open was checked, ahead of what it
        // sends next, as an open lane takes it.
        for message in opening.messages {
            self.deliver(lane, message);
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
        match open.assembling.take(flags, piece, MAX_PAGE_MESSAGE_BYTES) {
            Ok(Some(message)) => self.deliver(lane, lane_message(message)),
            Ok(None) => {}
            Err(error) => {
                let (code, reason) = assembly_close(error);
                self.close_lane(lane, code, reason);
            }
        }
        Ok(())
    }

    /// Hands a whole message of open `lane`'s to its owner, within its
    /// credit.
    fn deliver(&mut self, lane: u32, message: LaneMessage) {
        let Some(Slot::Open(open)) = self.lanes.get_mut(&lane) else {
            return;
        };
        if open.closed {
            return;
        }
        if open.held >= LANE_CREDIT {
            self.close_lane(lane, close_code::POLICY, "credit_exceeded");
            return;
        }
        open.held += 1;
        // The channel holds the credit and the close; an owner that ended
        // takes nothing more.
        let _ = open.to_owner.try_send(FromConnection::Message(message));
    }

    /// Closes `lane` for a message of the page's that broke its rules: the
    /// page and the owner both hear the close.
    fn close_lane(&mut self, lane: u32, code: u16, reason: &'static str) {
        let Some(Slot::Open(open)) = self.lanes.get_mut(&lane) else {
            return;
        };
        open.closed = true;
        open.assembling.clear();
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
                    open.assembling.clear();
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
                        self.write(Write::Control(Frame::Credit { lane, messages: 1 }.encode()));
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
/// with `caller` as it is now.
async fn check_open(state: &Api, caller: &Arc<dyn Caller>, target: LaneTarget) -> Result<Opened, ApiError> {
    let caller = caller.opens(&target).await?;
    let user = caller.principal().clone();
    match target {
        LaneTarget::Sync => Ok(sync::open(state, caller)),
        LaneTarget::Conversation { conversation } => {
            conversations::open_lane(state, &user, &conversation).await
        }
        LaneTarget::FileWatch { conversation } => file_watch::open(state, &user, &conversation).await,
        LaneTarget::DeviceWatch { device } => file_watch::open_device(state, &user, &device).await,
        LaneTarget::Signaling { device } => direct::open(state, &user, &device).await,
        LaneTarget::UserStream {
            conversation,
            stream,
        } => streams::open(state, &user, &conversation, stream).await,
        LaneTarget::Starter => {
            let lane = caller
                .starter_lane()
                .ok_or_else(|| ApiError::forbidden("Only the program that opened the core opens its starter lane"))?;
            let claim = lane.claim().ok_or_else(|| {
                ApiError::new(
                    StatusCode::CONFLICT,
                    ErrorCode::StarterLaneOpen,
                    "The core's starter lane is open already",
                )
            })?;
            Ok(Opened::new(move |lane, reader| claim.serve(lane, reader)))
        }
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
    let rest = message.bytes.slice(message.at..);
    let first = demi_shared_lanes::pieces(message.lane, message.binary, rest)
        .next()
        .expect("a message has a first piece");
    let Frame::Data { piece, .. } = &first else {
        unreachable!("a message's pieces are data frames")
    };
    message.at += piece.len();
    let more = message.at < message.bytes.len();
    let rest = if more { Ok(message) } else { Err(message.written) };
    (first.encode(), rest)
}

fn refused_frame(lane: u32, refusal: &LaneRefusal) -> Bytes {
    // A refusal serializes its fields as JSON strings, numbers and objects
    // with string keys, which serde_json never refuses.
    let refusal = Bytes::from(serde_json::to_vec(refusal).expect("a lane's refusal serializes"));
    Frame::Refused { lane, refusal }.encode()
}

fn close_frame(lane: u32, code: u16, reason: &str) -> Bytes {
    Frame::Close {
        lane,
        code,
        reason: reason.to_owned(),
    }
    .encode()
}

fn connection_frame(message: &BackendConnectionMessage) -> Bytes {
    let json = serde_json::to_vec(message).expect("a connection message serializes");
    Frame::Connection(Bytes::from(json)).encode()
}

/// A duration as the integer `_ms` fields of the log hold it.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
