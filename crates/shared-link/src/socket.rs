//! A message socket as the bytes yamux runs over.
//!
//! The runner and the backend each hand over their WebSocket as a stream of
//! [`Received`] and a sink of [`Wire`], so this crate knows neither
//! tungstenite nor axum. What yamux writes between two flushes becomes one
//! binary message of at most [`COALESCE_BYTES`], so a message is small enough
//! to cross a slow link in a moment and a ping never waits behind a long
//! one; a message read is the bytes read. Every message that arrives, whatever
//! it is, tells the link its peer is there ([`Heard`]). Besides the bytes, the
//! socket carries how it ends:
//!
//! - a peer that closes over a message of ours it cannot decode says so in
//!   its close frame ([`Received::Close`] with a [`Refusal`]), which ends the
//!   link as [`SocketEnd::Refused`];
//! - an end of ours that refuses a message sends the same close frame
//!   ([`Connection::close`](crate::Connection::close)).

use std::{
    future::Future as _,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use futures_util::{Sink, Stream, ready};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::{Notify, mpsc},
    time::{Instant, Sleep},
};

/// What a socket received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Received {
    /// A binary message.
    Binary(Vec<u8>),
    /// A ping or a pong, which the WebSocket answers by itself.
    Nothing,
    /// The peer closed the socket; with its refusal when it did so over a
    /// message of ours it cannot decode.
    Close(Option<Refusal>),
}

/// A peer's close frame that says it cannot decode a message: its reason
/// names the message and the decoding error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

/// What a socket is given to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wire {
    Binary(Vec<u8>),
    /// A close frame; with a reason when this end refuses a message.
    Close(Option<String>),
}

/// Why a link carries no more messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketEnd {
    /// The socket failed, or the peer broke the protocol.
    Broken(String),
    /// The peer closed it over a message of ours that it cannot decode, and
    /// its close frame says which and why.
    Refused(String),
    /// The peer did not answer a ping within its deadline.
    Unresponsive,
}

/// How reading the socket stopped, kept for whoever asks why the link ended.
pub(crate) type EndSlot = Arc<Mutex<Option<SocketEnd>>>;

/// The most bytes of one binary message.
pub(crate) const COALESCE_BYTES: usize = 32 * 1024;
/// How long a close waits for the peer's close frame.
const PEER_CLOSE_WAIT: Duration = Duration::from_secs(5);

/// When the socket last gave the link a message of any kind, and how many it
/// gave in all: what the link's liveness is judged by.
pub(crate) struct Heard {
    at: Mutex<Instant>,
    count: AtomicU64,
    changed: Notify,
}

impl Heard {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            at: Mutex::new(Instant::now()),
            count: AtomicU64::new(0),
            changed: Notify::new(),
        })
    }

    fn touch(&self) {
        *self.at.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
        self.count.fetch_add(1, Ordering::AcqRel);
        self.changed.notify_waiters();
    }

    /// Counts the time from now as silence, as after a pause.
    pub(crate) fn restart(&self) {
        *self.at.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    pub(crate) fn at(&self) -> Instant {
        *self.at.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn count(&self) -> u64 {
        self.count.load(Ordering::Acquire)
    }

    /// Resolves once more than `seen` messages have arrived.
    pub(crate) async fn after(&self, seen: u64) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.count() > seen {
                return;
            }
            changed.await;
        }
    }
}

/// The socket as `AsyncRead` and `AsyncWrite`, for yamux's connection.
pub(crate) struct SocketIo<R, W> {
    reader: R,
    writer: W,
    /// The bytes of the message being read.
    reading: Vec<u8>,
    offset: usize,
    /// What yamux wrote since the last message was handed to the writer.
    pending: Vec<u8>,
    /// A message was handed to the writer and is not flushed yet.
    unflushed: bool,
    /// A request to end the link over a message this end refuses, with the
    /// reason; it goes out at once, ahead of whatever is queued.
    refusals: mpsc::UnboundedReceiver<String>,
    state: State,
    heard: Arc<Heard>,
    end: EndSlot,
}

enum State {
    Open,
    /// Refusing: the close frame with the reason is yet to be written.
    Refusing(Option<String>),
    RefusingFlush,
    /// Ended over a refusal: reads end and writes fail.
    Ended,
    /// Closing in order: what was written goes, the close frame, and the
    /// peer's.
    CloseFlush,
    CloseSend,
    CloseSent,
    CloseAwait(Pin<Box<Sleep>>),
    Closed,
}

impl<R, W> SocketIo<R, W> {
    pub(crate) fn new(
        reader: R,
        writer: W,
        refusals: mpsc::UnboundedReceiver<String>,
        heard: Arc<Heard>,
        end: EndSlot,
    ) -> Self {
        Self {
            reader,
            writer,
            reading: Vec::new(),
            offset: 0,
            pending: Vec::new(),
            unflushed: false,
            refusals,
            state: State::Open,
            heard,
            end,
        }
    }

    fn ended(&self, end: SocketEnd) {
        self.end
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(end);
    }
}

impl<R, W, E> SocketIo<R, W>
where
    R: Stream<Item = Result<Received, String>> + Unpin,
    W: Sink<Wire, Error = E> + Unpin,
    E: std::fmt::Display,
{
    fn broken(&mut self, error: impl std::fmt::Display) -> io::Error {
        let error = error.to_string();
        self.ended(SocketEnd::Broken(error.clone()));
        self.state = State::Ended;
        io::Error::other(error)
    }

    /// A write failed. The peer may have said why before the socket did:
    /// its refusal is among what it sent, and names the failure better.
    fn write_failed(&mut self, cx: &mut Context<'_>, error: impl std::fmt::Display) -> io::Error {
        loop {
            match Pin::new(&mut self.reader).poll_next(cx) {
                Poll::Ready(Some(Ok(Received::Close(Some(Refusal(reason)))))) => {
                    self.ended(SocketEnd::Refused(reason));
                    break;
                }
                Poll::Ready(Some(Ok(_))) => {}
                Poll::Ready(Some(Err(_)) | None) | Poll::Pending => break,
            }
        }
        self.broken(error)
    }

    /// Hands what yamux wrote to the writer as one message.
    fn poll_hand_over(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.pending.is_empty() {
            return Poll::Ready(Ok(()));
        }
        match Pin::new(&mut self.writer).poll_ready(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(self.write_failed(cx, error))),
            Poll::Ready(Ok(())) => {}
        }
        let message = Wire::Binary(std::mem::take(&mut self.pending));
        if let Err(error) = Pin::new(&mut self.writer).start_send(message) {
            return Poll::Ready(Err(self.write_failed(cx, error)));
        }
        self.unflushed = true;
        Poll::Ready(Ok(()))
    }

    fn poll_flush_writer(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.poll_hand_over(cx))?;
        if self.unflushed {
            match Pin::new(&mut self.writer).poll_flush(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(self.write_failed(cx, error))),
                Poll::Ready(Ok(())) => self.unflushed = false,
            }
        }
        Poll::Ready(Ok(()))
    }

    /// Carries out a refusal once asked: the close frame with its reason,
    /// at once. True once nothing more may be read or written.
    fn poll_state(&mut self, cx: &mut Context<'_>) -> bool {
        if matches!(self.state, State::Open)
            && let Poll::Ready(Some(reason)) = self.refusals.poll_recv(cx)
        {
            self.pending.clear();
            self.state = State::Refusing(Some(reason));
        }
        loop {
            match &mut self.state {
                State::Open => return false,
                State::Refusing(reason) => {
                    match Pin::new(&mut self.writer).poll_ready(cx) {
                        Poll::Pending => return false,
                        // A socket that fails here is gone; the close has
                        // nothing left to say to it.
                        Poll::Ready(Err(_)) => self.state = State::Ended,
                        Poll::Ready(Ok(())) => {
                            let reason = reason.take();
                            let _ = Pin::new(&mut self.writer).start_send(Wire::Close(reason));
                            self.state = State::RefusingFlush;
                        }
                    }
                }
                State::RefusingFlush => match Pin::new(&mut self.writer).poll_flush(cx) {
                    Poll::Pending => return false,
                    Poll::Ready(_) => self.state = State::Ended,
                },
                State::Ended | State::Closed => return true,
                // A close in order is the connection's own, which reads and
                // writes nothing meanwhile.
                State::CloseFlush | State::CloseSend | State::CloseSent | State::CloseAwait(_) => {
                    return false;
                }
            }
        }
    }
}

impl<R, W, E> AsyncRead for SocketIo<R, W>
where
    R: Stream<Item = Result<Received, String>> + Unpin,
    W: Sink<Wire, Error = E> + Unpin,
    E: std::fmt::Display,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if this.poll_state(cx) {
            return Poll::Ready(Ok(()));
        }
        loop {
            if this.offset < this.reading.len() {
                let take = buf.remaining().min(this.reading.len() - this.offset);
                buf.put_slice(&this.reading[this.offset..this.offset + take]);
                this.offset += take;
                return Poll::Ready(Ok(()));
            }
            match Pin::new(&mut this.reader).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(Received::Binary(bytes)))) => {
                    this.heard.touch();
                    this.reading = bytes;
                    this.offset = 0;
                }
                Poll::Ready(Some(Ok(Received::Nothing))) => this.heard.touch(),
                Poll::Ready(Some(Ok(Received::Close(Some(Refusal(reason)))))) => {
                    this.heard.touch();
                    this.ended(SocketEnd::Refused(reason));
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "the peer refused a message",
                    )));
                }
                Poll::Ready(Some(Ok(Received::Close(None))) | None) => return Poll::Ready(Ok(())),
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Err(this.broken(error))),
            }
        }
    }
}

impl<R, W, E> AsyncWrite for SocketIo<R, W>
where
    R: Stream<Item = Result<Received, String>> + Unpin,
    W: Sink<Wire, Error = E> + Unpin,
    E: std::fmt::Display,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        this.poll_state(cx);
        if !matches!(this.state, State::Open) {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        if this.pending.len() >= COALESCE_BYTES {
            ready!(this.poll_hand_over(cx))?;
        }
        let take = buf.len().min(COALESCE_BYTES - this.pending.len());
        this.pending.extend_from_slice(&buf[..take]);
        Poll::Ready(Ok(take))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        this.poll_state(cx);
        match this.state {
            State::Open => this.poll_flush_writer(cx),
            // Nothing is written once the link is refused or closed.
            _ => Poll::Ready(Ok(())),
        }
    }

    /// Closes in order: what yamux wrote, a close frame, then the peer's
    /// close frame for a moment, so the peer sees this end leave.
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            match &mut this.state {
                State::Open => this.state = State::CloseFlush,
                State::CloseFlush => {
                    if ready!(this.poll_flush_writer(cx)).is_err() {
                        this.state = State::Closed;
                        return Poll::Ready(Ok(()));
                    }
                    this.state = State::CloseSend;
                }
                State::CloseSend => match Pin::new(&mut this.writer).poll_ready(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(_)) => this.state = State::Closed,
                    Poll::Ready(Ok(())) => {
                        let _ = Pin::new(&mut this.writer).start_send(Wire::Close(None));
                        this.state = State::CloseSent;
                    }
                },
                State::CloseSent => match Pin::new(&mut this.writer).poll_flush(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(_) => {
                        this.state = State::CloseAwait(Box::pin(tokio::time::sleep(PEER_CLOSE_WAIT)));
                    }
                },
                State::CloseAwait(sleep) => {
                    if sleep.as_mut().poll(cx).is_ready() {
                        this.state = State::Closed;
                        continue;
                    }
                    match Pin::new(&mut this.reader).poll_next(cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Some(Ok(Received::Binary(_) | Received::Nothing))) => {}
                        Poll::Ready(_) => this.state = State::Closed,
                    }
                }
                State::Refusing(_) | State::RefusingFlush | State::Ended | State::Closed => {
                    return Poll::Ready(Ok(()));
                }
            }
        }
    }
}
