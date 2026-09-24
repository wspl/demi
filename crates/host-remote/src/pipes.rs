//! Pipe records (`runner.md` § Pipes and output): every byte stream between
//! processes. A pipe has a source and a sink; an end on a device is that
//! runner's HTTP exchange with the backend (`PUT` from the source, `GET` by
//! the sink), an end in this process is a reader or a writer. The records
//! live in the user's shard and hold only the byte in flight; the ends are
//! `Send`, so the edge that serves a device's request copies the bytes.
//!
//! An end may be named after the pipe is minted: an rpc call's pipes are
//! minted before its handler decides whether it reads and writes them or
//! hands them to a job elsewhere. A pipe whose ends have not both arrived
//! within the arrival window fails.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fmt::Display,
    future::{Future, pending},
    io,
    pin::Pin,
    rc::{Rc, Weak},
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use demi_runner_protocol::wire::PipeRef;
use futures_util::{Sink, Stream, StreamExt, stream::BoxStream};
use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};
use tokio_util::{sync::PollSender, task::TaskTracker};

/// How long a pipe waits for its ends.
pub const ARRIVAL: Duration = Duration::from_secs(120);

/// The pipes of one user's devices.
#[derive(Clone)]
pub struct Pipes(Rc<Inner>);

struct Inner {
    slots: RefCell<HashMap<Arc<str>, Slot>>,
    arrival: Duration,
    /// One task per pipe: its arrival window and its removal once settled.
    tasks: TaskTracker,
    closed: Cell<bool>,
}

struct Slot {
    source: End,
    sink: End,
    source_arrived: bool,
    sink_arrived: bool,
    /// The byte channel's ends, until the ends that use them arrive.
    sender: Option<mpsc::Sender<Bytes>>,
    receiver: Option<mpsc::Receiver<Bytes>>,
    core: Arc<Core>,
    deadline: watch::Sender<Option<Instant>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum End {
    /// Not named yet.
    Open,
    /// This process: a reader or a writer, or a source held for one.
    Process,
    Device(String),
}

/// What the ends of one pipe share: its outcome, and whether a source that
/// sends bytes is there, a writer or a device's `PUT` (a held source is
/// not).
struct Core {
    state: watch::Sender<Outcome>,
    source_arrived: watch::Sender<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Open,
    /// The sink read the source to its end, or stopped reading early.
    Drained,
    Failed(Arc<str>),
}

/// Why a pipe failed; the text begins `pipe failed: `.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct PipeFailure(Arc<str>);

/// A failed pipe, to a reader or writer of bytes that fails with an IO
/// error.
impl From<PipeFailure> for io::Error {
    fn from(failure: PipeFailure) -> Self {
        io::Error::other(failure)
    }
}

/// Why a device's request cannot claim a pipe end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PipeRefusal {
    /// No such pipe, the pipe is over, or the end belongs to another device.
    #[error("no such pipe")]
    NotFound,
    #[error("the end is already connected")]
    AlreadyConnected,
}

/// Why this process cannot take a pipe end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PipeError {
    #[error("the pipe is over")]
    Settled,
    #[error("the pipe's {0} is already fixed")]
    AlreadyFixed(&'static str),
    #[error("the pipe's {0} is already taken")]
    Taken(&'static str),
}

impl Core {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: watch::Sender::new(Outcome::Open),
            source_arrived: watch::Sender::new(false),
        })
    }

    /// Fails the pipe for both ends; false when it was over already.
    fn fail(&self, reason: &str) -> bool {
        self.state.send_if_modified(|state| {
            if *state != Outcome::Open {
                return false;
            }
            *state = Outcome::Failed(format!("pipe failed: {reason}").into());
            true
        })
    }

    fn drain(&self) {
        self.state.send_if_modified(|state| {
            if *state != Outcome::Open {
                return false;
            }
            *state = Outcome::Drained;
            true
        });
    }

    fn is_open(&self) -> bool {
        *self.state.borrow() == Outcome::Open
    }

    fn failure(&self) -> Option<PipeFailure> {
        match &*self.state.borrow() {
            Outcome::Failed(reason) => Some(PipeFailure(reason.clone())),
            _ => None,
        }
    }

    /// Why a writer cannot go on: the pipe failed, or its sink is gone.
    fn stopped(&self) -> PipeFailure {
        self.failure()
            .unwrap_or_else(|| PipeFailure("pipe failed: the sink stopped reading".into()))
    }

    /// Resolves once the pipe is over: drained, or failed with why.
    fn done(&self) -> impl Future<Output = Result<(), PipeFailure>> + Send + 'static {
        let mut state = self.state.subscribe();
        async move {
            // The core owns the sender while anyone subscribes.
            let outcome = state
                .wait_for(|state| *state != Outcome::Open)
                .await
                .map(|outcome| outcome.clone())
                .unwrap_or(Outcome::Drained);
            match outcome {
                Outcome::Failed(reason) => Err(PipeFailure(reason)),
                _ => Ok(()),
            }
        }
    }

    /// Resolves once the pipe failed.
    fn failed(&self) -> impl Future<Output = PipeFailure> + Send + 'static {
        let mut state = self.state.subscribe();
        async move {
            let failed = state
                .wait_for(|state| matches!(state, Outcome::Failed(_)))
                .await
                .map(|outcome| outcome.clone());
            match failed {
                Ok(Outcome::Failed(reason)) => PipeFailure(reason),
                // The core owns the sender while anyone subscribes.
                _ => pending().await,
            }
        }
    }
}

impl Pipes {
    pub fn new(arrival: Duration) -> Self {
        Self(Rc::new(Inner {
            slots: RefCell::default(),
            arrival,
            tasks: TaskTracker::new(),
            closed: Cell::new(false),
        }))
    }

    /// A pipe whose source is `source`'s runner or, when none, named later;
    /// likewise its sink.
    pub fn mint(&self, source: Option<&str>, sink: Option<&str>) -> Pipe {
        let id: Arc<str> = uuid::Uuid::new_v4().simple().to_string().into();
        let core = Core::new();
        let (sender, receiver) = mpsc::channel(1);
        let deadline = watch::Sender::new(Some(Instant::now() + self.0.arrival));
        let end =
            |device: Option<&str>| device.map_or(End::Open, |device| End::Device(device.into()));
        let watching = deadline.subscribe();
        self.0.slots.borrow_mut().insert(
            id.clone(),
            Slot {
                source: end(source),
                sink: end(sink),
                source_arrived: false,
                sink_arrived: false,
                sender: Some(sender),
                receiver: Some(receiver),
                core: core.clone(),
                deadline,
            },
        );
        self.0.tasks.spawn_local(watch_arrival(
            Rc::downgrade(&self.0),
            id.clone(),
            core.clone(),
            watching,
        ));
        if self.0.closed.get() {
            core.fail("backend shutting down");
        }
        Pipe {
            id,
            pipes: self.clone(),
            core,
        }
    }

    /// A pipe `device`'s runner fills and this process reads.
    pub fn from_device(&self, device: &str) -> Pipe {
        self.mint(Some(device), None)
    }

    /// A pipe this process fills and `device`'s runner reads.
    pub fn to_device(&self, device: &str) -> Pipe {
        self.mint(None, Some(device))
    }

    /// The pipe `id` names, while its record lives: a call's relayed pipe,
    /// for a handler that hands one of its ends to a job elsewhere.
    pub fn pipe(&self, id: &str) -> Option<Pipe> {
        let slots = self.0.slots.borrow();
        let slot = slots.get(id)?;
        Some(Pipe {
            id: id.into(),
            pipes: self.clone(),
            core: slot.core.clone(),
        })
    }

    /// The source end of pipe `id` for `device`'s `PUT`.
    pub fn claim_source(&self, id: &str, device: &str) -> Result<DeviceSource, PipeRefusal> {
        let mut slots = self.0.slots.borrow_mut();
        let slot = slots.get_mut(id).ok_or(PipeRefusal::NotFound)?;
        if slot.source != End::Device(device.into()) || !slot.core.is_open() {
            return Err(PipeRefusal::NotFound);
        }
        let sender = slot.sender.take().ok_or(PipeRefusal::AlreadyConnected)?;
        slot.source_arrived = true;
        slot.core.source_arrived.send_replace(true);
        slot.update_deadline(self.0.arrival);
        Ok(DeviceSource {
            writer: PipeWriter {
                sender: Some(sender),
                core: slot.core.clone(),
                abandoned: "source HTTP request disconnected",
            },
        })
    }

    /// The sink end of pipe `id` for `device`'s `GET`.
    pub fn claim_sink(&self, id: &str, device: &str) -> Result<DeviceSink, PipeRefusal> {
        let mut slots = self.0.slots.borrow_mut();
        let slot = slots.get_mut(id).ok_or(PipeRefusal::NotFound)?;
        if slot.sink != End::Device(device.into()) || !slot.core.is_open() {
            return Err(PipeRefusal::NotFound);
        }
        let receiver = slot.receiver.take().ok_or(PipeRefusal::AlreadyConnected)?;
        slot.sink_arrived = true;
        slot.update_deadline(self.0.arrival);
        Ok(DeviceSink {
            reader: PipeReader {
                receiver,
                core: slot.core.clone(),
                ended: false,
                abandoned: Abandoned::Fail("sink HTTP request disconnected"),
            },
        })
    }

    pub fn fail(&self, id: &str, reason: &str) {
        if let Some(slot) = self.0.slots.borrow().get(id) {
            slot.core.fail(reason);
        }
    }

    /// A device's report that its end failed. It counts only from a device at
    /// one of the pipe's ends; returns whether it ended the pipe, which is
    /// false for a pipe already over, whose failure on the device is its
    /// consequence.
    pub fn fail_from_device(&self, id: &str, device: &str, reason: &str) -> bool {
        let slots = self.0.slots.borrow();
        let Some(slot) = slots.get(id) else {
            return false;
        };
        let party = End::Device(device.into());
        (slot.source == party || slot.sink == party) && slot.core.fail(reason)
    }

    /// Fails every pipe `device` is an end of: its runner went away.
    pub fn device_gone(&self, device: &str) {
        let party = End::Device(device.into());
        let reason = format!("device {device} disconnected");
        for slot in self.0.slots.borrow().values() {
            if slot.source == party || slot.sink == party {
                slot.core.fail(&reason);
            }
        }
    }

    /// Fails every pipe, and every pipe minted from now on, and waits until
    /// each has let go of its record.
    pub async fn close(&self) {
        self.0.closed.set(true);
        let cores: Vec<_> = self
            .0
            .slots
            .borrow()
            .values()
            .map(|slot| slot.core.clone())
            .collect();
        for core in cores {
            core.fail("backend shutting down");
        }
        self.0.tasks.close();
        self.0.tasks.wait().await;
    }

    fn with_slot<T>(
        &self,
        id: &str,
        change: impl FnOnce(&mut Slot, Duration) -> Result<T, PipeError>,
    ) -> Result<T, PipeError> {
        let mut slots = self.0.slots.borrow_mut();
        let slot = slots.get_mut(id).ok_or(PipeError::Settled)?;
        if !slot.core.is_open() {
            return Err(PipeError::Settled);
        }
        change(slot, self.0.arrival)
    }
}

impl Slot {
    /// The arrival window runs while an end is missing: it stops once both
    /// arrived, and starts again when an end is named anew.
    fn update_deadline(&mut self, arrival: Duration) {
        if self.source_arrived && self.sink_arrived {
            self.deadline.send_replace(None);
        } else if self.deadline.borrow().is_none() {
            self.deadline.send_replace(Some(Instant::now() + arrival));
        }
    }
}

/// Fails the pipe when its ends do not arrive in time, and forgets it once
/// it is over.
async fn watch_arrival(
    pipes: Weak<Inner>,
    id: Arc<str>,
    core: Arc<Core>,
    mut deadline: watch::Receiver<Option<Instant>>,
) {
    let done = core.done();
    tokio::pin!(done);
    loop {
        let due = *deadline.borrow_and_update();
        let expired = async {
            match due {
                Some(due) => tokio::time::sleep_until(due).await,
                None => pending().await,
            }
        };
        tokio::select! {
            _ = &mut done => break,
            () = expired => {
                core.fail("an end never arrived");
            }
            changed = deadline.changed() => if changed.is_err() {
                break;
            },
        }
    }
    if let Some(pipes) = pipes.upgrade() {
        pipes.slots.borrow_mut().remove(&id);
    }
}

/// A pipe as this process holds it.
#[derive(Clone)]
pub struct Pipe {
    id: Arc<str>,
    pipes: Pipes,
    core: Arc<Core>,
}

impl Pipe {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The pipe as the runner wire names it: the id its end reports
    /// `pipe_done` under, and the origin-relative URL of its HTTP exchange.
    pub fn wire_ref(&self) -> PipeRef {
        PipeRef {
            id: self.id.to_string(),
            url: format!("/api/pipes/{}", self.id),
        }
    }

    /// Makes this process the sink: the pipe's bytes as they come.
    pub fn reader(&self) -> Result<PipeReader, PipeError> {
        self.pipes.with_slot(&self.id, |slot, arrival| {
            match slot.sink {
                End::Open => slot.sink = End::Process,
                End::Process => {}
                End::Device(_) => return Err(PipeError::AlreadyFixed("sink")),
            }
            let receiver = slot.receiver.take().ok_or(PipeError::Taken("sink"))?;
            slot.sink_arrived = true;
            slot.update_deadline(arrival);
            Ok(PipeReader {
                receiver,
                core: slot.core.clone(),
                ended: false,
                abandoned: Abandoned::Drain,
            })
        })
    }

    /// Makes this process the source: a writer whose writes wait until the
    /// sink takes them.
    pub fn writer(&self) -> Result<PipeWriter, PipeError> {
        self.pipes.with_slot(&self.id, |slot, arrival| {
            match slot.source {
                End::Open => slot.source = End::Process,
                End::Process => {}
                End::Device(_) => return Err(PipeError::AlreadyFixed("source")),
            }
            let sender = slot.sender.take().ok_or(PipeError::Taken("source"))?;
            slot.source_arrived = true;
            slot.core.source_arrived.send_replace(true);
            slot.update_deadline(arrival);
            Ok(PipeWriter {
                sender: Some(sender),
                core: slot.core.clone(),
                abandoned: "the writer went away before the end",
            })
        })
    }

    /// This process owns the source, which counts as arrived, until it takes
    /// the writer or hands the source to a device: an rpc call's standard
    /// output, whose handler may never write or may hand it on.
    pub fn hold_source(&self) -> Result<(), PipeError> {
        self.pipes.with_slot(&self.id, |slot, arrival| {
            if slot.source != End::Open {
                return Err(PipeError::AlreadyFixed("source"));
            }
            slot.source_arrived = true;
            slot.update_deadline(arrival);
            Ok(())
        })
    }

    /// Names a device that will `GET` the pipe as its sink.
    pub fn sink_to(&self, device: &str) -> Result<(), PipeError> {
        self.pipes.with_slot(&self.id, |slot, arrival| {
            if slot.sink != End::Open {
                return Err(PipeError::AlreadyFixed("sink"));
            }
            slot.sink = End::Device(device.into());
            slot.sink_arrived = false;
            slot.update_deadline(arrival);
            Ok(())
        })
    }

    /// Names a device that will `PUT` the pipe as its source, taking over a
    /// source this process held.
    pub fn source_from(&self, device: &str) -> Result<(), PipeError> {
        self.pipes.with_slot(&self.id, |slot, arrival| {
            if slot.source != End::Open {
                return Err(PipeError::AlreadyFixed("source"));
            }
            slot.source = End::Device(device.into());
            slot.source_arrived = false;
            slot.update_deadline(arrival);
            Ok(())
        })
    }

    /// Ends the pipe for both ends; its readers and writer see `reason`.
    pub fn fail(&self, reason: &str) {
        self.core.fail(reason);
    }

    /// Resolves once the pipe is over: its sink drained it, or why it failed.
    pub fn done(&self) -> impl Future<Output = Result<(), PipeFailure>> + Send + 'static {
        self.core.done()
    }

    /// Why the pipe failed, when it has.
    pub fn failure(&self) -> Option<PipeFailure> {
        self.core.failure()
    }

    /// Resolves only if the pipe fails.
    pub fn failed(&self) -> impl Future<Output = PipeFailure> + Send + 'static {
        self.core.failed()
    }
}

/// What dropping a reader before the end means.
#[derive(Debug, Clone, Copy)]
enum Abandoned {
    /// A reader in this process that stops early drained the pipe, as a
    /// closed pipe would.
    Drain,
    /// A device's response cut short fails the pipe.
    Fail(&'static str),
}

/// The sink's side of a pipe. Dropping it before the end drains the pipe.
pub struct PipeReader {
    receiver: mpsc::Receiver<Bytes>,
    core: Arc<Core>,
    ended: bool,
    abandoned: Abandoned,
}

impl PipeReader {
    /// The next chunk; none at the source's end.
    pub async fn next(&mut self) -> Option<Result<Bytes, PipeFailure>> {
        if self.ended {
            return None;
        }
        let failed = self.core.failed();
        tokio::select! {
            biased;
            failure = failed => {
                self.ended = true;
                Some(Err(failure))
            }
            chunk = self.receiver.recv() => match chunk {
                Some(bytes) => Some(Ok(bytes)),
                None => {
                    self.ended = true;
                    // A writer fails the pipe before it lets go of its end.
                    match self.core.failure() {
                        Some(failure) => Some(Err(failure)),
                        None => {
                            self.core.drain();
                            None
                        }
                    }
                }
            },
        }
    }

    /// Ends reading: the pipe fails with `reason` unless it is over.
    pub fn fail(mut self, reason: &str) {
        self.ended = true;
        self.core.fail(reason);
    }

    pub fn into_stream(self) -> BoxStream<'static, Result<Bytes, PipeFailure>> {
        futures_util::stream::unfold(self, |mut reader| async move {
            reader.next().await.map(|item| (item, reader))
        })
        .boxed()
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        match self.abandoned {
            Abandoned::Drain => self.core.drain(),
            Abandoned::Fail(reason) => {
                self.core.fail(reason);
            }
        }
    }
}

/// The source's side of a pipe. It must end: dropping it before
/// [`PipeWriter::end`] fails the pipe, so a sink never takes a cut stream for
/// a whole one.
pub struct PipeWriter {
    sender: Option<mpsc::Sender<Bytes>>,
    core: Arc<Core>,
    abandoned: &'static str,
}

impl PipeWriter {
    /// Writes a chunk once the sink has taken the one before.
    pub async fn write(&mut self, bytes: Bytes) -> Result<(), PipeFailure> {
        let Some(sender) = &self.sender else {
            return Err(self.core.stopped());
        };
        if bytes.is_empty() {
            return Ok(());
        }
        let failed = self.core.failed();
        tokio::select! {
            biased;
            failure = failed => Err(failure),
            sent = sender.send(bytes) => sent.map_err(|_| self.core.stopped()),
        }
    }

    /// The source's end: the sink sees it after the last chunk.
    pub fn end(mut self) {
        self.sender = None;
    }

    /// Fails the pipe with `reason`.
    pub fn fail(mut self, reason: &str) {
        self.core.fail(reason);
        self.sender = None;
    }

    /// The writer as a sink of chunks, for an adapter that makes a byte
    /// stream of one, such as `tokio_util::io::SinkWriter`. Closing the sink
    /// is the source's end, as [`PipeWriter::end`] is; dropping it before it
    /// closed fails the pipe.
    pub fn into_sink(mut self) -> PipeSink {
        let sender = self
            .sender
            .take()
            .expect("a writer holds its sender until it ends or fails");
        PipeSink {
            sender: PollSender::new(sender),
            core: self.core.clone(),
            abandoned: self.abandoned,
            closed: false,
        }
    }
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        if self.sender.take().is_some() {
            self.core.fail(self.abandoned);
        }
    }
}

/// The source's side of a pipe as a sink of chunks: each is sent once the
/// sink took the one before, as [`PipeWriter::write`] sends it.
pub struct PipeSink {
    sender: PollSender<Bytes>,
    core: Arc<Core>,
    abandoned: &'static str,
    closed: bool,
}

impl Sink<Bytes> for PipeSink {
    type Error = PipeFailure;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), PipeFailure>> {
        let this = self.get_mut();
        if let Some(failure) = this.core.failure() {
            return Poll::Ready(Err(failure));
        }
        this.sender.poll_reserve(cx).map_err(|_| this.core.stopped())
    }

    fn start_send(self: Pin<&mut Self>, chunk: Bytes) -> Result<(), PipeFailure> {
        let this = self.get_mut();
        // An empty chunk carries nothing; the slot it reserved waits for the
        // next one.
        if chunk.is_empty() {
            return Ok(());
        }
        this.sender.send_item(chunk).map_err(|_| this.core.stopped())
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), PipeFailure>> {
        // A chunk is sent once the channel holds it, as a writer's is.
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), PipeFailure>> {
        let this = self.get_mut();
        this.sender.close();
        this.closed = true;
        Poll::Ready(Ok(()))
    }
}

impl Drop for PipeSink {
    fn drop(&mut self) {
        if !self.closed {
            self.core.fail(self.abandoned);
        }
    }
}

/// A device's `PUT` of a pipe's source, for the edge to feed.
pub struct DeviceSource {
    writer: PipeWriter,
}

impl DeviceSource {
    /// Forwards the request body into the pipe to its end, and resolves with
    /// the pipe's outcome, which is the request's answer: drained, even when
    /// the sink stopped early, or failed, also mid-body. Dropping the future
    /// before its end fails the pipe: the request went away.
    pub async fn pump<S, E>(self, body: S) -> Result<(), PipeFailure>
    where
        S: Stream<Item = Result<Bytes, E>>,
        E: Display,
    {
        let core = self.writer.core.clone();
        let mut writer = self.writer;
        let failed = core.failed();
        tokio::pin!(body, failed);
        loop {
            // A failure answers at once, even while the body is quiet.
            let chunk = tokio::select! {
                biased;
                failure = &mut failed => return Err(failure),
                chunk = body.next() => chunk,
            };
            match chunk {
                None => break,
                Some(Ok(bytes)) => {
                    if writer.write(bytes).await.is_err() {
                        return core.done().await;
                    }
                }
                Some(Err(error)) => {
                    writer.fail(&format!("source HTTP request disconnected: {error}"));
                    return core.done().await;
                }
            }
        }
        writer.end();
        core.done().await
    }
}

/// A device's `GET` of a pipe's sink, for the edge to answer.
pub struct DeviceSink {
    reader: PipeReader,
}

impl DeviceSink {
    /// Waits until the source has something to send: the response's head
    /// goes out only then, and a pipe that fails first is the refusal.
    pub async fn source_arrived(&mut self) -> Result<(), PipeFailure> {
        let mut arrived = self.reader.core.source_arrived.subscribe();
        let failed = self.reader.core.failed();
        tokio::select! {
            biased;
            failure = failed => Err(failure),
            _ = arrived.wait_for(|arrived| *arrived) => Ok(()),
        }
    }

    /// The response body. Its end drains the pipe; a response cut short
    /// fails it.
    pub fn into_stream(mut self) -> BoxStream<'static, Result<Bytes, PipeFailure>> {
        self.reader.abandoned = Abandoned::Fail("sink HTTP response disconnected before EOF");
        self.reader.into_stream()
    }
}
