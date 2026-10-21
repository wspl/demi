//! One link: a yamux connection over a socket, its streams, and its liveness.

use std::{
    collections::VecDeque,
    future::{Future as _, poll_fn},
    io,
    pin::Pin,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};

use demi_runner_protocol::wire::{Lane, RUNNER_PING_INTERVAL, RUNNER_PING_TIMEOUT};
use futures_util::Stream;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _, ReadHalf, WriteHalf},
    sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    time::{Instant, Sleep},
};
use tokio_util::{
    compat::FuturesAsyncReadCompatExt as _,
    sync::CancellationToken,
    task::TaskTracker,
};

use crate::{
    framing,
    lanes::Lanes,
    outgoing::Outgoing,
    socket::{EndSlot, Heard, SocketEnd},
};

/// The bytes of messages, read and not yet taken, that this end holds for
/// every stream together.
pub(crate) const MESSAGE_BUDGET: usize = 32 * 1024 * 1024;
/// The messages that wait for the application beyond the budget's bytes.
pub(crate) const MESSAGE_QUEUE: usize = 8;

/// A message waiting for the application, with what it holds.
pub(crate) type Delivery = (Vec<u8>, Held);

/// The messages of one stream the application may hold, taken from
/// [`Incoming`] and not yet let go of, before the link stops reading that
/// stream: its window then fills, and its sender waits, and no other stream
/// waits with it.
pub(crate) const STREAM_AHEAD: usize = 8;

/// What a message holds of the link until the application lets go of it: its
/// bytes of the budget and its place among the messages of its stream the
/// application may hold. An application that cannot use a message yet
/// keeps it, as the runner does a command's input until the command reads it,
/// and the link reads no further of the stream meanwhile.
pub struct Held {
    pub(crate) _budget: Option<OwnedSemaphorePermit>,
    pub(crate) _stream: Option<OwnedSemaphorePermit>,
}

impl Held {
    /// Holds nothing: for a message that came by another way than the link.
    pub fn nothing() -> Self {
        Self {
            _budget: None,
            _stream: None,
        }
    }
}

/// How long closing the link may take to send what is queued and to hear the
/// peer's close.
pub(crate) const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the messages a link received before it ended may take to reach
/// the application.
const DELIVER_TIMEOUT: Duration = Duration::from_secs(10);
/// The streams this end has open to its peer at once, for commands and
/// requests; opening one more waits until one ends.
pub(crate) const OUTBOUND_STREAMS: usize = 1024;
/// The streams the peer may have open to this end at once, besides its
/// connection's and its heartbeat: a peer that opens more is broken.
pub(crate) const INBOUND_STREAMS: usize = OUTBOUND_STREAMS + 2;
/// Streams yamux admits: both ends' streams, with the two each end opens for
/// itself.
pub(crate) const MAX_STREAMS: usize = OUTBOUND_STREAMS + INBOUND_STREAMS + 2;
/// The window every stream starts with, as yamux's specification has it.
pub(crate) const WINDOW_BYTES: usize = 256 * 1024;
/// The window a command's stream receives with from its first byte: its
/// first output or input of this size goes in a round trip, as before the
/// streams, and a stream reaches the rate of a gigabit through a 100 ms
/// round trip without growing. The window is what the stream may leave
/// unread, so past it the sender waits for the reader.
pub(crate) const WINDOW_COMMAND: u32 = 16 * 1024 * 1024;
/// What the windows of all streams may be larger than [`WINDOW_BYTES`] by, at
/// their start and by growth: the first windows of four commands at once, or
/// one stream's window grown past its first. A command opened when this is
/// spent starts with what is left, down to [`WINDOW_BYTES`], and its window
/// grows only as the stream is read, so a peer cannot grow the window of a
/// stream nobody reads.
pub(crate) const WINDOW_GROWTH: usize = 64 * 1024 * 1024;

/// Which end of the link this is; yamux needs the two to tell their streams
/// apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The runner, which opens the connection.
    Runner,
    /// The backend, which accepts it.
    Backend,
}

/// How each end checks that the other is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Liveness {
    /// How often a probe goes.
    pub interval: Duration,
    /// How long nothing from the peer, whatever it is, may go on before the
    /// link counts as lost.
    pub timeout: Duration,
}

impl Default for Liveness {
    fn default() -> Self {
        Self {
            interval: RUNNER_PING_INTERVAL,
            timeout: RUNNER_PING_TIMEOUT,
        }
    }
}

/// An open link: what it sends, what it receives and how it fares.
pub struct Connection {
    pub outgoing: Outgoing,
    pub incoming: Incoming,
    pub health: Health,
}

/// The messages the other end sent, as the encoded bytes of each, until the
/// link ends: with `None` when it ended gracefully, with the reason
/// otherwise. Messages of one stream arrive in the order they were sent;
/// those of different streams arrive in whatever order they were read.
pub struct Incoming {
    pub(crate) messages: mpsc::Receiver<Result<Delivery, SocketEnd>>,
    pub(crate) _owner: Arc<Owner>,
}

/// What the users of a link hold together: the link ends, as a socket does,
/// once the last of them is dropped (its [`Outgoing`] clones, its
/// [`Incoming`] and its [`Health`]). What the streams queued still goes first.
/// The link's own tasks hold the link without holding it open.
pub(crate) struct Owner(pub(crate) Arc<Shared>);

impl std::ops::Deref for Owner {
    type Target = Arc<Shared>;

    fn deref(&self) -> &Arc<Shared> {
        &self.0
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        let shared = self.0.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move { shared.close(None).await });
            }
            // Without a runtime nothing can send what is queued.
            Err(_) => shared.closed.cancel(),
        }
    }
}

impl Stream for Incoming {
    type Item = Result<Vec<u8>, SocketEnd>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.messages
            .poll_recv(cx)
            .map(|next| next.map(|item| item.map(|(message, _held)| message)))
    }
}

impl Incoming {
    /// The next message, which holds its place in its stream until its
    /// [`Held`] is dropped: the link reads at most a few more of the
    /// stream's messages meanwhile. Taking messages with `next` of
    /// [`Stream`] lets each go at once.
    pub async fn next_held(&mut self) -> Option<Result<(Vec<u8>, Held), SocketEnd>> {
        self.messages.recv().await
    }
}

/// The link's liveness: a probe on demand, and a pause while the machine
/// that runs this end or the other is copied.
#[derive(Clone)]
pub struct Health(pub(crate) HealthKind);

#[derive(Clone)]
pub(crate) enum HealthKind {
    Link(Arc<Owner>),
    /// A link of a test that has no peer to probe.
    Inert,
}

impl Health {
    /// A health that answers every probe, for a test of a link's user.
    pub fn inert() -> Self {
        Self(HealthKind::Inert)
    }

    /// Whether the other end answers a probe within `within`: whether any
    /// message arrives from it in that time, the probe's answer or anything
    /// else, as a healthy link that is busy with data answers. A paused link
    /// answers: its silence is no death.
    pub async fn answers(&self, within: Duration) -> bool {
        let HealthKind::Link(shared) = &self.0 else {
            return true;
        };
        // A link that is not running yet is being set up, not lost.
        if shared.paused.load(Ordering::Acquire) || !shared.started.load(Ordering::Acquire) {
            return true;
        }
        let seen = shared.heard.count();
        shared.probe_now.notify_one();
        tokio::select! {
            heard = tokio::time::timeout(within, shared.heard.after(seen)) => heard.is_ok(),
            () = shared.closed.cancelled() => false,
        }
    }

    /// Stops probing, and counting the silence, while a checkpoint copies the
    /// machine of either end.
    pub fn pause(&self) {
        if let HealthKind::Link(shared) = &self.0 {
            shared.paused.store(true, Ordering::Release);
        }
    }

    /// Probes again, counting the silence from now.
    pub fn resume(&self) {
        if let HealthKind::Link(shared) = &self.0 {
            shared.heard.restart();
            shared.paused.store(false, Ordering::Release);
        }
    }

    /// How many records the link keeps of commands and requests: what a
    /// long-lived link must not accumulate.
    #[cfg(feature = "testing")]
    pub fn records(&self) -> usize {
        match &self.0 {
            HealthKind::Link(shared) => shared.records(),
            HealthKind::Inert => 0,
        }
    }
}

pub(crate) type YStream = yamux::Stream;
/// A yamux stream as tokio reads and writes it.
pub(crate) type Compat = tokio_util::compat::Compat<YStream>;
pub(crate) type ReadSide = EndingRead<ReadHalf<Compat>>;
pub(crate) type WriteSide = WriteHalf<Compat>;

/// The read half of a stream, which reads what the stream still holds once
/// the link is over and then ends: yamux does not wake a reader of a stream
/// whose connection was dropped after a close, and nothing more can arrive.
pub(crate) struct EndingRead<T> {
    inner: T,
    over: Pin<Box<tokio_util::sync::WaitForCancellationFutureOwned>>,
}

impl<T> EndingRead<T> {
    pub(crate) fn new(inner: T, shared: &Shared) -> Self {
        Self {
            inner,
            over: Box::pin(shared.closed.clone().cancelled_owned()),
        }
    }

    pub(crate) fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for EndingRead<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Pending if self.over.as_mut().poll(cx).is_ready() => Poll::Ready(Ok(())),
            other => other,
        }
    }
}

pub(crate) enum Command {
    /// Opens a stream whose receive window starts at the given size, or at
    /// yamux's own.
    Open(oneshot::Sender<io::Result<YStream>>, Option<u32>),
    /// Close in order: the connection says goodbye after the frames its
    /// streams handed over.
    Close,
}

/// What the link's tasks share.
pub(crate) struct Shared {
    pub(crate) liveness: Option<Liveness>,
    pub(crate) commands: mpsc::UnboundedSender<Command>,
    /// Ends the link at once over a message this end refuses.
    pub(crate) refusals: mpsc::UnboundedSender<String>,
    pub(crate) lanes: Mutex<Lanes>,
    pub(crate) next_token: AtomicU64,
    /// The bytes of messages read and not yet taken that the link may hold.
    pub(crate) budget: Arc<Semaphore>,
    /// The messages for the application; none once the link ended.
    pub(crate) messages: Mutex<Option<mpsc::Sender<Result<Delivery, SocketEnd>>>>,
    /// Cancelled once the link is over: nothing sent reaches the peer.
    pub(crate) closed: CancellationToken,
    /// Cancelled when this end closes the link: streams send what they queue
    /// and end.
    pub(crate) closing: CancellationToken,
    /// Stops the readers that did not finish.
    pub(crate) abort: CancellationToken,
    /// The streams' writers, which a close lets send what is queued.
    pub(crate) tasks: TaskTracker,
    /// The streams' readers, which a graceful end lets deliver what the
    /// connection received.
    pub(crate) readers: TaskTracker,
    /// The streams this end may still open.
    pub(crate) outbound: Arc<Semaphore>,
    /// The streams the peer has open to this end.
    pub(crate) inbound: AtomicUsize,
    pub(crate) paused: AtomicBool,
    pub(crate) heard: Arc<Heard>,
    pub(crate) probe_now: Notify,
    /// Whether the session runs: a link before its handshake is over holds
    /// what is sent on it, and ends at once when it is dropped.
    pub(crate) started: AtomicBool,
    pub(crate) end: EndSlot,
}

/// What an open stream holds for as long as either of its halves lives: the
/// right to have it, given back when the last half is dropped.
pub(crate) struct StreamLife {
    _outbound: Option<OwnedSemaphorePermit>,
    inbound: Option<Arc<Shared>>,
}

impl StreamLife {
    /// A stream this end opened, with its permit when it counts.
    pub(crate) fn opened(permit: Option<OwnedSemaphorePermit>) -> Arc<Self> {
        Arc::new(Self {
            _outbound: permit,
            inbound: None,
        })
    }

    /// A stream the peer opened.
    pub(crate) fn accepted(shared: &Arc<Shared>) -> Arc<Self> {
        Arc::new(Self {
            _outbound: None,
            inbound: Some(shared.clone()),
        })
    }
}

impl Drop for StreamLife {
    fn drop(&mut self) {
        if let Some(shared) = &self.inbound {
            shared.inbound.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

enum Outcome {
    /// Closed in order, by either end.
    Graceful,
    /// The peer did not answer for its whole deadline.
    Unresponsive,
    Failed(String),
}

/// The liveness check: it fires when the peer was last heard a whole timeout
/// ago, and a check that fires long after it was due means this end was
/// not running (a machine copied or asleep), which is no silence of the
/// peer's.
struct Monitor {
    sleep: Pin<Box<Sleep>>,
    due: Instant,
    timeout: Duration,
}

impl Monitor {
    fn new(timeout: Duration) -> Self {
        let due = Instant::now() + timeout;
        Self {
            sleep: Box::pin(tokio::time::sleep_until(due)),
            due,
            timeout,
        }
    }

    /// Checks the peer once the check is due. It runs after the connection
    /// has read what arrived, so a message that waited while this end did not
    /// run counts before the silence does.
    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
        heard: &Heard,
        paused: bool,
    ) -> Option<Outcome> {
        loop {
            if self.sleep.as_mut().poll(cx).is_pending() {
                return None;
            }
            let now = Instant::now();
            let late = now.saturating_duration_since(self.due);
            let next = if paused || late >= self.timeout {
                heard.restart();
                now + self.timeout
            } else {
                let silent_until = heard.at() + self.timeout;
                if silent_until <= now {
                    return Some(Outcome::Unresponsive);
                }
                silent_until
            };
            self.due = next;
            self.sleep.as_mut().reset(next);
        }
    }
}

/// Runs the connection, serving each stream the other end opens and opening

/// Runs the connection, serving each stream the other end opens and opening
/// those the senders ask for, until it ends.
pub(crate) async fn drive<T>(
    shared: Arc<Shared>,
    mut connection: yamux::Connection<T>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) where
    T: futures_util::AsyncRead + futures_util::AsyncWrite + Unpin + Send + 'static,
{
    type Opening = (oneshot::Sender<io::Result<YStream>>, Option<u32>);
    let mut opening: VecDeque<Opening> = VecDeque::new();
    let mut closing = false;
    let mut monitor = shared.liveness.map(|liveness| Monitor::new(liveness.timeout));
    let closed = shared.closed.clone();
    let mut aborted = Box::pin(closed.cancelled());
    let outcome = poll_fn(|cx| {
        loop {
            match commands.poll_recv(cx) {
                Poll::Ready(Some(Command::Open(reply, window))) => opening.push_back((reply, window)),
                Poll::Ready(Some(Command::Close)) => closing = true,
                Poll::Ready(None) | Poll::Pending => break,
            }
        }
        if closing {
            match connection.poll_close(cx) {
                Poll::Ready(Ok(())) => return Poll::Ready(Outcome::Graceful),
                Poll::Ready(Err(error)) => return Poll::Ready(Outcome::Failed(error.to_string())),
                Poll::Pending => {}
            }
        } else {
            while let Some((reply, window)) = opening.front() {
                if reply.is_closed() {
                    opening.pop_front();
                    continue;
                }
                let opened = match window {
                    Some(window) => connection.poll_new_outbound_with_window(cx, *window),
                    None => connection.poll_new_outbound(cx),
                };
                match opened {
                    Poll::Ready(Ok(stream)) => {
                        if let Some((reply, _)) = opening.pop_front() {
                            let _ = reply.send(Ok(stream));
                        }
                    }
                    Poll::Ready(Err(error)) => {
                        if let Some((reply, _)) = opening.pop_front() {
                            let _ = reply.send(Err(io::Error::other(error.to_string())));
                        }
                    }
                    Poll::Pending => break,
                }
            }
            loop {
                match connection.poll_next_inbound(cx) {
                    Poll::Ready(Some(Ok(stream))) => crate::lanes::accept(&shared, stream),
                    Poll::Ready(Some(Err(error))) => {
                        return Poll::Ready(Outcome::Failed(error.to_string()));
                    }
                    Poll::Ready(None) => return Poll::Ready(Outcome::Graceful),
                    Poll::Pending => break,
                }
            }
        }
        if aborted.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Outcome::Failed("the link was dropped".into()));
        }
        if let Some(monitor) = &mut monitor
            && let Some(outcome) = monitor.poll(cx, &shared.heard, shared.paused.load(Ordering::Acquire))
        {
            return Poll::Ready(outcome);
        }
        Poll::Pending
    })
    .await;
    drop(connection);
    for (reply, _) in opening {
        let _ = reply.send(Err(io::ErrorKind::BrokenPipe.into()));
    }
    let recorded = shared
        .end
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let graceful = matches!(outcome, Outcome::Graceful) && recorded.is_none();
    let end = match (outcome, recorded) {
        (_, Some(end)) => Some(end),
        (Outcome::Unresponsive, None) => Some(SocketEnd::Unresponsive),
        (Outcome::Failed(error), None) => Some(SocketEnd::Broken(error)),
        (Outcome::Graceful, None) => None,
    };
    shared.closed.cancel();
    // What the connection received before a graceful end is still read from
    // its streams, and reaches the application before the end does.
    if graceful {
        shared.readers.close();
        let _ = tokio::time::timeout(DELIVER_TIMEOUT, shared.readers.wait()).await;
    }
    shared.abort.cancel();
    let messages = shared
        .messages
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    if let (Some(messages), Some(end)) = (messages, end) {
        let _ = messages.send(Err(end)).await;
    }
}

/// Sends a probe each interval and when asked, and answers the peer's.
///
/// A probe is a byte on a stream of its own, which the peer echoes: ordinary
/// traffic of the link, so a proxy cannot answer it for the peer, and its turn
/// on the wire comes from the connection's fairness among streams, not
/// behind the data of any one.
pub(crate) async fn heartbeat(shared: Arc<Shared>) {
    let Ok(stream) = shared.open_raw(None).await else {
        return;
    };
    let (mut read, mut write) = tokio::io::split(stream.compat());
    let header = framing::lane_header(&Lane::heartbeat(), false);
    if write.write_all(&header).await.is_err() {
        return;
    }
    let draining = shared.clone();
    tokio::spawn(async move {
        let mut buffer = [0u8; 64];
        loop {
            tokio::select! {
                () = draining.abort.cancelled() => return,
                read = read.read(&mut buffer) => if !matches!(read, Ok(1..)) { return },
            }
        }
    });
    loop {
        let timed = async {
            match shared.liveness {
                Some(liveness) => tokio::time::sleep(liveness.interval).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = shared.closed.cancelled() => return,
            () = timed => {}
            () = shared.probe_now.notified() => {}
        }
        let sent = tokio::select! {
            () = shared.closed.cancelled() => return,
            sent = write.write_all(&[1]) => sent,
        };
        if sent.is_err() {
            return;
        }
    }
}

impl Shared {
    /// A stream could not be read to its end.
    pub(crate) fn read_failed(self: &Arc<Self>, error: &io::Error) {
        if error.kind() == io::ErrorKind::InvalidData {
            // A frame the protocol does not allow: the peer is not one of
            // this version's, or is broken.
            self.violation(error.to_string());
        }
    }

    /// The peer broke the protocol: the link ends over it, and its peer is
    /// told why.
    pub(crate) fn violation(self: &Arc<Self>, reason: String) {
        self.end
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(SocketEnd::Broken(reason.clone()));
        let closing = self.clone();
        tokio::spawn(async move { closing.close(Some(reason)).await });
    }

    /// Ends the link on this end's word. In order, when `refusal` is none:
    /// what the streams queued is sent, then the connection says goodbye
    /// after the frames its streams hold. Over a message this end refuses: at
    /// once, with the reason in the close frame, since the peer is broken.
    pub(crate) async fn close(&self, refusal: Option<String>) {
        if self.closed.is_cancelled() {
            return;
        }
        if !self.started.load(Ordering::Acquire) {
            // No session ever ran, so nothing was sent and nothing waits.
            self.closed.cancel();
            return;
        }
        self.closing.cancel();
        self.tasks.close();
        match refusal {
            Some(reason) => {
                let _ = self.refusals.send(reason);
            }
            None => {
                if tokio::time::timeout(CLOSE_TIMEOUT, self.tasks.wait())
                    .await
                    .is_err()
                {
                    tracing::debug!("the link's streams did not drain before it closed");
                }
                let _ = self.commands.send(Command::Close);
            }
        }
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, self.closed.cancelled()).await;
        self.closed.cancel();
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.is_cancelled() || self.closing.is_cancelled()
    }

    #[cfg(feature = "testing")]
    pub(crate) fn records(&self) -> usize {
        self.lanes.lock().unwrap_or_else(PoisonError::into_inner).records()
    }

    /// Opens a stream to the peer once the link has room for one, when it
    /// counts: the permit comes back with it. Fails once the link is over.
    pub(crate) async fn open_stream(
        &self,
        window: Option<u32>,
        counted: bool,
    ) -> io::Result<(YStream, Option<OwnedSemaphorePermit>)> {
        let permit = if counted {
            let permit = tokio::select! {
                () = self.closed.cancelled() => return Err(io::ErrorKind::BrokenPipe.into()),
                permit = self.outbound.clone().acquire_owned() => permit,
            };
            Some(permit.map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?)
        } else {
            None
        };
        Ok((self.open_raw(window).await?, permit))
    }

    /// Opens a stream that starts with `header` and goes on with whatever
    /// the caller writes, as a peer that breaks the protocol would.
    #[cfg(feature = "testing")]
    pub(crate) async fn rogue(&self, header: &[u8]) -> io::Result<WriteSide> {
        let stream = self.open_raw(None).await?;
        let (_read, mut write) = tokio::io::split(stream.compat());
        write.write_all(header).await?;
        Ok(write)
    }

    pub(crate) async fn open_raw(&self, window: Option<u32>) -> io::Result<YStream> {
        let (reply, opened) = oneshot::channel();
        self.commands
            .send(Command::Open(reply, window))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        tokio::select! {
            () = self.closed.cancelled() => Err(io::ErrorKind::BrokenPipe.into()),
            opened = opened => opened.map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?,
        }
    }
}
