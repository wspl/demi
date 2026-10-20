//! A socket to a page: the synchronization channel and the conversation sockets,
//! which the shard serves, and the file watch, the direct channel's signaling and
//! the user streams, which the edge serves. Each sends its protocol's heartbeat once
//! it has sent nothing else for the heartbeat interval, so that the page can tell a
//! quiet socket from one that died without a close, and each ends with a close frame
//! that a page which stopped reading cannot hold up
//! (`crates/backend/src/backend.rs`). Each but a user stream, which carries bytes
//! alone, answers the page's `{ ping: id }` with `pong` at once, ahead of any
//! message its owner has yet to send, whatever the owner is doing meanwhile: the
//! page asks so as it returns, to learn whether the socket is still alive. The ping
//! never reaches the owner.
//!
//! So that a pong never waits for the owner, one writer task per socket
//! sends everything the socket sends: the owner's messages, one at a time
//! and each awaited, and the pongs the receiving half reads, which go first
//! at each message's end. The task ends with the socket's sending half, the
//! close's bound included, whether the owner ends it or is dropped.
//!
//! Each socket is logged here, and only here, once as it opens and once as it
//! ends, under the request ID of its upgrade, which ties the two lines, so
//! that a drop a user saw can be traced afterwards. The lines never carry what
//! the socket carried.

use std::collections::VecDeque;
use std::net::IpAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use demi_conversation_socket_protocol::PagePing;
use demi_shared_types::RequestId;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, Stream, StreamExt as _};
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, Sleep};
use tokio_util::task::AbortOnDropHandle;

use crate::tuning::PageTuning;

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

    /// Whether the page pings the socket: every one but a user stream, whose
    /// text messages are refused.
    fn answers_pings(self) -> bool {
        self != Self::UserStream
    }
}

/// Who opened a page socket and from where, as its opening line says. The
/// edge reads it from the upgrade request.
#[derive(Debug, Clone)]
pub struct PageSocketOrigin {
    pub kind: PageSocketKind,
    /// The request ID of the socket's upgrade, which the socket keeps for
    /// its whole life.
    pub request: RequestId,
    pub user: UserId,
    /// The conversation the socket serves, when it serves one.
    pub conversation: Option<ConversationId>,
    /// The device a signaling socket reaches.
    pub device: Option<DeviceId>,
    /// The user stream's name.
    pub stream: Option<String>,
    /// Cloudflare's request id, its `CF-Ray`, when a proxy passes it.
    pub ray: Option<String>,
    /// The client's address: the one a proxy passes in `CF-Connecting-IP`,
    /// else the connection's own peer; none for a local socket.
    pub client: Option<IpAddr>,
    pub user_agent: Option<String>,
}

/// One of the owner's messages for the writer, with where it says whether
/// the message went.
type Outgoing = (Message, oneshot::Sender<Result<(), PageGone>>);

/// The sending half of a socket to a page, which knows when it last sent.
pub struct PageSocket {
    /// The writer's queue of the owner's messages; the owner awaits each.
    to_writer: mpsc::Sender<Outgoing>,
    /// Ends the writer when the socket is dropped, as when its owner was
    /// cancelled in the middle of a send to a page that stopped reading.
    writer: AbortOnDropHandle<()>,
    tuning: PageTuning,
    /// When the socket last sent a message, or was opened.
    sent_at: Instant,
    /// The messages it sent, heartbeats included.
    sent: u64,
    opened_at: Instant,
    origin: PageSocketOrigin,
}

/// The receiving half of a socket to a page, which notes when the page last
/// sent anything and the close frame it ended with, and answers the page's
/// pings as it reads them.
pub struct PageStream {
    stream: SplitStream<WebSocket>,
    /// When the page last sent a message, a ping or a close included, or the
    /// socket opened.
    received_at: Instant,
    /// The page's close, once it came: with its frame, or without one.
    closed: Option<Option<CloseFrame>>,
    /// Whether the page's `{ ping: id }` is answered here.
    answers_pings: bool,
    /// The ids of the pings read, for the writer to answer.
    pongs: mpsc::Sender<u32>,
    /// What was read while the owner was busy, in order, for its next reads
    /// ([`Self::next_unless_busy`]); the end of the page's side last, if it came.
    held: VecDeque<Option<Result<Message, axum::Error>>>,
}

/// The answer to the page's ping `id`, as every page socket's protocol has
/// it: `{"type":"pong","id":id}`.
#[derive(Serialize)]
#[serde(tag = "type", rename = "pong")]
struct Pong {
    id: u32,
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
    tracing::info!(
        request_id = %origin.request,
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
    let (to_writer, messages) = mpsc::channel(1);
    let (pongs, pings) = mpsc::channel(MAX_PINGS);
    let writer = AbortOnDropHandle::new(tokio::spawn(write(sink, messages, pings)));
    let now = Instant::now();
    let stream = PageStream {
        stream,
        received_at: now,
        closed: None,
        answers_pings: origin.kind.answers_pings(),
        pongs,
        held: VecDeque::new(),
    };
    let page = PageSocket {
        to_writer,
        writer,
        tuning,
        sent_at: now,
        sent: 0,
        opened_at: now,
        origin,
    };
    (page, stream)
}

impl PageSocket {
    /// The request ID of the socket's upgrade.
    pub fn request(&self) -> &RequestId {
        &self.origin.request
    }

    /// Sends one JSON text message.
    pub async fn send(&mut self, text: String) -> Result<(), PageGone> {
        self.send_message(Message::Text(text.into())).await
    }

    /// Sends one binary message.
    pub async fn send_bytes(&mut self, bytes: Bytes) -> Result<(), PageGone> {
        self.send_message(Message::Binary(bytes)).await
    }

    async fn send_message(&mut self, message: Message) -> Result<(), PageGone> {
        self.write(message).await?;
        self.sent_at = Instant::now();
        self.sent += 1;
        Ok(())
    }

    /// Has the writer send `message`, and resolves once it went.
    async fn write(&self, message: Message) -> Result<(), PageGone> {
        let (sent, went) = oneshot::channel();
        // A writer that ended did so as a send failed.
        self.to_writer.send((message, sent)).await.map_err(|_| PageGone)?;
        went.await.map_err(|_| PageGone)?
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
    pub async fn end(self, from_page: PageStream, end: PageEnd) {
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
                self.write(Message::Close(Some(frame.clone()))),
            )
            .await
            .is_err();
            warn |= close_late;
        }
        macro_rules! ended {
            ($level:ident) => {
                tracing::$level!(
                    request_id = %self.origin.request,
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
        // Dropping the socket ends the writer, wherever it waits.
        drop(self.writer);
    }
}

/// The socket's writer: sends the pongs due and the owner's messages, a
/// pong first whenever both wait, until the owner's queue closes or a send
/// fails. Each of the owner's messages is answered with whether it went.
async fn write(
    mut sink: SplitSink<WebSocket, Message>,
    mut messages: mpsc::Receiver<Outgoing>,
    mut pings: mpsc::Receiver<u32>,
) {
    let mut reading = true;
    loop {
        tokio::select! {
            biased;
            ping = pings.recv(), if reading => match ping {
                Some(id) => {
                    let pong = serde_json::to_string(&Pong { id }).expect("a pong serializes");
                    // A page gone meanwhile hears nothing; the owner's next
                    // send finds the writer ended.
                    if sink.send(Message::Text(pong.into())).await.is_err() {
                        return;
                    }
                }
                // The receiving half is gone; the owner may still send.
                None => reading = false,
            },
            message = messages.recv() => {
                let Some((message, sent)) = message else {
                    return;
                };
                let went = sink.send(message).await.map_err(|_| PageGone);
                let failed = went.is_err();
                // An owner that stopped waiting no longer asks.
                let _ = sent.send(went);
                if failed {
                    return;
                }
            }
        }
    }
}

/// The most pings a socket holds unanswered: a page sends one as it returns
/// and waits for its pong, so more are a page that does not follow the
/// protocol, whose further pings go unanswered.
const MAX_PINGS: usize = 16;

/// The most messages a socket reads ahead while its owner is busy: a page
/// sends a few while one is handled, such as a steer behind an upload. Past
/// them it reads no more, its pings included, until the owner reads: the
/// page's messages may each be a megabyte.
const MAX_HELD: usize = 8;

/// Whether `read` is the end of the page's side, after which nothing is read.
fn ends(read: &Option<Result<Message, axum::Error>>) -> bool {
    !matches!(read, Some(Ok(message)) if !matches!(message, Message::Close(_)))
}

impl PageStream {
    /// The page's next message; while `busy`, as while a conversation
    /// socket handles the last one, none: it reads on meanwhile, answering
    /// the page's pings at once, and holds the other messages in their
    /// order for the reads once it is not busy.
    pub async fn next_unless_busy(&mut self, busy: bool) -> Option<Result<Message, axum::Error>> {
        if !busy {
            return self.next().await;
        }
        std::future::poll_fn(|cx| {
            while self.held.len() < MAX_HELD && !self.held.back().is_some_and(ends) {
                match self.poll_read(cx) {
                    Poll::Ready(read) => self.held.push_back(read),
                    Poll::Pending => break,
                }
            }
            // Never ready: what it holds waits for a read that is not busy,
            // which the owner's next turn makes.
            Poll::<Option<Result<Message, axum::Error>>>::Pending
        })
        .await
    }

    /// Reads the page's next message other than a ping, which it answers.
    fn poll_read(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Message, axum::Error>>> {
        loop {
            let polled = self.stream.poll_next_unpin(cx);
            let Poll::Ready(Some(Ok(message))) = &polled else {
                return polled;
            };
            self.received_at = Instant::now();
            match message {
                Message::Close(frame) => self.closed = Some(frame.clone()),
                Message::Text(text) if self.answers_pings => {
                    if let Ok(PagePing { ping }) = serde_json::from_str::<PagePing>(text.as_str()) {
                        // Past `MAX_PINGS` unanswered, or once the writer
                        // ended with the socket, the ping goes unanswered.
                        let _ = self.pongs.try_send(ping);
                        continue;
                    }
                }
                _ => {}
            }
            return polled;
        }
    }
}

impl Stream for PageStream {
    type Item = Result<Message, axum::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Some(held) = this.held.pop_front() {
            return Poll::Ready(held);
        }
        this.poll_read(cx)
    }
}
