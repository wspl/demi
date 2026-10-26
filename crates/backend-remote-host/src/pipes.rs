//! Pipe records (`crates/runner-protocol/src/wire/pipe_frames.rs`): every byte
//! stream between processes.
//!
//! A pipe has a source and a sink. An end on a device is that runner's HTTP
//! exchange with the backend: a `PUT` body or a stream pipe's WebSocket from
//! the source, `GET` by the sink. An end in this process is a reader or a
//! writer.
//!
//! The records live in the user's shard and hold only the byte in flight. The
//! ends are `Send`, so the edge that serves a device's request copies the
//! bytes.
//!
//! The pipes of an rpc call outlive the connection their device is on: a call
//! goes on across a drop, and its standard output, which the runner reads over a
//! WebSocket of its own, resumes where the runner got to ([`SinkSession`]).
//! Such a pipe has no arrival window, since its call bounds it, and the loss of
//! the device's connection fails it only as the loss of its own: the input the
//! runner was sending is cut with its connection.
//!
//! An end may be named after the pipe is minted: an rpc call's pipes are minted
//! before its handler decides whether it reads and writes them or hands them to
//! a job elsewhere. A pipe whose ends have not both arrived within the arrival
//! window fails.
//!
//! A content pipe from a device comes in chunks, each a `PUT` that names its
//! offset (`crates/runner-protocol/src/wire/pipe_frames.rs`). Between two, the
//! record keeps the source's writer and the bytes it took ([`UploadHold`]), so
//! a chunk whose connection broke goes again from there. A runner whose upload
//! waits for its turn says so, and the pipe then waits for its source without
//! its arrival window.
//!
//! A pipe between two devices whose runners may talk directly, as its minter
//! says ([`Pipe::may_go_direct`]), moves its bytes to their direct channel and
//! back (`crates/runner-protocol/src/wire/pipe_frames.rs`): its record tells the
//! source's stream pipe its sink's device, and carries the source's moves to the
//! sink in order with its bytes, naming the source's device from the record.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fmt::Display,
    future::{Future, pending},
    io,
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use demi_runner_protocol::wire::{self, PipePath, PipeRef};
use futures_util::{Stream, StreamExt, stream::BoxStream};
use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};
use tokio_util::{
    sync::CancellationToken,
    task::TaskTracker,
};

/// How long a pipe waits for its ends.
pub const ARRIVAL: Duration = Duration::from_secs(120);

/// The pipes of one user's devices.
#[derive(Clone)]
pub struct Pipes(Rc<Inner>);

struct Inner {
    slots: RefCell<HashMap<Arc<str>, Slot>>,
    /// The files offered to the devices for download.
    offers: crate::offers::FileOffers,
    arrival: Duration,
    /// One task per pipe: its arrival window and its removal once settled.
    tasks: TaskTracker,
    closed: Cell<bool>,
}

/// The resumable sinks of a device's session
/// ([`crate::session::DeviceSession`]), which holds them.
pub(crate) type Sinks = Arc<Mutex<HashMap<Arc<str>, Arc<SinkSession>>>>;

struct Slot {
    source: End,
    sink: End,
    source_arrived: bool,
    /// Whether the source's runner said its upload waits for its turn: the
    /// pipe then waits for it without the arrival window.
    source_queued: bool,
    sink_arrived: bool,
    /// A device source's writer between the chunks of its upload.
    upload: Arc<UploadHold>,
    /// The byte channel's ends, until the ends that use them arrive.
    sender: Option<mpsc::Sender<Piece>>,
    /// How long its ends have to arrive; none for a pipe of a call, which
    /// the call bounds.
    arrival: Option<Duration>,
    receiver: Option<mpsc::Receiver<Piece>>,
    /// Whether its two ends' runners may talk directly, once both are
    /// devices.
    direct: bool,
    /// The sink's device the source's stream pipe is told, once the pipe
    /// may move to the direct channel.
    sink_told: Arc<watch::Sender<Option<String>>>,
    /// Whether the pipe outlives its device's connection, as a call's does.
    survives: bool,
    /// Whether its device reads it over a WebSocket that resumes.
    resumable: bool,
    core: Arc<Core>,
    deadline: watch::Sender<Option<Instant>>,
}

/// What a pipe carries from its source to its sink.
#[derive(Debug)]
enum Piece {
    Bytes(Bytes),
    /// The source's bytes from `at` on go over `to`; `source` is its device.
    Moved { to: PipePath, at: u64, source: Arc<str> },
}

/// What a device's source sends besides its end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceMessage {
    Bytes(Bytes),
    /// Its bytes from `at` on go over `to`.
    Moved { to: PipePath, at: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum End {
    /// Not named yet.
    Open,
    /// This process: a reader or a writer, or a source held for one.
    Process,
    Device(String),
}

/// What the ends of one pipe share: its outcome, and the connection a
/// device's sink reads it on.
struct Core {
    state: watch::Sender<Outcome>,
    /// Closes the HTTP connection a device's sink reads the pipe on, once
    /// the device claimed it.
    sink_connection: OnceLock<CancellationToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Open,
    /// The sink read the source to its end, or stopped reading early.
    Drained,
    Failed(Arc<str>),
}

/// Why a pipe failed; it shows as `pipe failed: ` and the reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("pipe failed: {0}")]
pub struct PipeFailure(Arc<str>);

impl PipeFailure {
    /// Why the pipe failed, without the words that say a pipe failed: what a
    /// runner's end is answered, which its command reports as its own error
    /// (`crates/host-interface/src/rpc.rs`).
    pub fn reason(&self) -> &str {
        &self.0
    }
}

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
            sink_connection: OnceLock::new(),
        })
    }

    /// Fails the pipe for both ends; false when it was over already.
    fn fail(&self, reason: &str) -> bool {
        self.state.send_if_modified(|state| {
            if *state != Outcome::Open {
                return false;
            }
            *state = Outcome::Failed(reason.into());
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
            .unwrap_or_else(|| PipeFailure("the sink stopped reading".into()))
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
            offers: crate::offers::FileOffers::default(),
            arrival,
            tasks: TaskTracker::new(),
            closed: Cell::new(false),
        }))
    }

    /// A pipe whose source is `source`'s runner or, when none, named later;
    /// likewise its sink.
    pub fn mint(&self, source: Option<&str>, sink: Option<&str>) -> Pipe {
        self.mint_within(source, sink, self.0.arrival)
    }

    /// [`Pipes::mint`], whose ends have `arrival` to arrive rather than the
    /// pipes' own window, as a forwarded connection's do.
    pub fn mint_within(&self, source: Option<&str>, sink: Option<&str>, arrival: Duration) -> Pipe {
        self.mint_pipe(source, sink, Some(arrival), false, false)
    }

    /// The standard output of an rpc call `device`'s runner reads: it
    /// outlives the device's connection and resumes where the runner got to.
    pub fn call_stdout(&self, device: &str) -> Pipe {
        self.mint_pipe(None, Some(device), None, true, true)
    }

    /// The finite standard input of an rpc call `device`'s runner sends: it
    /// outlives the device's connection, and the loss of the connection it
    /// goes over fails it.
    pub fn call_stdin(&self, device: &str) -> Pipe {
        self.mint_pipe(Some(device), None, None, true, false)
    }

    fn mint_pipe(
        &self,
        source: Option<&str>,
        sink: Option<&str>,
        arrival: Option<Duration>,
        survives: bool,
        resumable: bool,
    ) -> Pipe {
        let id: Arc<str> = uuid::Uuid::new_v4().simple().to_string().into();
        let core = Core::new();
        // One chunk at a time: a pipe carries bulk bytes, so its writer waits for
        // its reader rather than the backend holding them without end.
        let (sender, receiver) = mpsc::channel(1);
        let deadline = watch::Sender::new(arrival.map(|arrival| Instant::now() + arrival));
        let end =
            |device: Option<&str>| device.map_or(End::Open, |device| End::Device(device.into()));
        let watching = deadline.subscribe();
        self.0.slots.borrow_mut().insert(
            id.clone(),
            Slot {
                source: end(source),
                sink: end(sink),
                source_arrived: false,
                source_queued: false,
                sink_arrived: false,
                upload: Arc::default(),
                sender: Some(sender),
                receiver: Some(receiver),
                direct: false,
                sink_told: Arc::new(watch::Sender::new(None)),
                core: core.clone(),
                deadline,
                arrival,
                survives,
                resumable,
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

    /// The files offered to the user's devices for download.
    pub fn offers(&self) -> &crate::offers::FileOffers {
        &self.0.offers
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
        slot.update_deadline();
        Ok(DeviceSource {
            writer: PipeWriter {
                sender: Some(sender),
                core: slot.core.clone(),
                abandoned: "source HTTP request disconnected",
            },
            device: device.into(),
            sink: slot.sink_told.subscribe(),
        })
    }

    /// The source end of pipe `id` for a chunk of `device`'s upload from
    /// `offset`: the first claims the pipe's writer, and each after takes it
    /// from where the one before left it.
    pub fn claim_chunk(&self, id: &str, device: &str, offset: u64) -> Result<DeviceChunk, ChunkRefusal> {
        let mut slots = self.0.slots.borrow_mut();
        let slot = slots.get_mut(id).ok_or(ChunkRefusal::NotFound)?;
        if slot.source != End::Device(device.into()) || !slot.core.is_open() {
            return Err(ChunkRefusal::NotFound);
        }
        let hold = slot.upload.clone();
        let taken = hold.taken.load(Ordering::Acquire);
        if offset != taken {
            return Err(ChunkRefusal::Offset(taken));
        }
        let writer = match slot.sender.take() {
            Some(sender) => {
                slot.source_arrived = true;
                slot.update_deadline();
                PipeWriter {
                    sender: Some(sender),
                    core: slot.core.clone(),
                    abandoned: "source HTTP request disconnected",
                }
            }
            // A chunk still sends: the next goes once it ended.
            None => hold.take().ok_or(ChunkRefusal::Offset(taken))?,
        };
        Ok(DeviceChunk {
            writer: Some(writer),
            hold,
        })
    }

    /// The bytes of pipe `id` that `device`'s upload into it took.
    pub fn taken_by(&self, id: &str, device: &str) -> Result<u64, PipeRefusal> {
        let slots = self.0.slots.borrow();
        let slot = slots.get(id).ok_or(PipeRefusal::NotFound)?;
        if slot.source != End::Device(device.into()) || !slot.core.is_open() {
            return Err(PipeRefusal::NotFound);
        }
        Ok(slot.upload.taken.load(Ordering::Acquire))
    }

    /// A device's report that its upload into pipe `id` waits for its turn:
    /// the pipe waits for its source without the arrival window. It counts
    /// only from the pipe's source.
    pub fn queued_from_device(&self, id: &str, device: &str) {
        let mut slots = self.0.slots.borrow_mut();
        if let Some(slot) = slots.get_mut(id)
            && slot.source == End::Device(device.into())
        {
            slot.source_queued = true;
            slot.update_deadline();
        }
    }

    /// Whether a pipe between the runners of `one` and `other` that may go
    /// direct is open: what lets the backend introduce the two to each
    /// other.
    pub fn link_direct(&self, one: &str, other: &str) -> bool {
        let one = End::Device(one.into());
        let other = End::Device(other.into());
        self.0.slots.borrow().values().any(|slot| {
            slot.direct
                && slot.core.is_open()
                && ((slot.source == one && slot.sink == other) || (slot.source == other && slot.sink == one))
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
        slot.update_deadline();
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

    /// A device's report that its sink stopped reading before the source's
    /// end, as a job that ended or closed its standard input does: the pipe
    /// drains, as a reader in this process that lets go drains it, and its
    /// source and the device's response end without a failure. It counts
    /// only from the pipe's sink; a pipe already over stays as it ended.
    pub fn drain_from_device(&self, id: &str, device: &str) {
        let slots = self.0.slots.borrow();
        if let Some(slot) = slots.get(id)
            && slot.sink == End::Device(device.into())
        {
            slot.core.drain();
        }
    }

    /// Fails every pipe `device` is an end of: its runner went away.
    pub fn device_gone(&self, device: &str) {
        let party = End::Device(device.into());
        let reason = format!("device {device} disconnected");
        for slot in self.0.slots.borrow().values() {
            // A call's pipes outlive the connection; its session's end and its
            // own failures end them.
            if (slot.source == party || slot.sink == party) && !slot.survives {
                slot.core.fail(&reason);
            }
        }
    }

    /// The device's standard output of a call that resumes, for the runner's
    /// WebSocket: the session it reads through, taking the pipe's sink end.
    /// Another pipe is refused as not found, whatever its device.
    pub(crate) fn claim_call_sink(
        &self,
        id: &str,
        device: &str,
        registry: &Sinks,
    ) -> Result<Arc<SinkSession>, PipeRefusal> {
        let mut slots = self.0.slots.borrow_mut();
        let slot = slots
            .get_mut(id)
            .filter(|slot| slot.resumable)
            .ok_or(PipeRefusal::NotFound)?;
        if slot.sink != End::Device(device.into()) || !slot.core.is_open() {
            return Err(PipeRefusal::NotFound);
        }
        let receiver = slot.receiver.take().ok_or(PipeRefusal::AlreadyConnected)?;
        slot.sink_arrived = true;
        slot.update_deadline();
        let reader = PipeReader {
            receiver,
            core: slot.core.clone(),
            ended: false,
            abandoned: Abandoned::Fail("the call's output connection ended"),
        };
        Ok(Arc::new(SinkSession::new(
            id.into(),
            reader,
            Arc::downgrade(registry),
        )))
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
        change: impl FnOnce(&mut Slot) -> Result<T, PipeError>,
    ) -> Result<T, PipeError> {
        let mut slots = self.0.slots.borrow_mut();
        let slot = slots.get_mut(id).ok_or(PipeError::Settled)?;
        if !slot.core.is_open() {
            return Err(PipeError::Settled);
        }
        change(slot)
    }
}

impl Slot {
    /// Tells the source its sink, once both ends are two devices that may
    /// talk directly.
    fn tell_sink(&mut self) {
        if let (End::Device(source), End::Device(sink)) = (&self.source, &self.sink)
            && self.direct
            && source != sink
        {
            self.sink_told.send_replace(Some(sink.clone()));
        }
    }

    /// The arrival window runs while an end is missing: it stops once both
    /// arrived, and starts again when an end is named anew.
    fn update_deadline(&mut self) {
        if (self.source_arrived || self.source_queued) && self.sink_arrived {
            self.deadline.send_replace(None);
        } else if self.deadline.borrow().is_none() {
            self.deadline
                .send_replace(self.arrival.map(|arrival| Instant::now() + arrival));
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
            shown: false,
        }
    }

    /// A call's standard output (`Pipes::call_stdout`) as the runner wire names
    /// it: the route its WebSocket that resumes opens on.
    pub fn call_output_ref(&self) -> PipeRef {
        PipeRef {
            id: self.id.to_string(),
            url: format!("{}/{}", wire::CALL_OUTPUT_ROUTE, self.id),
            shown: false,
        }
    }

    /// Makes this process the sink: the pipe's bytes as they come.
    pub fn reader(&self) -> Result<PipeReader, PipeError> {
        self.pipes.with_slot(&self.id, |slot| {
            match slot.sink {
                End::Open => slot.sink = End::Process,
                End::Process => {}
                End::Device(_) => return Err(PipeError::AlreadyFixed("sink")),
            }
            let receiver = slot.receiver.take().ok_or(PipeError::Taken("sink"))?;
            slot.sink_arrived = true;
            slot.update_deadline();
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
        self.pipes.with_slot(&self.id, |slot| {
            match slot.source {
                End::Open => slot.source = End::Process,
                End::Process => {}
                End::Device(_) => return Err(PipeError::AlreadyFixed("source")),
            }
            let sender = slot.sender.take().ok_or(PipeError::Taken("source"))?;
            slot.source_arrived = true;
            slot.update_deadline();
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
        self.pipes.with_slot(&self.id, |slot| {
            if slot.source != End::Open {
                return Err(PipeError::AlreadyFixed("source"));
            }
            slot.source_arrived = true;
            slot.update_deadline();
            Ok(())
        })
    }

    /// Names a device that will `GET` the pipe as its sink.
    pub fn sink_to(&self, device: &str) -> Result<(), PipeError> {
        self.pipes.with_slot(&self.id, |slot| {
            if slot.sink != End::Open {
                return Err(PipeError::AlreadyFixed("sink"));
            }
            slot.sink = End::Device(device.into());
            slot.sink_arrived = false;
            slot.update_deadline();
            slot.tell_sink();
            Ok(())
        })
    }

    /// Names a device that will `PUT` the pipe as its source, taking over a
    /// source this process held.
    pub fn source_from(&self, device: &str) -> Result<(), PipeError> {
        self.pipes.with_slot(&self.id, |slot| {
            if slot.source != End::Open {
                return Err(PipeError::AlreadyFixed("source"));
            }
            slot.source = End::Device(device.into());
            slot.source_arrived = false;
            slot.update_deadline();
            slot.tell_sink();
            Ok(())
        })
    }

    /// The pipe's two ends, devices whose runners may talk directly, may
    /// move its bytes to their direct channel: the minter knows the
    /// devices, and the policy that lets them.
    pub fn may_go_direct(&self) {
        let _ = self.pipes.with_slot(&self.id, |slot| {
            slot.direct = true;
            slot.tell_sink();
            Ok(())
        });
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

    /// Closes the HTTP connection a device's sink reads the pipe on, when a
    /// device claimed it: the device's read ends, also after the pipe
    /// drained, when its last bytes may still wait in the connection.
    pub fn close_sink_connection(&self) {
        if let Some(connection) = self.core.sink_connection.get() {
            connection.cancel();
        }
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
    /// A device's response cut short, or a reader whose caller went away,
    /// fails the pipe.
    Fail(&'static str),
}

/// The sink's side of a pipe. Dropping it before the end drains the pipe.
pub struct PipeReader {
    receiver: mpsc::Receiver<Piece>,
    core: Arc<Core>,
    ended: bool,
    abandoned: Abandoned,
}

impl PipeReader {
    /// The next chunk; none at the source's end. A reader in this process
    /// is never a device's sink, so no source's move reaches it.
    pub async fn next(&mut self) -> Option<Result<Bytes, PipeFailure>> {
        loop {
            match self.next_piece().await? {
                Ok(Piece::Bytes(bytes)) => return Some(Ok(bytes)),
                Ok(Piece::Moved { .. }) => {}
                Err(failure) => return Some(Err(failure)),
            }
        }
    }

    /// The next piece; none at the source's end.
    async fn next_piece(&mut self) -> Option<Result<Piece, PipeFailure>> {
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
                Some(piece) => Some(Ok(piece)),
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

    /// A reader whose drop before the end fails the pipe with `reason`
    /// rather than draining it: the source then learns that nobody waits
    /// for the rest, even while it sends nothing.
    pub fn failing_when_dropped(mut self, reason: &'static str) -> Self {
        self.abandoned = Abandoned::Fail(reason);
        self
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
    sender: Option<mpsc::Sender<Piece>>,
    core: Arc<Core>,
    abandoned: &'static str,
}

impl PipeWriter {
    /// Writes a chunk once the sink has taken the one before.
    pub async fn write(&mut self, bytes: Bytes) -> Result<(), PipeFailure> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.send(Piece::Bytes(bytes)).await
    }

    async fn send(&mut self, piece: Piece) -> Result<(), PipeFailure> {
        let Some(sender) = &self.sender else {
            return Err(self.core.stopped());
        };
        let failed = self.core.failed();
        tokio::select! {
            biased;
            failure = failed => Err(failure),
            sent = sender.send(piece) => sent.map_err(|_| self.core.stopped()),
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
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        if self.sender.take().is_some() {
            self.core.fail(self.abandoned);
        }
    }
}

/// A call's standard output as its runner reads it, which outlives each
/// WebSocket the runner reads it on: the pipe's sink end, and the numbers of
/// the messages written to the runner, whose unconfirmed ones the runner's
/// next connection receives again (`crates/runner-protocol/src/wire/session.rs`).
/// It goes once the runner confirmed the pipe's end, or with the device's
/// session.
pub struct SinkSession {
    pub(crate) id: Arc<str>,
    registry: std::sync::Weak<Mutex<HashMap<Arc<str>, Arc<SinkSession>>>>,
    /// Held by the connection that serves, which a newer one waits for.
    pub(crate) state: tokio::sync::Mutex<SinkState>,
    /// Ends the connection that serves, as a newer one takes over.
    attached: Mutex<Option<CancellationToken>>,
}

/// What a [`SinkSession`] keeps between connections.
pub(crate) struct SinkState {
    reader: PipeReader,
    pub(crate) numbers: wire::Session,
    /// Whether the end frame was written: the session is over once the
    /// runner confirmed everything.
    pub(crate) ended: bool,
}

impl SinkSession {
    fn new(
        id: Arc<str>,
        reader: PipeReader,
        registry: std::sync::Weak<Mutex<HashMap<Arc<str>, Arc<SinkSession>>>>,
    ) -> Self {
        Self {
            id,
            registry,
            state: tokio::sync::Mutex::new(SinkState {
                reader,
                numbers: wire::Session::new(),
                ended: false,
            }),
            attached: Mutex::new(None),
        }
    }

    /// A new connection takes the session over: the one that served ends.
    /// The token ends the new one when another takes over in its turn.
    pub(crate) fn attach(&self) -> CancellationToken {
        let token = CancellationToken::new();
        let previous = self
            .attached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(token.clone());
        if let Some(previous) = previous {
            previous.cancel();
        }
        token
    }

    pub(crate) fn cancel(&self) {
        if let Some(token) = self
            .attached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            token.cancel();
        }
    }

    /// The runner confirmed the pipe's end: nothing more to resume.
    pub(crate) fn finish(&self) {
        if let Some(registry) = self.registry.upgrade() {
            registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&self.id);
        }
    }
}

impl SinkState {
    /// The frames of the pipe's next bytes, or of its end, which come last;
    /// none ever once the end went. The pipe's own end, a failure or a sink
    /// the device reported done, is its end frame at once.
    pub(crate) async fn next_frames(&mut self) -> Vec<Bytes> {
        if self.ended {
            return pending().await;
        }
        let over = self.reader.core.done();
        let next = tokio::select! {
            biased;
            chunk = self.reader.next_piece() => chunk,
            outcome = over => {
                self.reader.ended = true;
                self.ended = true;
                let end = outcome.map_err(|failure| failure.reason().to_owned());
                return vec![wire::end_frame(&end)];
            }
        };
        match next {
            Some(Ok(Piece::Moved { to, at, source })) => vec![wire::moved_frame(to, at, &source)],
            Some(Ok(Piece::Bytes(bytes))) => bytes
                .chunks(wire::PIPE_DATA_BYTES)
                .map(|part| {
                    let mut frames = bytes::BytesMut::new();
                    wire::encode_data(part, &mut frames);
                    frames.freeze()
                })
                .collect(),
            Some(Err(failure)) => {
                self.ended = true;
                vec![wire::end_frame(&Err(failure.reason().to_owned()))]
            }
            None => {
                self.ended = true;
                vec![wire::end_frame(&Ok(()))]
            }
        }
    }
}

/// A device source's writer while no chunk of its upload sends, and the
/// bytes its chunks took.
#[derive(Default)]
struct UploadHold {
    writer: Mutex<Option<PipeWriter>>,
    taken: AtomicU64,
}

impl UploadHold {
    fn take(&self) -> Option<PipeWriter> {
        self.writer.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    fn keep(&self, writer: PipeWriter) {
        *self.writer.lock().unwrap_or_else(PoisonError::into_inner) = Some(writer);
    }
}

/// Why a chunk of a device's upload is not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChunkRefusal {
    /// No such pipe, the pipe is over, or its source is another device.
    #[error("no such pipe")]
    NotFound,
    /// The upload goes on from this offset, which the backend took.
    #[error("the upload goes on from byte {0}")]
    Offset(u64),
}

/// How a chunk of a device's upload went.
#[derive(Debug)]
pub enum ChunkEnd {
    /// It was taken, and the upload goes on from this offset.
    Taken(u64),
    /// Its request broke off: the upload goes on from the offset taken.
    Broken,
    /// The pipe is over: drained, or failed with why.
    Over(Result<(), PipeFailure>),
}

/// A chunk of a device's upload, for the edge to feed. Dropped before its
/// end, it leaves the writer for the next chunk.
pub struct DeviceChunk {
    writer: Option<PipeWriter>,
    hold: Arc<UploadHold>,
}

impl DeviceChunk {
    /// Forwards the chunk's bytes into the pipe as the sink takes them. The
    /// last, `complete`, ends the source and answers the pipe's outcome;
    /// another answers the offset the upload goes on from.
    pub async fn pump<S, E>(mut self, body: S, complete: bool) -> ChunkEnd
    where
        S: Stream<Item = Result<Bytes, E>>,
    {
        let Some(core) = self.writer.as_ref().map(|writer| writer.core.clone()) else {
            return ChunkEnd::Broken;
        };
        let over = core.done();
        tokio::pin!(body, over);
        loop {
            let chunk = tokio::select! {
                biased;
                outcome = &mut over => return ChunkEnd::Over(outcome),
                chunk = body.next() => chunk,
            };
            let Some(writer) = self.writer.as_mut() else {
                return ChunkEnd::Broken;
            };
            match chunk {
                Some(Ok(bytes)) => {
                    let count = bytes.len() as u64;
                    if writer.write(bytes).await.is_err() {
                        return ChunkEnd::Over(core.done().await);
                    }
                    self.hold.taken.fetch_add(count, Ordering::AcqRel);
                }
                // Dropping the chunk leaves the writer for the next.
                Some(Err(_)) => return ChunkEnd::Broken,
                None if complete => {
                    if let Some(writer) = self.writer.take() {
                        writer.end();
                    }
                    return ChunkEnd::Over(core.done().await);
                }
                None => return ChunkEnd::Taken(self.hold.taken.load(Ordering::Acquire)),
            }
        }
    }
}

/// A chunk whose request went away leaves its writer for the next.
impl Drop for DeviceChunk {
    fn drop(&mut self) {
        if let Some(writer) = self.writer.take() {
            self.hold.keep(writer);
        }
    }
}

/// A device's source end of a pipe, a `PUT` body or a stream pipe's
/// WebSocket, for the edge to feed.
pub struct DeviceSource {
    writer: PipeWriter,
    device: Arc<str>,
    sink: watch::Receiver<Option<String>>,
}

impl DeviceSource {
    /// The sink's device, once the pipe may move to the two runners' direct
    /// channel; the edge tells the source.
    pub fn sink(&self) -> watch::Receiver<Option<String>> {
        self.sink.clone()
    }

    /// Forwards the source's bytes and moves into the pipe to their end, and
    /// resolves with the pipe's outcome, which is the source's answer: drained,
    /// even when the sink stopped early, or failed, also mid-body. A move of a
    /// pipe that may not go direct fails it. Dropping the future before its end
    /// fails the pipe: the request went away.
    pub async fn pump<S, E>(self, body: S) -> Result<(), PipeFailure>
    where
        S: Stream<Item = Result<SourceMessage, E>>,
        E: Display,
    {
        let core = self.writer.core.clone();
        let mut writer = self.writer;
        let device = self.device;
        let sink = self.sink;
        let over = core.done();
        tokio::pin!(body, over);
        loop {
            // The pipe's end answers at once, even while the body is quiet:
            // its failure, or a sink that stopped reading.
            let chunk = tokio::select! {
                biased;
                outcome = &mut over => return outcome,
                chunk = body.next() => chunk,
            };
            match chunk {
                None => break,
                Some(Ok(SourceMessage::Bytes(bytes))) => {
                    if writer.write(bytes).await.is_err() {
                        return core.done().await;
                    }
                }
                Some(Ok(SourceMessage::Moved { to, at })) => {
                    if sink.borrow().is_none() {
                        writer.fail("a pipe that may not go direct cannot move");
                        return core.done().await;
                    }
                    let moved = Piece::Moved {
                        to,
                        at,
                        source: device.clone(),
                    };
                    if writer.send(moved).await.is_err() {
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
    /// Names the HTTP connection the response goes out on: cancelling
    /// `connection` closes it.
    pub fn closes_with(&self, connection: CancellationToken) {
        // A sink is claimed once, so no other connection is named.
        let _ = self.reader.core.sink_connection.set(connection);
    }

    /// The response body, which goes out at once, before the source is there,
    /// in the pipe's frames (`crates/runner-protocol/src/wire/pipe_frames.rs`):
    ///
    /// - Its bytes go in data frames.
    /// - An idle frame follows [`wire::PIPE_IDLE`] without a frame, so an edge
    ///   that ends a silent response keeps it.
    /// - Its end goes in an end frame, with the cause when it failed.
    ///
    /// Reading the source to its end drains the pipe, and a response cut short
    /// fails it. A pipe the device drained by reporting that it stopped reading
    /// ([`Pipes::drain_from_device`]) ends the response with a whole end.
    pub fn into_frames(mut self) -> BoxStream<'static, Bytes> {
        self.reader.abandoned = Abandoned::Fail("sink HTTP response disconnected before EOF");
        futures_util::stream::unfold(Some(self.reader), |reader| async move {
            let mut reader = reader?;
            let over = reader.core.done();
            let next = tokio::select! {
                biased;
                chunk = reader.next_piece() => chunk,
                outcome = over => {
                    reader.ended = true;
                    return Some((wire::end_frame(&outcome.map_err(|failure| failure.reason().to_owned())), None));
                }
                () = tokio::time::sleep(wire::PIPE_IDLE) => {
                    return Some((wire::idle_frame(), Some(reader)));
                }
            };
            Some(match next {
                Some(Ok(Piece::Bytes(bytes))) => {
                    let mut frames = bytes::BytesMut::new();
                    wire::encode_data(&bytes, &mut frames);
                    (frames.freeze(), Some(reader))
                }
                Some(Ok(Piece::Moved { to, at, source })) => (wire::moved_frame(to, at, &source), Some(reader)),
                Some(Err(failure)) => (wire::end_frame(&Err(failure.reason().to_owned())), None),
                None => (wire::end_frame(&Ok(())), None),
            })
        })
        .boxed()
    }
}
