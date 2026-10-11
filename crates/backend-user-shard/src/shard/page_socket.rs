//! A socket to a page: the synchronization channel and the conversation
//! sockets, which the shard serves (`backend.md` § Page synchronization), and
//! the file watch, the direct channel's signaling and the user streams, which
//! the edge serves (`web-api.md` § File watch, § User streams). Each sends its
//! protocol's heartbeat once it has sent nothing else for the heartbeat
//! interval, so that the page can tell a quiet socket from one that died
//! without a close (`web-application.md` § Liveness and reconnection), and
//! each ends with a close frame that a page which stopped reading cannot hold
//! up (`backend.md` § Startup and shutdown).
//!
//! Each socket is logged here, and only here, once as it opens and once as it
//! ends, under an id that ties the two lines, so that a drop a user saw can
//! be traced afterwards (`web-application.md` § Liveness and reconnection).
//! The lines never carry what the socket carried.

use std::net::IpAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, Stream, StreamExt as _};
use tokio::time::{Instant, Sleep};

use crate::tuning::PageTuning;

/// The id of the next page socket, unique within the backend's run.
static NEXT_SOCKET: AtomicU64 = AtomicU64::new(1);

/// What a page socket is for, as its log lines name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageSocketKind {
    Sync,
    Conversation,
    FileWatch,
    DirectSignaling,
    UserStream,
}

impl PageSocketKind {
    fn name(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Conversation => "conversation",
            Self::FileWatch => "file_watch",
            Self::DirectSignaling => "direct_signaling",
            Self::UserStream => "user_stream",
        }
    }
}

/// Who opened a page socket and from where, as its opening line says. The
/// edge reads it from the upgrade request.
#[derive(Debug, Clone)]
pub struct PageSocketOrigin {
    pub kind: PageSocketKind,
    pub user: UserId,
    /// The conversation the socket serves, when it serves one.
    pub conversation: Option<ConversationId>,
    /// The device a signaling socket reaches.
    pub device: Option<DeviceId>,
    /// The user stream's name.
    pub stream: Option<String>,
    /// The edge's request id, Cloudflare's `CF-Ray`, when a proxy passes it.
    pub ray: Option<String>,
    /// The client's address: the one a proxy passes in `CF-Connecting-IP`,
    /// else the connection's own peer; none for a local socket.
    pub client: Option<IpAddr>,
    pub user_agent: Option<String>,
}

/// The sending half of a socket to a page, which knows when it last sent.
pub struct PageSocket {
    sink: SplitSink<WebSocket, Message>,
    tuning: PageTuning,
    /// When the socket last sent a message, or was opened.
    sent_at: Instant,
    /// The messages it sent, heartbeats included.
    sent: u64,
    opened_at: Instant,
    id: u64,
    origin: PageSocketOrigin,
}

/// The receiving half of a socket to a page, which notes when the page last
/// sent anything and the close frame it ended with.
pub struct PageStream {
    stream: SplitStream<WebSocket>,
    /// When the page last sent a message, a ping or a close included, or the
    /// socket opened.
    received_at: Instant,
    /// The page's close, once it came: with its frame, or without one.
    closed: Option<Option<CloseFrame>>,
}

/// A send failed: the page closed the socket, or it broke.
#[derive(Debug)]
pub struct PageGone;

/// How a page socket ended, as its owner tells [`PageSocket::end`].
#[derive(Debug)]
pub enum PageEnd {
    /// The page's side ended: it closed the socket, or the connection broke,
    /// which what the page last sent tells apart.
    Gone,
    /// The backend ends the socket with this close frame.
    Close(CloseFrame),
    /// The backend ends the socket with this close frame for a failure, which
    /// warns: a page that fell behind, or the backend's own error.
    Fail(CloseFrame),
    /// The backend lets the socket go without a close frame, for this reason.
    Dropped(&'static str),
}

/// Opens a page's `socket` from `origin`, and logs that it opened.
pub fn open_page_socket(
    socket: WebSocket,
    tuning: PageTuning,
    origin: PageSocketOrigin,
) -> (PageSocket, PageStream) {
    let id = NEXT_SOCKET.fetch_add(1, Ordering::Relaxed);
    tracing::info!(
        socket = id,
        kind = origin.kind.name(),
        user = %origin.user,
        conversation = origin.conversation.as_ref().map(tracing::field::display),
        device = origin.device.as_ref().map(tracing::field::display),
        stream = origin.stream.as_deref(),
        ray = origin.ray.as_deref(),
        client = origin.client.map(tracing::field::display),
        user_agent = origin.user_agent.as_deref(),
        "a page socket opened"
    );
    let (sink, stream) = socket.split();
    let now = Instant::now();
    let page = PageSocket {
        sink,
        tuning,
        sent_at: now,
        sent: 0,
        opened_at: now,
        id,
        origin,
    };
    let stream = PageStream {
        stream,
        received_at: now,
        closed: None,
    };
    (page, stream)
}

impl PageSocket {
    /// Sends one JSON text message.
    pub async fn send(&mut self, text: String) -> Result<(), PageGone> {
        self.send_message(Message::Text(text.into())).await
    }

    /// Sends one binary message.
    pub async fn send_bytes(&mut self, bytes: Bytes) -> Result<(), PageGone> {
        self.send_message(Message::Binary(bytes)).await
    }

    async fn send_message(&mut self, message: Message) -> Result<(), PageGone> {
        self.sink.send(message).await.map_err(|_| PageGone)?;
        self.sent_at = Instant::now();
        self.sent += 1;
        Ok(())
    }

    /// Resolves once the socket has sent nothing for the heartbeat interval,
    /// when its heartbeat is due.
    pub fn silent(&self) -> Sleep {
        tokio::time::sleep_until(self.sent_at + self.tuning.heartbeat)
    }

    /// Ends the socket as `end` says and logs why. A close frame waits for
    /// the page at most the close's bound: behind it may be the rest of a
    /// message the socket was sending when its owner stopped, which a page
    /// that stopped reading never takes.
    pub async fn end(mut self, from_page: PageStream, end: PageEnd) {
        let mut warn = matches!(end, PageEnd::Fail(_));
        let since_sent = self.sent_at.elapsed();
        let (reason, code, close_reason) = match &end {
            PageEnd::Gone => match &from_page.closed {
                Some(Some(frame)) => ("page closed", Some(frame.code), Some(frame.reason.as_str())),
                Some(None) => ("page closed", None, None),
                None => ("no close frame", None, None),
            },
            PageEnd::Close(frame) | PageEnd::Fail(frame) => {
                ("backend closed", Some(frame.code), Some(frame.reason.as_str()))
            }
            PageEnd::Dropped(why) => (*why, None, None),
        };
        let mut close_late = false;
        if let PageEnd::Close(frame) | PageEnd::Fail(frame) = &end {
            // A page that went meanwhile hears nothing, and dropping the
            // socket ends the connection all the same.
            close_late = tokio::time::timeout(
                self.tuning.close_wait,
                self.sink.send(Message::Close(Some(frame.clone()))),
            )
            .await
            .is_err();
            warn |= close_late;
        }
        macro_rules! ended {
            ($level:ident) => {
                tracing::$level!(
                    socket = self.id,
                    kind = self.origin.kind.name(),
                    user = %self.origin.user,
                    conversation = self.origin.conversation.as_ref().map(tracing::field::display),
                    reason,
                    close_code = code,
                    close_reason,
                    close_late,
                    lived = ?self.opened_at.elapsed(),
                    since_sent = ?since_sent,
                    since_received = ?from_page.received_at.elapsed(),
                    sent = self.sent,
                    "a page socket ended"
                )
            };
        }
        if warn {
            ended!(warn);
        } else {
            ended!(info);
        }
    }
}

impl Stream for PageStream {
    type Item = Result<Message, axum::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let polled = this.stream.poll_next_unpin(cx);
        if let Poll::Ready(Some(Ok(message))) = &polled {
            this.received_at = Instant::now();
            if let Message::Close(frame) = message {
                this.closed = Some(frame.clone());
            }
        }
        polled
    }
}
