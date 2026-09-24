//! The connection engine (`runner.md` § Connection and identity): one runner
//! connection's state, keyed by the ids of what the backend asked and waits
//! for, the routing of every message the runner sends, liveness, and the
//! teardown that ends everything in flight when the connection goes.
//!
//! The engine does no network IO. Its owner accepts the socket, reads the
//! runner's `hello`, and hands the driver the socket's frames
//! ([`LinkDriver::serve`]); the driver routes each message synchronously, so
//! a burst of replies never waits for the work they wake.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    fmt::Display,
    rc::Rc,
    time::Duration,
};

use bytes::Bytes;
use demi_command_service::protocol::{ArtifactLocation, CommandContext, PackageDescriptor};
use demi_core::StreamKind;
use demi_gates::{GateLease, SerialGate};
use demi_runner_protocol::wire::{
    self, ArtifactOwner, FsResult, GitResult, Inbound, LogLine, Outbound, OutputStream, VolumeName,
};
use demi_shell::{
    HostError, HostErrorKind, HostIdentity, HostKey, JobCaller, PortError, ProcessEnd,
    ProcessOutput, RpcError, RpcInvocation, RpcPort, SpawnError, SpawnErrorKind, StorageOp,
    StorageReply,
};
use futures_util::{Sink, SinkExt, Stream, StreamExt, future::LocalBoxFuture};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    manifest::CommandSelection,
    pipes::Pipes,
    relay::{CallEntry, Stop},
};

/// How often the backend asks a runner whether it is there.
pub const PING_INTERVAL: Duration = Duration::from_secs(30);

/// The frames the backend queues for a runner before a sender waits.
pub const OUTBOUND_FRAMES: usize = 64;

/// The artifact requests a connection answers at a time.
const ARTIFACT_REQUESTS: usize = 32;

/// How long a conversation release may take on the runner.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(360);

/// What the connection's owner decides for its jobs' calls: the product's
/// policy on top of the generic plumbing.
pub trait LinkPolicy {
    /// Whether calls from `job` may run, such as whether the job belongs to
    /// the conversation of the Host that started it. A refusal is the call's
    /// failure, before any pipe is minted.
    fn admit_call(&self, job: &JobOrigin) -> Result<(), String>;

    /// Runs the `rpc` call `invocation` from `job`.
    fn dispatch(
        &self,
        job: Rc<JobOrigin>,
        invocation: RpcInvocation,
        port: RpcPort,
    ) -> LocalBoxFuture<'static, Result<u8, RpcError>>;

    /// One operation on the command storage `job` is bound to, for a call
    /// that lives while `call` does: a write commits only while it lives.
    fn storage(
        &self,
        job: Rc<JobOrigin>,
        op: StorageOp,
        call: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<StorageReply, PortError>>;

    /// A managed guest asks for a larger volume.
    fn grow_volume(
        &self,
        volume: VolumeName,
        bytes: u64,
    ) -> LocalBoxFuture<'static, Result<(), String>>;
}

/// Whose a job is, recorded when it starts and read by its calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobOrigin {
    /// The Host that started the job.
    pub host: HostKey,
    /// What the job's declared commands receive.
    pub context: CommandContext,
    /// Whose command storage its `rpc` calls reach; none for a job no agent
    /// started.
    pub caller: Option<JobCaller>,
}

/// What makes a connection.
pub struct LinkOptions {
    /// The device whose runner this is: the name of its pipe ends.
    pub device: String,
    /// The account the runner works as, from its `hello`.
    pub identity: HostIdentity,
    pub pipes: Pipes,
    pub policy: Rc<dyn LinkPolicy>,
    /// How often to ping; none turns liveness off.
    pub ping: Option<Duration>,
}

/// Why a connection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEnd {
    /// The runner's side went away.
    Closed(String),
    /// The runner broke the protocol.
    Refused(String),
    /// The backend ended it, for this reason.
    Disconnected(String),
}

/// One runner connection, as the Hosts on its device reach it. Cloning it is
/// cheap; two handles are equal when they are the same connection.
#[derive(Clone)]
pub struct Link(Rc<Inner>);

impl PartialEq for Link {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Link {}

pub(crate) struct Inner {
    device: String,
    identity: HostIdentity,
    outbound: mpsc::Sender<Vec<u8>>,
    pipes: Pipes,
    policy: Rc<dyn LinkPolicy>,
    state: RefCell<State>,
    /// Calls, artifact resolutions and volume growth.
    tasks: TaskTracker,
    /// Job starts take turns: a job's manifest and its start travel
    /// together.
    jobs: SerialGate,
    /// Cancelled when the connection ends.
    closed: CancellationToken,
    /// Why it ended, once it has.
    end: RefCell<Option<String>>,
    /// Why the backend ends it; the driver returns with this.
    disconnect: RefCell<Option<String>>,
    liveness: Cell<Liveness>,
}

#[derive(Default)]
struct State {
    /// What waits for a reply, by request or stream id.
    waiting: HashMap<String, Waiting>,
    jobs: HashMap<String, JobEntry>,
    spawns: HashMap<String, SpawnEntry>,
    services: HashMap<String, ServiceEntry>,
    calls: HashMap<String, Rc<CallEntry>>,
    artifact_requests: HashSet<String>,
    /// The manifest this connection last carried, which later jobs share.
    manifest: Option<String>,
    /// The runner's own count of its jobs, from its last pong.
    pong_jobs: u64,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Liveness {
    #[default]
    Idle,
    /// A ping is unanswered.
    Waiting,
    /// A checkpoint copies the machine: silence is no death.
    Paused,
}

/// What a reply must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Expected {
    Fs(&'static str),
    Git(&'static str),
    Log,
    Release,
    Sync,
    Net,
    Service,
}

pub(crate) enum Answer {
    Fs(FsResult),
    Git(GitResult),
    Log { lines: Vec<LogLine>, next: u64 },
    Done,
}

struct Waiting {
    expected: Expected,
    answer: oneshot::Sender<Result<Answer, HostError>>,
}

/// Output and an end that the connection delivers and one consumer takes.
pub(crate) struct Shared<E> {
    state: RefCell<SharedState<E>>,
    changed: watch::Sender<u64>,
}

struct SharedState<E> {
    output: VecDeque<ProcessOutput>,
    end: Option<E>,
    /// The running declared commands' hints, first registered first.
    hints: Vec<(String, String)>,
}

impl<E: Clone> Shared<E> {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            state: RefCell::new(SharedState {
                output: VecDeque::new(),
                end: None,
                hints: Vec::new(),
            }),
            changed: watch::Sender::new(0),
        })
    }

    fn bump(&self) {
        self.changed
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    fn push(&self, stream: OutputStream, bytes: Vec<u8>) {
        let mut state = self.state.borrow_mut();
        if state.end.is_some() {
            return;
        }
        state.output.push_back(ProcessOutput {
            stream: match stream {
                OutputStream::Stdout => StreamKind::Stdout,
                OutputStream::Stderr => StreamKind::Stderr,
            },
            bytes: bytes.into(),
        });
        drop(state);
        self.bump();
    }

    pub(crate) fn finish(&self, end: E) {
        let mut state = self.state.borrow_mut();
        if state.end.is_some() {
            return;
        }
        state.end = Some(end);
        state.hints.clear();
        drop(state);
        self.bump();
    }

    fn hint(&self, invocation: String, hint: Option<String>) {
        let mut state = self.state.borrow_mut();
        if state.end.is_some() {
            return;
        }
        match hint {
            None => state.hints.retain(|(id, _)| *id != invocation),
            Some(hint) => match state.hints.iter_mut().find(|(id, _)| *id == invocation) {
                Some((_, current)) => *current = hint,
                None => state.hints.push((invocation, hint)),
            },
        }
    }

    /// The latest first-registered running command's hint.
    pub(crate) fn running_hint(&self) -> Option<String> {
        self.state
            .borrow()
            .hints
            .last()
            .map(|(_, hint)| hint.clone())
    }

    pub(crate) fn ended(&self) -> Option<E> {
        self.state.borrow().end.clone()
    }

    /// The next output chunk; none once the end came and every chunk was
    /// taken.
    pub(crate) async fn next_output(&self) -> Option<ProcessOutput> {
        let mut changed = self.changed.subscribe();
        loop {
            {
                let mut state = self.state.borrow_mut();
                if let Some(chunk) = state.output.pop_front() {
                    return Some(chunk);
                }
                if state.end.is_some() {
                    return None;
                }
            }
            // The sender lives as long as this `Shared`.
            let _ = changed.changed().await;
        }
    }

    /// The end, once it came.
    pub(crate) async fn end(&self) -> E {
        let mut changed = self.changed.subscribe();
        loop {
            if let Some(end) = self.ended() {
                return end;
            }
            let _ = changed.changed().await;
        }
    }
}

/// How a job ended: its status, where its script ended, its retained output
/// and the files it changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobEnd {
    pub status: ProcessEnd,
    /// The directory the script ended in; none when bash never ran it.
    pub cwd: Option<String>,
    pub output: Option<wire::RetainedOutput>,
    pub files: Vec<wire::JobFileChange>,
    pub files_truncated: bool,
}

impl JobEnd {
    pub(crate) fn lost(reason: &str) -> Self {
        Self {
            status: ProcessEnd::Lost(reason.into()),
            cwd: None,
            output: None,
            files: Vec::new(),
            files_truncated: false,
        }
    }
}

pub(crate) struct JobEntry {
    pub(crate) shared: Rc<Shared<JobEnd>>,
    pub(crate) origin: Rc<JobOrigin>,
    pub(crate) commands: Option<CommandSelection>,
    /// Cancelled when the job ends: its artifact resolutions stop.
    pub(crate) cancel: CancellationToken,
    /// Holds the Host's admission while the job runs.
    pub(crate) _lease: Option<GateLease>,
}

pub(crate) struct SpawnEntry {
    pub(crate) shared: Rc<Shared<ProcessEnd>>,
    /// A retained process is no work: it is not counted.
    pub(crate) retained: bool,
    pub(crate) _lease: Option<GateLease>,
}

pub(crate) struct ServiceEntry {
    pub(crate) package: PackageDescriptor,
    pub(crate) resolver: Rc<dyn crate::ArtifactResolver>,
    pub(crate) cancel: CancellationToken,
    pub(crate) done: Option<oneshot::Sender<Result<crate::ServiceEnd, HostError>>>,
}

impl Link {
    /// A connection, and the driver its owner serves the socket with.
    pub fn new(options: LinkOptions) -> (Link, LinkDriver) {
        let (outbound, frames) = mpsc::channel(OUTBOUND_FRAMES);
        let link = Link(Rc::new(Inner {
            device: options.device,
            identity: options.identity,
            outbound,
            pipes: options.pipes,
            policy: options.policy,
            state: RefCell::default(),
            tasks: TaskTracker::new(),
            jobs: SerialGate::new(),
            closed: CancellationToken::new(),
            end: RefCell::new(None),
            disconnect: RefCell::new(None),
            liveness: Cell::new(Liveness::Idle),
        }));
        let driver = LinkDriver {
            link: link.clone(),
            frames,
            ping: options.ping,
            tap: None,
        };
        (link, driver)
    }

    pub fn device(&self) -> &str {
        &self.0.device
    }

    pub fn identity(&self) -> &HostIdentity {
        &self.0.identity
    }

    pub fn pipes(&self) -> &Pipes {
        &self.0.pipes
    }

    /// Whether the connection is closing or closed: nothing new starts on
    /// it.
    pub fn is_closed(&self) -> bool {
        self.0.closed.is_cancelled()
    }

    /// Ends the connection for `reason`, such as the backend shutting down or
    /// the device being revoked.
    pub fn disconnect(&self, reason: &str) {
        self.0
            .disconnect
            .borrow_mut()
            .get_or_insert_with(|| reason.into());
        self.0.closed.cancel();
    }

    /// Stops liveness while a checkpoint copies the machine.
    pub fn pause_liveness(&self) {
        self.0.liveness.set(Liveness::Paused);
    }

    /// Starts liveness again, with no ping outstanding.
    pub fn resume_liveness(&self) {
        self.0.liveness.set(Liveness::Idle);
    }

    /// The jobs running on the device: the runner's own count, or the work
    /// the backend dispatched, whichever is larger.
    pub fn running_jobs(&self) -> u64 {
        let state = self.0.state.borrow();
        let dispatched = state.jobs.len()
            + state
                .spawns
                .values()
                .filter(|spawn| !spawn.retained)
                .count();
        state.pong_jobs.max(dispatched as u64)
    }

    /// Asks a managed guest to flush its writable filesystems.
    pub async fn sync(&self) -> Result<(), HostError> {
        self.call(Expected::Sync, |id| Inbound::Sync { id })
            .await
            .map(|_| ())
    }

    /// Asks the runner to release what its services hold for
    /// `conversation`. It admits no work and waits at most six minutes.
    pub async fn release_conversation(&self, conversation: &str) -> Result<(), HostError> {
        let release = self.call(Expected::Release, |id| Inbound::ConversationRelease {
            id,
            conversation_id: conversation.into(),
        });
        match tokio::time::timeout(RELEASE_TIMEOUT, release).await {
            Ok(result) => result.map(|_| ()),
            Err(_) => Err(HostError::interrupted("Conversation release timed out")),
        }
    }

    pub(crate) fn policy(&self) -> &Rc<dyn LinkPolicy> {
        &self.0.policy
    }

    /// Runs `task` as the connection's work, which its end waits for.
    pub(crate) fn spawn(&self, task: impl std::future::Future<Output = ()> + 'static) {
        self.0.tasks.spawn_local(task);
    }

    pub(crate) fn job_turn(&self) -> &SerialGate {
        &self.0.jobs
    }

    /// Why the connection ended, for what it fails.
    pub(crate) fn end_reason(&self) -> String {
        let ended = self.0.end.borrow().clone();
        ended
            .or_else(|| self.0.disconnect.borrow().clone())
            .unwrap_or_else(|| "runner disconnected".into())
    }

    pub(crate) fn offline(&self) -> HostError {
        HostError::offline(self.end_reason())
    }

    /// Queues `message` for the runner, once there is room. A message over
    /// the wire's limit fails here and leaves the connection as it is.
    pub(crate) async fn send(&self, message: &Inbound) -> Result<(), HostError> {
        if self.is_closed() {
            return Err(self.offline());
        }
        let frame = frame(message)?;
        self.send_frame(frame).await
    }

    /// Queues `message` for the runner without waiting, for a caller that
    /// cannot wait, such as a drop: at once when the queue has room, and
    /// otherwise from a task of the connection, which its end waits for. A
    /// closed connection sends nothing.
    pub(crate) fn post(&self, message: &Inbound) {
        if self.is_closed() {
            return;
        }
        // The messages posted are a few fields each, far below the limit
        // that is the only reason a frame could not be made.
        let Ok(frame) = frame(message) else {
            return;
        };
        if let Err(mpsc::error::TrySendError::Full(frame)) = self.0.outbound.try_send(frame) {
            let link = self.clone();
            self.spawn(async move {
                // A connection that ends first has nothing left to tell the
                // runner.
                let _ = link.send_frame(frame).await;
            });
        }
    }

    async fn send_frame(&self, frame: Vec<u8>) -> Result<(), HostError> {
        tokio::select! {
            biased;
            _ = self.0.closed.cancelled() => Err(self.offline()),
            sent = self.0.outbound.send(frame) => sent.map_err(|_| self.offline()),
        }
    }

    /// Sends the request `message` builds around a fresh id and waits for
    /// its reply, which must be `expected`.
    pub(crate) async fn call(
        &self,
        expected: Expected,
        message: impl FnOnce(String) -> Inbound,
    ) -> Result<Answer, HostError> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let answered = self.wait_for(id.clone(), expected)?;
        self.send(&message(id)).await?;
        answered.receive().await
    }

    /// Registers a reply to wait for under `id`, before its request goes
    /// out. Dropping the registration forgets it.
    pub(crate) fn wait_for(
        &self,
        id: String,
        expected: Expected,
    ) -> Result<Registration, HostError> {
        if self.is_closed() {
            return Err(self.offline());
        }
        let (answer, receiver) = oneshot::channel();
        self.0
            .state
            .borrow_mut()
            .waiting
            .insert(id.clone(), Waiting { expected, answer });
        Ok(Registration {
            link: self.clone(),
            id,
            receiver,
        })
    }

    pub(crate) fn with_state<T>(&self, f: impl FnOnce(&mut StateView<'_>) -> T) -> T {
        let mut state = self.0.state.borrow_mut();
        f(&mut StateView(&mut state))
    }

    /// The manifest hash this connection last carried.
    pub(crate) fn sent_manifest(&self) -> Option<String> {
        self.0.state.borrow().manifest.clone()
    }

    pub(crate) fn set_sent_manifest(&self, hash: Option<String>) {
        self.0.state.borrow_mut().manifest = hash;
    }

    /// Routes one message from the runner.
    pub(crate) fn receive(&self, message: Outbound) {
        match message {
            Outbound::ConversationReleased { id, error } => match error {
                None => self.answer(&id, Expected::Release, Answer::Done),
                Some(error) => self.refuse(&id, HostError::failed(None, error)),
            },
            Outbound::SyncDone { id, error } => match error {
                None => self.answer(&id, Expected::Sync, Answer::Done),
                Some(error) => self.refuse(&id, HostError::failed(None, error)),
            },
            // A repeated hello on a bound connection says nothing new.
            Outbound::Hello { .. } => {}
            Outbound::Pong { jobs } => {
                self.0.state.borrow_mut().pong_jobs = jobs;
                if self.0.liveness.get() == Liveness::Waiting {
                    self.0.liveness.set(Liveness::Idle);
                }
            }
            Outbound::VolumeGrow { id, volume, bytes } => self.grow_volume(id, volume, bytes),
            Outbound::FsOk(reply) => {
                let op = reply.result.op();
                self.answer(&reply.id, Expected::Fs(op), Answer::Fs(reply.result));
            }
            Outbound::FsError { id, code, message } => self.refuse(&id, fs_error(code, message)),
            Outbound::GitOk(reply) => {
                let op = reply.result.op();
                self.answer(&reply.id, Expected::Git(op), Answer::Git(reply.result));
            }
            Outbound::GitError { id, code, message } => {
                self.refuse(&id, HostError::failed(Some(code), message));
            }
            Outbound::LogLines { id, lines, next } => {
                self.answer(&id, Expected::Log, Answer::Log { lines, next });
            }
            Outbound::LogError { id, message } => {
                self.refuse(&id, HostError::failed(None, message))
            }
            Outbound::NetOpened { stream_id } => {
                self.answer(&stream_id, Expected::Net, Answer::Done)
            }
            Outbound::ServiceOpened { stream_id } => {
                self.answer(&stream_id, Expected::Service, Answer::Done);
            }
            Outbound::NetError {
                stream_id,
                code,
                message,
            } => self.refuse(
                &stream_id,
                HostError::failed(Some(code.to_string()), message),
            ),
            Outbound::ServiceError {
                stream_id,
                code,
                message,
            } => self.refuse(
                &stream_id,
                HostError::failed(Some(code.to_string()), message),
            ),
            Outbound::ServiceDone {
                stream_id,
                exit_code,
                stderr,
            } => self.end_service(&stream_id, Ok(crate::ServiceEnd { exit_code, stderr })),
            Outbound::SpawnOutput {
                spawn_id,
                stream,
                bytes,
            } => {
                if let Some(spawn) = self.0.state.borrow().spawns.get(&spawn_id) {
                    spawn.shared.push(stream, bytes.0);
                }
            }
            Outbound::SpawnExit {
                spawn_id,
                exit_code,
                signal,
                spawn_error,
            } => {
                let spawn = self.0.state.borrow_mut().spawns.remove(&spawn_id);
                if let Some(spawn) = spawn {
                    spawn
                        .shared
                        .finish(process_end(exit_code, signal, spawn_error));
                }
            }
            Outbound::JobOutput {
                job_id,
                stream,
                bytes,
            } => {
                if let Some(job) = self.0.state.borrow().jobs.get(&job_id) {
                    job.shared.push(stream, bytes.0);
                }
            }
            Outbound::JobRunningHint {
                job_id,
                invocation_id,
                hint,
            } => {
                if let Some(job) = self.0.state.borrow().jobs.get(&job_id) {
                    job.shared.hint(invocation_id, hint);
                }
            }
            Outbound::JobExit {
                job_id,
                exit_code,
                signal,
                spawn_error,
                cwd,
                output,
                files,
                files_truncated,
            } => {
                let calls: Vec<_> = self
                    .0
                    .state
                    .borrow()
                    .calls
                    .values()
                    .filter(|call| call.job_id == job_id)
                    .cloned()
                    .collect();
                for call in calls {
                    call.stop(Stop::Ended(format!(
                        "calling job {job_id} exited before its RPC completed"
                    )));
                }
                let job = self.0.state.borrow_mut().jobs.remove(&job_id);
                if let Some(job) = job {
                    job.cancel.cancel();
                    job.shared.finish(JobEnd {
                        status: process_end(exit_code, signal, spawn_error),
                        cwd,
                        output,
                        files,
                        files_truncated,
                    });
                }
            }
            Outbound::RpcCall {
                job_id,
                call_id,
                root,
                path,
                argv,
                args,
                json,
                cwd,
                env,
                stdin,
            } => crate::relay::start(
                self,
                crate::relay::RpcCall {
                    job_id,
                    call_id,
                    root,
                    path,
                    argv,
                    args,
                    json,
                    cwd,
                    env,
                    stdin,
                },
            ),
            Outbound::RpcStdin { call_id, bytes } => {
                let call = self.0.state.borrow().calls.get(&call_id).cloned();
                if let Some(call) = call {
                    call.live_input(Bytes::from(bytes.0));
                }
            }
            Outbound::RpcStdinEnd { call_id } => {
                let call = self.0.state.borrow().calls.get(&call_id).cloned();
                if let Some(call) = call {
                    call.end_live_input();
                }
            }
            Outbound::RpcCancel { call_id } => {
                let call = self.0.state.borrow().calls.get(&call_id).cloned();
                if let Some(call) = call {
                    call.stop(Stop::Cancelled);
                }
            }
            Outbound::PipeDone { pipe_id, ok, error } => {
                // The HTTP exchange's end is what settles a pipe; a device
                // reports a failed transfer early. A pipe the backend already
                // ended fails on the device too, which says nothing new.
                let reason = error.unwrap_or_else(|| "device transfer failed".into());
                if !ok
                    && self
                        .0
                        .pipes
                        .fail_from_device(&pipe_id, &self.0.device, &reason)
                {
                    tracing::info!(device = %self.0.device, pipe = %pipe_id, "pipe failed on the device: {reason}");
                }
            }
            Outbound::ArtifactResolve {
                id,
                owner,
                sha256,
                target,
            } => self.resolve_artifact(id, owner, sha256, target),
        }
    }

    /// Delivers a reply to what waits for `id`: the answer when it is the
    /// kind asked for, a protocol failure of that request when not.
    fn answer(&self, id: &str, answered: Expected, answer: Answer) {
        let Some(waiting) = self.0.state.borrow_mut().waiting.remove(id) else {
            return;
        };
        let result = if waiting.expected == answered {
            Ok(answer)
        } else {
            Err(HostError::new(
                HostErrorKind::Protocol,
                format!(
                    "the runner answered {answered:?} to a {:?} request",
                    waiting.expected
                ),
            ))
        };
        // The requester may have given up; nothing then waits for it.
        let _ = waiting.answer.send(result);
    }

    fn refuse(&self, id: &str, error: HostError) {
        let Some(waiting) = self.0.state.borrow_mut().waiting.remove(id) else {
            return;
        };
        // The requester may have given up; nothing then waits for it.
        let _ = waiting.answer.send(Err(error));
    }

    fn end_service(&self, stream: &str, end: Result<crate::ServiceEnd, HostError>) {
        let done = self
            .0
            .state
            .borrow_mut()
            .services
            .get_mut(stream)
            .and_then(|service| service.done.take());
        if let Some(done) = done {
            // A stream's owner that stopped waiting has closed it.
            let _ = done.send(end);
        }
    }

    fn grow_volume(&self, id: String, volume: VolumeName, bytes: u64) {
        let link = self.clone();
        let growth = self.0.policy.grow_volume(volume, bytes);
        self.0.tasks.spawn_local(async move {
            let error = growth.await.err();
            let answer = Inbound::VolumeGrown {
                id,
                volume,
                bytes,
                error,
            };
            if let Err(error) = link.send(&answer).await {
                tracing::warn!(device = %link.0.device, "volume growth answer not sent: {error}");
            }
        });
    }

    /// Answers the runner's request for an executable's location, for the
    /// live work that runs it and only while it does
    /// (`native-runtime.md` § Install the selected executable).
    fn resolve_artifact(&self, id: String, owner: ArtifactOwner, sha256: String, target: String) {
        let grant = self.grant(&owner);
        let refusal = match &grant {
            None => Some("No matching live job or stream"),
            Some(_) => {
                let mut state = self.0.state.borrow_mut();
                if state.artifact_requests.len() >= ARTIFACT_REQUESTS
                    || !state.artifact_requests.insert(id.clone())
                {
                    Some("Artifact resolution request limit or duplicate id")
                } else {
                    None
                }
            }
        };
        let link = self.clone();
        self.0.tasks.spawn_local(async move {
            if let Some(refusal) = refusal {
                link.answer_artifact(id, Err(refusal.into())).await;
                return;
            }
            let Some(grant) = grant else {
                return;
            };
            let artifact = grant
                .packages
                .iter()
                .filter_map(|package| package.targets.get(&target))
                .find(|artifact| artifact.sha256 == sha256)
                .cloned();
            let location = match artifact {
                None => Err("Artifact does not belong to the live work's packages".to_owned()),
                Some(artifact) => tokio::select! {
                    biased;
                    _ = grant.cancel.cancelled() => {
                        link.0.state.borrow_mut().artifact_requests.remove(&id);
                        return;
                    }
                    location = grant.resolver.resolve(&artifact, &target, grant.cancel.clone()) => {
                        location.and_then(|location| {
                            garde::Validate::validate(&location).map(|()| location).map_err(|report| report.to_string())
                        })
                    }
                },
            };
            link.0.state.borrow_mut().artifact_requests.remove(&id);
            // The answer goes only while the work it serves is live.
            if !grant.cancel.is_cancelled() {
                link.answer_artifact(id, location).await;
            }
        });
    }

    async fn answer_artifact(&self, id: String, location: Result<ArtifactLocation, String>) {
        let (location, error) = match location {
            Ok(location) => (Some(location), None),
            Err(error) => (None, Some(error)),
        };
        let answer = Inbound::ArtifactLocation {
            id,
            location,
            error,
        };
        if let Err(error) = self.send(&answer).await {
            tracing::warn!(device = %self.0.device, "artifact location not sent: {error}");
        }
    }

    fn grant(&self, owner: &ArtifactOwner) -> Option<Grant> {
        let state = self.0.state.borrow();
        match owner {
            ArtifactOwner::Job(owner) => {
                let job = state.jobs.get(&owner.job_id)?;
                let commands = job.commands.as_ref()?;
                (commands.hash() == owner.manifest_hash).then(|| Grant {
                    packages: commands.packages(),
                    resolver: commands.resolver(),
                    cancel: job.cancel.clone(),
                })
            }
            ArtifactOwner::Stream(owner) => {
                let service = state.services.get(&owner.stream_id)?;
                Some(Grant {
                    packages: vec![service.package.clone()],
                    resolver: service.resolver.clone(),
                    cancel: service.cancel.clone(),
                })
            }
        }
    }

    /// Ends everything in flight on the connection: replies fail as offline,
    /// jobs and processes end as lost, calls stop, and the device's pipes
    /// fail. Idempotent.
    pub(crate) fn teardown(&self, reason: &str) {
        if self.0.end.borrow().is_some() {
            return;
        }
        *self.0.end.borrow_mut() = Some(reason.into());
        self.0.closed.cancel();
        let state = std::mem::take(&mut *self.0.state.borrow_mut());
        for (_, waiting) in state.waiting {
            // A requester that gave up waits for nothing.
            let _ = waiting.answer.send(Err(HostError::offline(reason)));
        }
        for (_, job) in state.jobs {
            job.cancel.cancel();
            job.shared.finish(JobEnd::lost(reason));
        }
        for (_, spawn) in state.spawns {
            spawn.shared.finish(ProcessEnd::Lost(reason.into()));
        }
        for (_, mut service) in state.services {
            service.cancel.cancel();
            if let Some(done) = service.done.take() {
                // An owner that stopped waiting has closed its stream.
                let _ = done.send(Err(HostError::offline(reason)));
            }
        }
        for (_, call) in state.calls {
            call.stop(Stop::Ended(reason.into()));
        }
        self.0.pipes.device_gone(&self.0.device);
        self.0.tasks.close();
    }
}

/// What a connection's state offers the Host facets.
pub(crate) struct StateView<'a>(&'a mut State);

impl StateView<'_> {
    pub(crate) fn add_job(&mut self, id: String, entry: JobEntry) {
        self.0.jobs.insert(id, entry);
    }

    pub(crate) fn remove_job(&mut self, id: &str) -> Option<JobEntry> {
        self.0.jobs.remove(id)
    }

    pub(crate) fn job_origin(&self, id: &str) -> Option<Rc<JobOrigin>> {
        self.0.jobs.get(id).map(|job| job.origin.clone())
    }

    pub(crate) fn add_spawn(&mut self, id: String, entry: SpawnEntry) {
        self.0.spawns.insert(id, entry);
    }

    pub(crate) fn remove_spawn(&mut self, id: &str) -> Option<SpawnEntry> {
        self.0.spawns.remove(id)
    }

    pub(crate) fn add_service(&mut self, id: String, entry: ServiceEntry) {
        self.0.services.insert(id, entry);
    }

    pub(crate) fn remove_service(&mut self, id: &str) -> Option<ServiceEntry> {
        self.0.services.remove(id)
    }

    pub(crate) fn add_call(&mut self, id: String, call: Rc<CallEntry>) -> bool {
        if self.0.calls.contains_key(&id) {
            return false;
        }
        self.0.calls.insert(id, call);
        true
    }

    pub(crate) fn remove_call(&mut self, id: &str) -> Option<Rc<CallEntry>> {
        self.0.calls.remove(id)
    }
}

struct Grant {
    packages: Vec<PackageDescriptor>,
    resolver: Rc<dyn crate::ArtifactResolver>,
    cancel: CancellationToken,
}

/// A reply the connection routes to its requester. Dropping it before the
/// reply forgets the request.
pub(crate) struct Registration {
    link: Link,
    id: String,
    receiver: oneshot::Receiver<Result<Answer, HostError>>,
}

impl Registration {
    pub(crate) async fn receive(mut self) -> Result<Answer, HostError> {
        let answered = (&mut self.receiver).await;
        match answered {
            Ok(result) => result,
            // Teardown answers every registration it takes; a dropped
            // sender is the same loss.
            Err(_) => Err(self.link.offline()),
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.link.0.state.borrow_mut().waiting.remove(&self.id);
    }
}

/// The connection's driver: serves the socket its owner hands over.
pub struct LinkDriver {
    link: Link,
    frames: mpsc::Receiver<Vec<u8>>,
    ping: Option<Duration>,
    tap: Option<mpsc::Sender<Outbound>>,
}

impl LinkDriver {
    /// Copies every message the runner sends into `tap`, for tests that
    /// audit the wire; a full tap loses the copy, never the message.
    pub fn tap(mut self, tap: mpsc::Sender<Outbound>) -> Self {
        self.tap = Some(tap);
        self
    }

    /// Serves the connection: routes every frame from `incoming`, writes the
    /// frames the Hosts queue to `outgoing`, and pings, until either side
    /// ends it. Everything in flight then ends with the reason.
    pub async fn serve<I, O>(self, incoming: I, outgoing: O) -> LinkEnd
    where
        I: Stream<Item = Result<Vec<u8>, String>>,
        O: Sink<Vec<u8>>,
        O::Error: Display,
    {
        let LinkDriver {
            link,
            mut frames,
            ping,
            tap,
        } = self;
        let reading = async {
            tokio::pin!(incoming);
            loop {
                let frame = match incoming.next().await {
                    None => return LinkEnd::Closed("runner disconnected".into()),
                    Some(Err(error)) => {
                        return LinkEnd::Closed(format!("runner disconnected: {error}"));
                    }
                    Some(Ok(frame)) => frame,
                };
                match wire::decode::<Outbound>(&frame) {
                    Ok(message) => {
                        if let Some(tap) = &tap {
                            // A full tap loses a copy, never the message.
                            let _ = tap.try_send(message.clone());
                        }
                        link.receive(message);
                    }
                    Err(error) => return LinkEnd::Refused(error.to_string()),
                }
            }
        };
        let writing = async {
            tokio::pin!(outgoing);
            while let Some(frame) = frames.recv().await {
                if let Err(error) = outgoing.send(frame).await {
                    return LinkEnd::Closed(format!("runner disconnected: {error}"));
                }
            }
            LinkEnd::Closed("runner disconnected".into())
        };
        let liveness = async {
            let Some(every) = ping else {
                return std::future::pending().await;
            };
            let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
            loop {
                ticks.tick().await;
                match link.0.liveness.get() {
                    Liveness::Paused => continue,
                    Liveness::Waiting => {
                        return LinkEnd::Disconnected("liveness: ping unanswered".into());
                    }
                    Liveness::Idle => {
                        link.0.liveness.set(Liveness::Waiting);
                        if link.send(&Inbound::Ping {}).await.is_err() {
                            return LinkEnd::Closed("runner disconnected".into());
                        }
                    }
                }
            }
        };
        let end = tokio::select! {
            end = reading => end,
            end = writing => end,
            end = liveness => end,
            () = link.0.closed.cancelled() => {
                let reason = link.0.disconnect.borrow().clone().unwrap_or_else(|| "runner disconnected".into());
                LinkEnd::Disconnected(reason)
            }
        };
        let reason = match &end {
            LinkEnd::Closed(_) | LinkEnd::Refused(_) => "runner disconnected".to_owned(),
            LinkEnd::Disconnected(reason) => reason.clone(),
        };
        link.teardown(&reason);
        link.0.tasks.wait().await;
        end
    }
}

/// `message` as the frame the connection sends; a message over the wire's
/// limit cannot be one.
fn frame(message: &Inbound) -> Result<Vec<u8>, HostError> {
    let frame = wire::encode(message)
        .map_err(|error| HostError::new(HostErrorKind::Protocol, error.to_string()))?;
    if frame.encoded_len() > wire::MAX_MESSAGE_BYTES {
        return Err(HostError::new(
            HostErrorKind::TooLarge,
            format!(
                "the request is {} bytes, over the {}-byte message limit",
                frame.encoded_len(),
                wire::MAX_MESSAGE_BYTES
            ),
        ));
    }
    Ok(frame.into_bytes())
}

/// A process's end as the runner reports it.
pub(crate) fn process_end(
    exit_code: Option<i32>,
    signal: Option<String>,
    spawn_error: Option<wire::SpawnError>,
) -> ProcessEnd {
    if let Some(code) = exit_code {
        return ProcessEnd::Exited(code);
    }
    if let Some(error) = spawn_error {
        // A task the runner could not run at all says why in `signal`.
        return ProcessEnd::NotStarted(SpawnError {
            kind: match error.kind {
                wire::SpawnErrorKind::ExecutableNotFound => SpawnErrorKind::ExecutableNotFound,
                wire::SpawnErrorKind::PermissionDenied => SpawnErrorKind::PermissionDenied,
                wire::SpawnErrorKind::CwdUnusable => SpawnErrorKind::CwdUnusable,
                wire::SpawnErrorKind::IsDirectory => SpawnErrorKind::IsDirectory,
                wire::SpawnErrorKind::Other => SpawnErrorKind::Other,
            },
            detail: error.detail.or(signal),
        });
    }
    match signal {
        Some(signal) => ProcessEnd::Signalled(signal),
        None => ProcessEnd::Lost("the process ended without a status".into()),
    }
}

/// An fs failure the runner reports: `too_large` fails the request alone;
/// any other code is the operating system's.
pub(crate) fn fs_error(code: Option<String>, message: String) -> HostError {
    match code.as_deref() {
        Some("too_large") => HostError::new(HostErrorKind::TooLarge, message),
        _ => HostError::failed(code, message),
    }
}
