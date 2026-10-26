//! The connection engine (`crates/runner-protocol/src/wire/mod.rs`): one runner
//! connection's state, keyed by the ids of what the backend asked and waits
//! for, the routing of every message the runner sends, liveness, and the
//! teardown that ends everything in flight when the connection goes.
//!
//! # Writing
//!
//! Every message the backend sends is control or input
//! ([`Inbound::is_input`]). The two wait in queues of their own, which grow
//! as far as what is sent, and the driver writes control first: a stop or a
//! small request waits only for the frame being written, never for the input
//! queued before it, and the ping is control. A frame whose write makes no
//! progress for [`wire::LINK_WRITE_TIMEOUT`], and a connection that has said
//! nothing for [`wire::LINK_SILENCE`], end the connection.
//!
//! # The session
//!
//! The connection is one of many a device's session carries
//! ([`DeviceSession`]). The driver numbers each message where it writes it,
//! keeps it until the runner confirms it, and a connection that replaces
//! another begins by writing again what the runner did not receive. The
//! messages of the device's calls wait in the session's lane, which the
//! driver writes between control and input; the calls themselves belong to
//! the session, so the end of a connection stops none of them.
//!
//! The engine does no network IO. Its owner accepts the socket, reads the
//! runner's `hello`, and hands the driver the socket's frames
//! ([`LinkDriver::serve`]); the driver routes each message synchronously, so
//! a burst of replies never waits for the work they wake.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    fmt::Display,
    rc::{Rc, Weak},
    time::Duration,
};

use bytes::Bytes;
use demi_command_protocol::{ArtifactLocation, CommandContext, PackageDescriptor, ResidentSequence};
use demi_host_interface::{
    HostError, HostErrorKind, HostIdentity, HostKey, JobCaller, ProcessEnd, ProcessOutput,
    RpcError, RpcInvocation, RpcPort, SpawnError, SpawnErrorKind,
};
use demi_runner_protocol::direct::{DialRefusal, Introduced, OfferRefusal, StunUrl};
use demi_runner_protocol::forward::{Forward, ForwardRunState};
use demi_runner_protocol::wire::{LinkedWorktree, 
    self, ArtifactOwner, FileRead, FsResult, GitResult, HostArtifact, HostDownload, HostLogLine, Inbound, Outbound,
    Read, ReadAt,
};
use demi_shared_gates::GateLease;
use demi_shared_types::{BlobRef, StreamKind};
use futures_util::{Sink, SinkExt, Stream, StreamExt, future::LocalBoxFuture};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    device_jobs::{DeviceJobs, Hello},
    file_watch::LinkSubscriptions,
    manifest::{CommandCatalog, CommandSelection},
    output_records::{decode_output, stream_kind},
    pipes::Pipes,
    relay::{CallEnv, Stop},
    session::DeviceSession,
};

/// How often the backend asks a runner whether it is there.
pub const PING_INTERVAL: Duration = Duration::from_secs(30);

/// Why an artifact request names no work the backend knows.
const NO_LIVE_WORK: &str = "No matching live job or stream";

/// How long the backend's work for a connection may go on once the
/// connection ended, such as an rpc handler committing; whatever still runs
/// then is aborted, so the device goes offline and its next hello is
/// answered (`crates/runner-protocol/src/wire/mod.rs`).
pub const END_GRACE: Duration = Duration::from_secs(5);

/// What the connection's owner decides for its jobs' calls: the product's
/// policy on top of the generic plumbing.
pub trait LinkPolicy {
    /// Whether calls from `job` may run, such as whether the job belongs to
    /// the conversation of the Host that started it, and whether the
    /// conversation still reaches the job's device. A refusal is the call's
    /// failure, before any pipe is minted.
    fn admit_call<'a>(&'a self, job: &'a JobOrigin) -> LocalBoxFuture<'a, Result<(), String>>;

    /// Runs the `rpc` call `invocation` from `job`.
    fn dispatch(
        &self,
        job: Rc<JobOrigin>,
        invocation: RpcInvocation,
        port: RpcPort,
    ) -> LocalBoxFuture<'static, Result<u8, RpcError>>;

    /// The bytes of the blob `blob` of the user's namespace, which a
    /// handler returns as a medium (`crates/runner-jobs/src/commands/command_output.rs`); none when
    /// the namespace does not hold it.
    fn read_blob(&self, blob: BlobRef) -> LocalBoxFuture<'static, Result<Option<Bytes>, String>>;

    /// The device's runner asks that the device be revoked, as `run
    /// uninstall` does (`crates/runner/src/removal.rs`):
    /// once it is, its connection ends with `revoked`; or why it could not
    /// be.
    fn revoke_device(&self) -> LocalBoxFuture<'static, Result<(), String>>;

    /// A job the backend recorded running, which no agent has taken up yet,
    /// makes an `rpc` call: the agent that ran it takes it up, as a report
    /// of it does (`crates/agent-session/src/reports.rs`), so the call is served.
    /// Answers once the take-up was asked for, or why it cannot be.
    fn take_up_job(&self, job: String) -> LocalBoxFuture<'static, Result<(), String>>;

    /// A resident on the device asks for `count` numbers of
    /// `conversation`'s `sequence` (`crates/command-protocol/src/numbers.rs`): the first of them, reserved, or why there are none.
    fn reserve_numbers(
        &self,
        conversation: String,
        sequence: ResidentSequence,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<u64, String>>;

    /// A page's direct channel opened the user stream `name` of
    /// `conversation` on the device (`crates/runner-protocol/src/direct.rs`): what the backend knows the stream with while it is open, as
    /// a stream of that name it opened, or why it does not know it.
    fn direct_stream(
        &self,
        conversation: String,
        name: String,
    ) -> LocalBoxFuture<'static, Result<DirectAdmission, String>>;

    /// The page watches an admitted direct stream of `conversation` again,
    /// after it said nobody did: the lease that holds the conversation
    /// active again, as the stream's admission's did, or why there is none.
    fn watch_stream(&self, conversation: String) -> LocalBoxFuture<'static, Result<GateLease, String>>;

    /// A resident on the device told its plugin something, or its
    /// state was lost (`crates/command-protocol/src/messages.rs`). The
    /// connection hands its events on one after another, in the order the
    /// runner sent them.
    fn package_event(&self, event: PackageEvent) -> LocalBoxFuture<'static, ()>;

    /// The command packages of the backend's release, whose programs the
    /// runner keeps in its cache once the connection opens; none from a
    /// backend that publishes no packages.
    fn programs(&self) -> Option<CommandCatalog>;

    /// The runner said something of a port forward or of a peer it made
    /// with another runner (`crates/backend-user-shard/src/shard/port_forwards.rs`), in the order it said
    /// it; the policy answers through the link.
    fn forward_event(&self, event: ForwardEvent);

    /// The runner reported the linked worktrees of the repository of its
    /// project directory `directory` (`crates/runner-host/src/worktrees.rs`).
    /// The policy takes them before the connection routes the runner's next
    /// message, which may follow their change, as a move does.
    fn worktrees(&self, directory: String, worktrees: Vec<LinkedWorktree>);

    /// Where the Host's log lines go; none keeps the connection without one,
    /// and then the backend never asks the runner to send them.
    fn host_log(&self) -> Option<Rc<dyn HostLog>> {
        None
    }
}

/// The backend's store of one Host's log lines, which the runner's `log_lines`
/// frames fill (`crates/runner/src/host_log.rs`).
pub trait HostLog {
    /// The highest `seq` stored for the Host; none for a Host with no line.
    fn since(&self) -> LocalBoxFuture<'static, Option<u64>>;

    /// Stores a frame's lines, `missed` being the lines the runner no longer
    /// held.
    fn store(&self, lines: Vec<HostLogLine>, missed: u64);
}

/// What a runner says of the user's port forwards (`crates/backend-user-shard/src/shard/port_forwards.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardEvent {
    /// What `forward` is on this runner, its Listen On, now; none once it
    /// no longer listens.
    State {
        forward: String,
        state: Option<ForwardRunState>,
    },
    /// A connection of `forward` that goes through the relay, which the
    /// policy answers with [`Link::forward_opened`] under `id`.
    Open { id: String, forward: String },
    /// The runner asks to be introduced to `device`'s runner as peer
    /// `peer` with its offer `sdp`; the policy answers with
    /// [`Link::direct_dialed`] under `id`.
    Dial {
        id: String,
        peer: String,
        device: String,
        sdp: String,
    },
    /// The runner's peer `peer` with another runner ended.
    Closed { peer: String },
}

/// What a resident's package says to its plugin, through the runner.
#[derive(Debug, Clone, PartialEq)]
pub enum PackageEvent {
    /// The resident of `package` told its plugin `message` about
    /// `conversation`.
    Message {
        package: String,
        conversation: String,
        message: serde_json::Value,
    },
    /// The resident of `package` ended without releasing its conversations.
    Lost { package: String },
}

/// What the backend knows an open direct stream with: the lease that holds
/// its conversation active, as an open relay stream's does, and what a relay
/// stream of its name may install: the executable of the package the name
/// binds, none for a name that binds none, from where `resolver` locates it
/// (`crates/runner-command-packages/src/cache.rs`).
pub struct DirectAdmission {
    pub lease: GateLease,
    pub package: Option<PackageDescriptor>,
    pub resolver: Rc<dyn crate::ArtifactResolver>,
}

/// Whose a job is, recorded when it starts and read by its calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobOrigin {
    /// The Host that started the job.
    pub host: HostKey,
    /// What the job's declared commands receive.
    pub context: CommandContext,
    /// The agent node its `rpc` calls act for; none for a job no agent
    /// started.
    pub caller: Option<JobCaller>,
}

/// What makes a connection.
pub struct LinkOptions {
    /// The device whose runner this is: the name of its pipe ends.
    pub device: String,
    /// The account the runner works as, from its `hello`.
    pub identity: HostIdentity,
    /// The kernel and the `SHELL` a process the runner starts sees, from
    /// its `hello`.
    pub kernel: Option<wire::Kernel>,
    pub shell: Option<String>,
    /// The Host's physical memory in bytes, from its `hello`.
    pub memory: u64,
    pub pipes: Pipes,
    pub policy: Rc<dyn LinkPolicy>,
    /// How often to ping; none turns liveness off.
    pub ping: Option<Duration>,
    /// The device's jobs, which outlive the connection.
    pub jobs: DeviceJobs,
    /// The device's session, which its calls belong to and each connection
    /// carries on.
    pub session: DeviceSession,
    /// What the runner's hello says of its jobs, which the connection takes
    /// up (`crates/runner/src/connection/kept.rs`).
    pub hello: Hello,
}

/// Why a connection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEnd {
    /// The runner's side went away.
    Closed(String),
    /// The runner broke the protocol: the backend cannot decode its
    /// message, which the refusal names with the decoding error
    /// ([`wire::refusal`]).
    Refused(String),
    /// The runner closed it over a message of the backend's that it cannot
    /// decode, naming the message and the decoding error.
    RunnerRefused(String),
    /// The backend ended it, for this reason.
    Disconnected(String),
}

impl LinkEnd {
    /// What the connection's end fails its work with: a refusal on either
    /// side names the message, so a command lost with the connection says
    /// why (`crates/runner-protocol/src/wire/mod.rs`).
    fn reason(&self) -> String {
        match self {
            Self::Closed(_) => "runner disconnected".to_owned(),
            Self::Refused(refusal) => format!("runner disconnected: the backend {refusal}"),
            Self::RunnerRefused(refusal) => format!("runner disconnected: the runner {refusal}"),
            Self::Disconnected(reason) => reason.clone(),
        }
    }
}

/// Why a runner's socket carries no more frames, as the driver reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketEnd {
    /// The transport failed, or the runner broke the WebSocket protocol.
    Broken(String),
    /// The runner closed it over a message of the backend's that it cannot
    /// decode, and its close frame says which and why.
    Refused(String),
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

/// A connection's identity that does not keep the connection: what is
/// remembered for as long as a connection lasts, such as the directories a
/// Host was checked for, compares with it.
#[derive(Clone)]
pub struct WeakLink(Weak<Inner>);

impl WeakLink {
    /// Whether `link` is the connection this names.
    pub fn is(&self, link: &Link) -> bool {
        Weak::ptr_eq(&self.0, &Rc::downgrade(&link.0))
    }
}

pub(crate) struct Inner {
    device: String,
    /// The instance of the runner at its other end, from its hello.
    instance: u64,
    identity: HostIdentity,
    kernel: Option<wire::Kernel>,
    shell: Option<String>,
    memory: u64,
    /// The device's jobs, which outlive the connection.
    device_jobs: DeviceJobs,
    /// The device's session, which this connection carries on.
    session: DeviceSession,
    /// Where the messages the driver writes first wait.
    control: mpsc::UnboundedSender<wire::Frame>,
    /// Where a command's input waits behind them.
    input: mpsc::UnboundedSender<wire::Frame>,
    pipes: Pipes,
    policy: Rc<dyn LinkPolicy>,
    /// Where the runner's log lines are stored, when the policy keeps them.
    host_log: Option<Rc<dyn HostLog>>,
    state: RefCell<State>,
    /// The backend's work for the connection, such as its calls and artifact
    /// resolutions, which its end waits for.
    tasks: TaskTracker,
    /// Cancelled once the connection's end gave `tasks` their grace: what
    /// still runs is aborted.
    overdue: CancellationToken,
    /// The conversations whose release this connection is delivering, so a
    /// request made meanwhile is not sent twice.
    /// The package events waiting to be handed to the policy, in order.
    package_events: mpsc::UnboundedSender<PackageEvent>,
    /// Cancelled when the connection ends.
    closed: CancellationToken,
    /// Why it ended, once it has.
    end: RefCell<Option<String>>,
    /// Why the backend ends it; the driver returns with this.
    disconnect: RefCell<Option<String>>,
    liveness: Cell<Liveness>,
    /// When anything last arrived from the runner, a message, a ping or a
    /// pong.
    heard: Cell<tokio::time::Instant>,
    /// Changes with each pong, which a probe of the connection waits for.
    pongs: watch::Sender<()>,
    /// What the runner last reported its artifact cache holds; none until
    /// it reports.
    installed: watch::Sender<Option<Vec<HostArtifact>>>,
    /// What the runner's cache downloads now, as it last said; kept in
    /// memory only.
    downloading: watch::Sender<Vec<HostDownload>>,
}

#[derive(Default)]
struct State {
    /// What waits for a reply, by request or stream id.
    waiting: HashMap<String, Waiting>,
    /// The packages whose programs the runner was sent to keep: the
    /// backend's release's, and the ones of the release a prefetch named.
    programs: Option<CommandCatalog>,
    next_programs: Option<CommandCatalog>,
    spawns: HashMap<String, SpawnEntry>,
    residents: HashMap<String, ResidentEntry>,
    artifact_requests: HashSet<String>,
    numbers_requests: HashSet<String>,
    /// The manifest this connection last carried, which later jobs share.
    manifest: Option<String>,
    /// The runner's own count of its jobs, from its last pong.
    pong_jobs: u64,
    /// The subscriptions the connection's pages hold.
    subscriptions: LinkSubscriptions,
    /// The direct streams open on the device, by the id the runner gave
    /// each.
    direct_streams: HashMap<String, DirectStream>,
    /// Where what the runner says of each peer goes: to the page's
    /// signaling socket, by the peer's id.
    direct_peers: HashMap<String, mpsc::UnboundedSender<PeerEvent>>,
}

/// A direct stream the runner reported open: what its close cancels, the
/// policy's admission of it, whether its page watches it, and once admitted
/// and while watched the lease that holds its conversation active.
struct DirectStream {
    conversation: String,
    cancel: CancellationToken,
    admitted: watch::Sender<Admitted>,
    watched: bool,
    lease: Option<GateLease>,
}

/// Whether the policy admitted a direct stream; its artifact requests wait
/// for the answer.
#[derive(Clone)]
enum Admitted {
    Waiting,
    Yes(Grant),
    No,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Liveness {
    #[default]
    Watching,
    /// A checkpoint copies the machine: silence is no death.
    Paused,
}

/// The id a prefetch of `release` waits for its answer under, apart from
/// every request's own.
fn prefetch_id(release: &str) -> String {
    format!("prefetch:{release}")
}

/// What a reply must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Expected {
    Fs(&'static str),
    Git(&'static str),
    Release,
    PackageRelease,
    Sync,
    Prefetch,
    Resident,
    JobRead,
    JobMediaRead,
    DirectOffer,
    Http,
}

pub(crate) enum Answer {
    /// An fs call's result, with the Host's number as the runner received
    /// the call.
    Fs(FsResult, ReadAt),
    Git(GitResult, ReadAt),
    /// Which files of a read of several were read.
    Read(Vec<FileRead>),
    /// The runner's answer to a page's offer.
    Direct(DirectAnswer),
    /// An exposed port's answer to a request, but for its body.
    Http(HttpHead),
    Done,
}

/// What an exposed port answered a request (`crates/backend-user-shard/src/shard/exposed_ports.rs`): its status
/// and headers; its body comes through the request's answer pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpHead {
    pub status: u16,
    pub headers: Vec<demi_runner_protocol::exposure::HttpHeader>,
}

/// A request the backend makes for the user to one of the device's exposed
/// ports, as [`Link::http_request`] sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub scheme: demi_runner_protocol::exposure::HttpScheme,
    pub port: u16,
    pub method: String,
    /// The path and query, from `/`.
    pub path: String,
    pub headers: Vec<demi_runner_protocol::exposure::HttpHeader>,
}

/// What the runner says of a page's peer besides its answer
/// (`crates/runner-direct/src/lib.rs`, `packages/web/src/direct/measure.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerEvent {
    /// A candidate it found after its answer.
    Candidate(String),
    /// Its answer to the page's relay probe.
    Pong(u32),
}

/// What a runner said to a page's offer (`crates/runner-direct/src/lib.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectAnswer {
    /// The runner's answer, with its candidates.
    Answer(String),
    /// The runner did not answer, for this reason.
    Refused(OfferRefusal, String),
}

struct Waiting {
    expected: Expected,
    answer: oneshot::Sender<Result<Answer, HostError>>,
}

/// Output and an end that the connection delivers and one consumer takes: a
/// job's [`JobOutput`], a raw process's [`ProcessOutput`].
pub(crate) struct Shared<E, C> {
    state: RefCell<SharedState<E, C>>,
    changed: watch::Sender<u64>,
    /// Whether one of its rpc calls waits for the user's answer.
    awaits_user: watch::Sender<bool>,
}

struct SharedState<E, C> {
    output: VecDeque<C>,
    end: Option<E>,
    /// The running declared commands' hints, first registered first.
    hints: Vec<(String, String)>,
    /// The keys of the rpc calls that wait for the user's answer.
    asking: Vec<String>,
    /// The background tasks that keep a job running once its script has
    /// ended, by their command lines; empty while the script runs.
    outliving: Vec<String>,
}

/// One message of a job's output (`crates/runner-jobs/src/tasks.rs`): the
/// stream's bytes from `offset`. Beyond the stream's first `JOB_VIEW_BYTES`,
/// an offset past the end of the stream's previous bytes says the runner
/// left the bytes between out, and one without bytes says only that the
/// stream is `offset` bytes long.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobOutput {
    pub stream: StreamKind,
    pub offset: u64,
    pub bytes: Bytes,
}

impl<E: Clone, C> Shared<E, C> {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            state: RefCell::new(SharedState {
                output: VecDeque::new(),
                end: None,
                hints: Vec::new(),
                asking: Vec::new(),
                outliving: Vec::new(),
            }),
            changed: watch::Sender::new(0),
            awaits_user: watch::Sender::new(false),
        })
    }

    fn bump(&self) {
        self.changed
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    pub(crate) fn push(&self, chunk: C) {
        let mut state = self.state.borrow_mut();
        if state.end.is_some() {
            return;
        }
        state.output.push_back(chunk);
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
        state.asking.clear();
        state.outliving.clear();
        drop(state);
        self.awaits_user.send_replace(false);
        self.bump();
    }

    pub(crate) fn hint(&self, invocation: String, hint: Option<String>) {
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

    /// Whether the rpc call `call` waits for the user's answer.
    pub(crate) fn await_user(&self, call: String, waits: bool) {
        let mut state = self.state.borrow_mut();
        if state.end.is_some() {
            return;
        }
        state.asking.retain(|asking| *asking != call);
        if waits {
            state.asking.push(call);
        }
        let awaits = !state.asking.is_empty();
        drop(state);
        self.awaits_user.send_if_modified(|current| std::mem::replace(current, awaits) != awaits);
    }

    /// Whether one of the job's rpc calls waits for the user's answer.
    pub(crate) fn awaits_user(&self) -> watch::Receiver<bool> {
        self.awaits_user.subscribe()
    }

    /// The job's script has ended, and `tasks` still run.
    pub(crate) fn outlived_by(&self, tasks: Vec<String>) {
        let mut state = self.state.borrow_mut();
        if state.end.is_none() {
            state.outliving = tasks;
        }
    }

    /// The background tasks that keep the job running once its script has
    /// ended.
    pub(crate) fn outliving(&self) -> Vec<String> {
        self.state.borrow().outliving.clone()
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
    pub(crate) async fn next_output(&self) -> Option<C> {
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

/// How a job ended: its status, its retained output and the files it
/// changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobEnd {
    pub status: ProcessEnd,
    /// Each stream's length; none when bash never ran the script.
    pub output: Option<wire::OutputLengths>,
    pub files: Vec<wire::JobFileChange>,
    pub path_changes: Vec<wire::JobPathChange>,
    pub files_truncated: bool,
}

impl JobEnd {
    pub(crate) fn lost(reason: &str) -> Self {
        Self {
            status: ProcessEnd::Lost(reason.into()),
            output: None,
            files: Vec::new(),
            path_changes: Vec::new(),
            files_truncated: false,
        }
    }
}

/// A medium a job's command returned to the job, as the runner announced it
/// once it kept it (`crates/runner-jobs/src/tasks.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobMedium {
    pub number: u32,
    pub media_type: String,
    pub size: u64,
    /// The SHA-256 of its bytes, which names its blob.
    pub sha256: BlobRef,
    /// Its file's name, and an image's or video's size in pixels and a
    /// video's length, as the handler read them from its header.
    pub name: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
}

pub(crate) struct SpawnEntry {
    pub(crate) shared: Rc<Shared<ProcessEnd, ProcessOutput>>,
    /// A retained process is no work: it is not counted.
    pub(crate) retained: bool,
    pub(crate) _lease: Option<GateLease>,
}

pub(crate) struct ResidentEntry {
    pub(crate) package: PackageDescriptor,
    pub(crate) resolver: Rc<dyn crate::ArtifactResolver>,
    pub(crate) attached: Vec<crate::AttachedArtifact>,
    pub(crate) cancel: CancellationToken,
    pub(crate) done: Option<oneshot::Sender<Result<crate::ResidentEnd, HostError>>>,
}

impl Link {
    pub fn downgrade(&self) -> WeakLink {
        WeakLink(Rc::downgrade(&self.0))
    }

    /// A connection, and the driver its owner serves the socket with. The
    /// jobs its runner's hello lists go on over it, are stopped when the
    /// backend has no command for them, or are lost (`crates/runner/src/connection/kept.rs`).
    pub fn new(options: LinkOptions) -> (Link, LinkDriver) {
        let (control, controls) = mpsc::unbounded_channel();
        let (input, inputs) = mpsc::unbounded_channel();
        let (package_events, mut events) = mpsc::unbounded_channel();
        let link = Link(Rc::new(Inner {
            device: options.device,
            instance: options.hello.instance,
            identity: options.identity,
            kernel: options.kernel,
            shell: options.shell,
            memory: options.memory,
            device_jobs: options.jobs.clone(),
            session: options.session.clone(),
            control,
            input,
            pipes: options.pipes,
            host_log: options.policy.host_log(),
            policy: options.policy,
            state: RefCell::default(),
            tasks: TaskTracker::new(),
            overdue: CancellationToken::new(),
            package_events,
            closed: CancellationToken::new(),
            end: RefCell::new(None),
            disconnect: RefCell::new(None),
            liveness: Cell::new(Liveness::Watching),
            heard: Cell::new(tokio::time::Instant::now()),
            installed: watch::Sender::new(None),
            downloading: watch::Sender::new(Vec::new()),
            pongs: watch::Sender::new(()),
        }));
        if let Some(log) = link.0.host_log.clone() {
            let following = link.clone();
            link.spawn("log follow", async move {
                let since = log.since().await;
                tracing::info!(
                    event = "log.followed",
                    host_id = %following.0.device,
                    since,
                    "the runner's log is followed"
                );
                following.post(&Inbound::LogFollow { since });
            });
        }
        tracing::info!(
            event = "link.opened",
            host_id = %link.0.device,
            protocol = wire::VERSION,
            release = options.hello.release.as_str(),
            "a runner connection opened"
        );
        link.send_programs();
        let listed = options.hello.jobs.len();
        let adoption = options.jobs.adopt(&link, options.hello);
        if listed > 0 || adoption.lost > 0 || !adoption.resend.is_empty() {
            tracing::info!(
                event = "jobs.adopted",
                host_id = %link.0.device,
                listed,
                stopped = adoption.stop.len(),
                released = adoption.release.len(),
                resynced = adoption.resync.len(),
                resent = adoption.resend.len(),
                lost = adoption.lost,
                "the jobs of a runner's hello were matched with the backend's"
            );
        }
        // A call whose job the device no longer holds has ended with it.
        for call in options.session.calls() {
            if options.jobs.with(&call.job_id, |_| ()).is_none()
                && options.jobs.detached(&call.job_id).is_none()
            {
                call.stop(Stop::Ended(format!(
                    "calling job {} ended before its RPC completed",
                    call.job_id
                )));
            }
        }
        for job in adoption.stop {
            link.stop_unknown(job);
        }
        for job_id in adoption.release {
            link.post(&Inbound::JobRelease { job_id });
        }
        for (job, lengths) in adoption.resync {
            let reading = link.clone();
            link.spawn("kept output resync", async move {
                reading.resync(&job, lengths).await
            });
        }
        for (job, start, commands) in adoption.resend {
            let sent = link.send_job_start(commands.as_ref(), &start);
            let start = sent.map_err(|_| start);
            link.0.device_jobs.resent(&link, &job, start);
        }
        let handing = link.clone();
        link.spawn("package events", async move {
            loop {
                let event = tokio::select! {
                    () = handing.ended() => return,
                    event = events.recv() => event,
                };
                // The connection holds the sender while it lives.
                let Some(event) = event else { return };
                handing.0.policy.package_event(event).await;
            }
        });
        let driver = LinkDriver {
            link: link.clone(),
            controls,
            inputs,
            ping: options.ping,
            writes: None,
        };
        (link, driver)
    }

    /// Stops a job the backend has no command for, as a stop does: `TERM`,
    /// then `KILL` after five seconds; its exit releases its directory.
    fn stop_unknown(&self, job: String) {
        self.post(&Inbound::JobKill {
            job_id: job.clone(),
            signal: Some(wire::Signal::Terminate),
        });
        let link = self.clone();
        self.spawn("unknown job's stop", async move {
            tokio::select! {
                () = link.ended() => {}
                () = tokio::time::sleep(wire::STOP_GRACE) => link.post(&Inbound::JobKill {
                    job_id: job,
                    signal: Some(wire::Signal::Kill),
                }),
            }
        });
    }

    /// Reads the kept output of `job`, which goes on over this connection,
    /// and delivers what its consumer did not receive: the output the
    /// runner printed while no connection served (`crates/runner/src/connection/kept.rs`). A read that fails delivers only what arrives from now on.
    async fn resync(&self, job: &str, lengths: wire::OutputLengths) {
        let began = tokio::time::Instant::now();
        let read = async {
            let reader = crate::remote_host::filled_job_read(self, job).await?;
            let bytes = crate::remote_host::collect_pipe(reader, wire::JOB_KEPT_READ_BYTES).await?;
            let read_bytes = bytes.len();
            decode_output(&bytes, None)
                .map(|output| (output, read_bytes))
                .map_err(|error| {
                    HostError::new(
                        HostErrorKind::Protocol,
                        format!("the job's kept output does not decode: {error}"),
                    )
                })
        };
        match read.await {
            Ok((output, read_bytes)) => {
                tracing::info!(
                    event = "output.resynced",
                    host_id = %self.0.device,
                    job_id = job,
                    stdout_bytes = lengths.stdout_bytes,
                    stderr_bytes = lengths.stderr_bytes,
                    read_bytes,
                    ms = began.elapsed().as_millis() as u64,
                    "a job's kept output was read again"
                );
                self.0.device_jobs.resynced(self, job, Some(&output), lengths)
            }
            Err(error) => {
                tracing::warn!(
                    event = "output.missed",
                    host_id = %self.0.device,
                    %error,
                    job_id = job,
                    stdout_bytes = lengths.stdout_bytes,
                    stderr_bytes = lengths.stderr_bytes,
                    ms = began.elapsed().as_millis() as u64,
                    "a job's kept output could not be read again"
                );
                self.0.device_jobs.resynced(self, job, None, lengths);
            }
        }
    }

    pub fn device(&self) -> &str {
        &self.0.device
    }

    pub fn identity(&self) -> &HostIdentity {
        &self.0.identity
    }

    /// The kernel a process the runner starts runs on, as its hello
    /// reported it; none from a runner on Windows.
    pub fn kernel(&self) -> Option<&wire::Kernel> {
        self.0.kernel.as_ref()
    }

    /// The Host's physical memory in bytes, as the runner's hello reported
    /// it.
    pub fn memory(&self) -> u64 {
        self.0.memory
    }

    /// The `SHELL` a process the runner starts inherits, as its hello
    /// reported it; none when the device environment names none.
    pub fn shell(&self) -> Option<&str> {
        self.0.shell.as_deref()
    }

    pub fn pipes(&self) -> &Pipes {
        &self.0.pipes
    }

    /// Whether the connection is closing or closed: nothing new starts on
    /// it.
    /// What the runner reports its artifact cache holds, from now on
    /// (`crates/runner-command-packages/src/cache.rs`): none until its first
    /// report.
    pub fn watch_installed(&self) -> watch::Receiver<Option<Vec<HostArtifact>>> {
        self.0.installed.subscribe()
    }

    /// What the runner's cache downloads now and how far each came, as it
    /// last said (`crates/runner-command-packages/src/cache.rs`).
    pub fn watch_downloading(&self) -> watch::Receiver<Vec<HostDownload>> {
        self.0.downloading.subscribe()
    }

    pub fn is_closed(&self) -> bool {
        self.0.closed.is_cancelled()
    }

    /// Resolves once the connection is closing or closed.
    pub async fn ended(&self) {
        self.0.closed.cancelled().await;
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

    /// Starts liveness again, counting the silence from now.
    pub fn resume_liveness(&self) {
        self.heard();
        self.0.liveness.set(Liveness::Watching);
    }

    /// Notes that something arrived from the runner, as a message, a ping or
    /// a pong does; the connection that hears nothing for
    /// [`wire::LINK_SILENCE`] is lost.
    pub fn heard(&self) {
        self.0.heard.set(tokio::time::Instant::now());
    }

    /// Whether the runner answers a ping within `within`, as a new hello for
    /// the device asks of the connection that holds it (`crates/runner-protocol/src/wire/mod.rs`): any pong that arrives meanwhile is an
    /// answer. A connection that is closing does not answer; one whose
    /// liveness is paused does, since its silence is no death.
    pub async fn answers(&self, within: Duration) -> bool {
        if self.0.liveness.get() == Liveness::Paused {
            return true;
        }
        // Subscribing marks the pongs so far as seen.
        let mut pongs = self.0.pongs.subscribe();
        let asked = async {
            if self.send(&Inbound::Ping {}).is_err() {
                return false;
            }
            // The connection holds the sender, so the watch never closes.
            pongs.changed().await.is_ok()
        };
        let answered = tokio::select! {
            answered = tokio::time::timeout(within, asked) => answered.unwrap_or(false),
            () = self.ended() => false,
        };
        if !answered && !self.is_closed() {
            tracing::warn!(
                event = "link.ping_unanswered",
                host_id = %self.0.device,
                within_ms = within.as_millis() as u64,
                since_heard_ms = self.0.heard.get().elapsed().as_millis() as u64,
                "a runner did not answer a ping that asked whether its connection lives"
            );
        }
        answered
    }

    /// The jobs running on the device: the runner's own count, or the work
    /// the backend dispatched, whichever is larger.
    pub fn running_jobs(&self) -> u64 {
        let jobs = self.0.device_jobs.running_on(self);
        let state = self.0.state.borrow();
        let dispatched = jobs
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

    /// Tells the runner the packages of the backend's release, whose programs
    /// it keeps in its cache (`crates/runner/src/connection/owner.rs`), and
    /// lets it locate them.
    fn send_programs(&self) {
        let Some(programs) = self.0.policy.programs() else {
            return;
        };
        let packages = programs.packages();
        if packages.is_empty() {
            return;
        }
        self.0.state.borrow_mut().programs = Some(programs);
        self.post(&Inbound::Programs { packages });
    }

    /// Asks the runner to download runner release `release` ahead, whose
    /// executable for its target has `sha256` and `size`, and the programs
    /// of the release's `programs` (`crates/runner/src/update.rs`): done once
    /// they are in place, or why the executable is not. The runner names its
    /// answer by the release, which stands for the request's id.
    pub async fn prefetch(
        &self,
        release: &str,
        sha256: &str,
        size: u64,
        programs: Option<CommandCatalog>,
    ) -> Result<(), HostError> {
        let answered = self.wait_for(prefetch_id(release), Expected::Prefetch)?;
        let packages = programs.as_ref().map(CommandCatalog::packages).unwrap_or_default();
        self.0.state.borrow_mut().next_programs = programs;
        self.send(&Inbound::Prefetch {
            release: release.to_owned(),
            sha256: sha256.to_owned(),
            size,
            packages,
        })?;
        answered.receive().await.map(|_| ())
    }

    /// Tells the runner, in the background, to release what its residents
    /// hold for `conversation`, and with `over` what it keeps itself: its
    /// recordings. Nothing waits for it and nothing keeps it: a message the
    /// runner never answers changes nothing, since its next `hello` names what
    /// it is to hold (`crates/runner-protocol/src/wire/messages.rs`).
    pub fn tell_conversation_release(&self, conversation: &str, over: bool) {
        let link = self.clone();
        let conversation = conversation.to_owned();
        self.spawn("conversation release", async move {
            let asked = std::time::Instant::now();
            let result = link.release_conversation(&conversation, over).await;
            link.told("conversation", &conversation, 0, asked, result);
        });
    }

    /// Tells the runner, in the background, to stop the residents of
    /// `packages`, as [`Self::tell_conversation_release`] does.
    pub fn tell_package_release(&self, packages: Vec<String>) {
        let link = self.clone();
        self.spawn("package release", async move {
            let asked = std::time::Instant::now();
            let count = packages.len();
            let result = link.release_packages(packages).await;
            link.told("packages", "", count, asked, result);
        });
    }

    fn told(&self, what: &str, conversation: &str, packages: usize, asked: std::time::Instant, result: Result<(), HostError>) {
        let ms = u64::try_from(asked.elapsed().as_millis()).unwrap_or(u64::MAX);
        match result {
            Ok(()) => tracing::info!(event = "release.told", host_id = %self.0.device, what, conversation, packages, ms, "a Host answered a release"),
            Err(error) => tracing::warn!(
                event = "release.lost",
                host_id = %self.0.device,
                what,
                conversation,
                packages,
                ms,
                %error,
                "a Host did not answer a release; its next hello names what it holds"
            ),
        }
    }

    /// Asks the runner to release what its residents hold for
    /// `conversation`, and with `over`, since the conversation is archived or
    /// deleted, what it keeps itself: its recordings.
    async fn release_conversation(&self, conversation: &str, over: bool) -> Result<(), HostError> {
        self.call(Expected::Release, |id| Inbound::ConversationRelease {
            id,
            conversation_id: conversation.into(),
            over,
        })
        .await
        .map(|_| ())
    }

    /// Asks the runner to stop the residents of `packages`, which ends what
    /// they hold for every conversation.
    async fn release_packages(&self, packages: Vec<String>) -> Result<(), HostError> {
        self.call(Expected::PackageRelease, |id| Inbound::PackageRelease { id, packages })
            .await
            .map(|_| ())
    }

    /// Introduces a page, or another runner, to the runner with its offer for peer `peer`,
    /// which replaces the peer of that id, and waits for the runner's
    /// answer; the caller bounds the wait. The runner asks the STUN servers
    /// `stun` for its public addresses; what it finds after its answer comes
    /// through what [`Link::direct_candidates`] answers.
    pub async fn direct_offer(
        &self,
        peer: &str,
        sdp: String,
        introduction: Introduced,
        stun: Vec<StunUrl>,
    ) -> Result<DirectAnswer, HostError> {
        let answer = self
            .call(Expected::DirectOffer, |id| Inbound::DirectOffer {
                id,
                peer: peer.to_owned(),
                sdp,
                introduction,
                stun,
            })
            .await?;
        match answer {
            Answer::Direct(answer) => Ok(answer),
            _ => Err(HostError::new(
                HostErrorKind::Protocol,
                "the runner answered an offer with another reply",
            )),
        }
    }

    /// What the runner says of peer `peer` from now on, its candidates and
    /// its answers to the page's probes, until the page's signaling socket
    /// closes.
    pub fn direct_peer(&self, peer: &str) -> mpsc::UnboundedReceiver<PeerEvent> {
        let (said, events) = mpsc::unbounded_channel();
        self.0
            .state
            .borrow_mut()
            .direct_peers
            .insert(peer.to_owned(), said);
        events
    }

    /// The page's probe `id` of the relay path, which the runner answers at
    /// once.
    pub fn direct_ping(&self, peer: &str, id: u32) {
        self.post(&Inbound::DirectPing {
            peer: peer.to_owned(),
            id,
        });
    }

    /// A candidate the page of peer `peer` found after its offer.
    pub fn direct_candidate(&self, peer: &str, candidate: String) {
        self.post(&Inbound::DirectCandidate {
            peer: peer.to_owned(),
            candidate,
        });
    }

    /// The runner closes peer `peer`: its page went, or the introduction
    /// it was made with is out of date.
    pub fn direct_close(&self, peer: &str) {
        self.post(&Inbound::DirectClose {
            peer: peer.to_owned(),
        });
    }

    /// The page's signaling socket of peer `peer` closed: nothing the
    /// runner says of the peer has anywhere to go.
    pub fn direct_forget(&self, peer: &str) {
        self.0.state.borrow_mut().direct_peers.remove(peer);
    }

    /// The other runner's answer to this runner's `direct_dial` under `id`,
    /// or why the backend did not introduce them.
    pub fn direct_dialed(&self, id: String, answered: Result<String, (DialRefusal, String)>) {
        let message = match answered {
            Ok(sdp) => Inbound::DirectDialed { id, sdp },
            Err((code, message)) => Inbound::DirectDialRefused { id, code, message },
        };
        self.post(&message);
    }

    /// Tells the runner the directories of the projects on its Host, whose
    /// worktrees it reports.
    pub fn projects(&self, directories: Vec<String>) {
        self.post(&Inbound::Projects { directories });
    }

    /// Tells the runner every forward it is an end of, and the STUN
    /// servers it asks when it offers a peer to another runner.
    pub fn forwards(&self, forwards: Vec<Forward>, stun: Vec<StunUrl>) {
        self.post(&Inbound::Forwards { forwards, stun });
    }

    /// Asks the runner to try `forward`'s port again.
    pub fn forward_retry(&self, forward: &str) {
        self.post(&Inbound::ForwardRetry {
            forward: forward.to_owned(),
        });
    }

    /// Hands the runner, the Connect To of `forward`, a connection on the
    /// relay: it reads the Listen On side's bytes from `input` and writes
    /// its own into `output`.
    pub fn forward_connect(&self, forward: &str, input: wire::PipeRef, output: wire::PipeRef) {
        self.post(&Inbound::ForwardConnect {
            forward: forward.to_owned(),
            input,
            output,
        });
    }

    /// Answers the runner's `forward_open` under `id`: the connection's
    /// pipes from its side, input and output, or why it is refused.
    pub fn forward_opened(&self, id: String, opened: Result<(wire::PipeRef, wire::PipeRef), String>) {
        let message = match opened {
            Ok((input, output)) => Inbound::ForwardOpened { id, input, output },
            Err(message) => Inbound::ForwardRefused { id, message },
        };
        self.post(&message);
    }

    /// Asks the runner to make `request` to one of its exposed ports, its
    /// body read from `body`, and waits for the answer's status and headers;
    /// the answer's body comes through `answer`. The caller decides that
    /// the port is exposed (`crates/backend-user-shard/src/shard/exposed_ports.rs`).
    pub async fn http_request(
        &self,
        request: HttpRequest,
        body: Option<wire::PipeRef>,
        answer: wire::PipeRef,
    ) -> Result<HttpHead, HostError> {
        let answered = self
            .call(Expected::Http, |id| Inbound::HttpRequest {
                id,
                scheme: request.scheme,
                port: request.port,
                method: request.method,
                path: request.path,
                headers: request.headers,
                body,
                answer,
            })
            .await?;
        match answered {
            Answer::Http(head) => Ok(head),
            _ => Err(HostError::new(
                HostErrorKind::Protocol,
                "the runner answered a request to a port with another reply",
            )),
        }
    }

    /// Tells the page of peer `peer` what the runner said of it; a page
    /// that went has nobody to tell.
    fn tell_peer(&self, peer: &str, event: PeerEvent) {
        let mut state = self.0.state.borrow_mut();
        let gone = state
            .direct_peers
            .get(peer)
            .is_some_and(|page| page.send(event).is_err());
        if gone {
            state.direct_peers.remove(peer);
        }
    }

    /// Runs `task` as the connection's work, which its end waits for up to
    /// [`END_GRACE`]; a task still running then is aborted and logged as
    /// `name`.
    pub(crate) fn spawn(
        &self,
        name: impl Into<String>,
        task: impl std::future::Future<Output = ()> + 'static,
    ) {
        let name = name.into();
        let device = self.0.device.clone();
        let overdue = self.0.overdue.clone();
        self.0.tasks.spawn_local(async move {
            tokio::select! {
                biased;
                () = task => {}
                () = overdue.cancelled() => tracing::warn!(
                    event = "link_task.aborted",
                    host_id = %device,
                    task = name.as_str(),
                    grace_ms = END_GRACE.as_millis() as u64,
                    "a task of a runner connection was aborted after its end"
                ),
            }
        });
    }

    /// What the calls this connection starts need of their device, which is
    /// not this connection.
    pub(crate) fn call_env(&self) -> CallEnv {
        CallEnv {
            device: self.0.device.clone(),
            policy: self.0.policy.clone(),
            pipes: self.0.pipes.clone(),
            jobs: self.0.device_jobs.clone(),
            session: self.0.session.clone(),
        }
    }

    /// The instance of the runner at the connection's other end.
    pub(crate) fn instance(&self) -> u64 {
        self.0.instance
    }

    /// Sends a job's `start`, after the manifest of its `commands` when the
    /// connection did not carry that one last. The runner keeps every
    /// manifest the connection carried and the start names the one it pins.
    pub(crate) fn send_job_start(
        &self,
        commands: Option<&CommandSelection>,
        start: &Inbound,
    ) -> Result<(), HostError> {
        let sent = (|| {
            if let Some(commands) = commands
                && self.sent_manifest().as_deref() != Some(commands.hash())
            {
                self.send(&Inbound::Manifest {
                    manifest: commands.wire().clone(),
                })?;
                self.set_sent_manifest(Some(commands.hash().to_owned()));
            }
            self.send(start)
        })();
        if sent.is_err() {
            // What the runner received is uncertain: the next job sends its
            // manifest again.
            self.set_sent_manifest(None);
        }
        sent
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

    /// Queues `message` for the runner, in the queue of its class. A message
    /// over the wire's limit fails here and leaves the connection as it is.
    pub(crate) fn send(&self, message: &Inbound) -> Result<(), HostError> {
        if self.is_closed() {
            return Err(self.offline());
        }
        let frame = frame(message)?;
        self.queue(message.is_input(), frame)
    }

    /// [`Link::send`] for a message whose failure nobody could act on, such
    /// as a drop's: a closed connection has nothing left to tell the runner.
    pub(crate) fn post(&self, message: &Inbound) {
        let _ = self.send(message);
    }

    fn queue(&self, input: bool, frame: wire::Frame) -> Result<(), HostError> {
        let queue = if input { &self.0.input } else { &self.0.control };
        queue.send(frame).map_err(|_| self.offline())
    }

    /// Sends the request `message` builds around a fresh id and waits for
    /// its reply, which must be `expected`.
    pub(crate) async fn call(
        &self,
        expected: Expected,
        message: impl FnOnce(String) -> Inbound,
    ) -> Result<Answer, HostError> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let operation = match expected {
            Expected::Fs(name) | Expected::Git(name) => name,
            Expected::Release => "release",
            Expected::PackageRelease => "package_release",
            Expected::Sync => "sync",
            Expected::Prefetch => "prefetch",
            Expected::Resident => "resident",
            Expected::JobRead => "job_read",
            Expected::JobMediaRead => "job_media_read",
            Expected::DirectOffer => "direct_offer",
            Expected::Http => "http",
        };
        let kind = match expected {
            Expected::Fs(_) => "fs",
            Expected::Git(_) => "git",
            _ => "runner",
        };
        tracing::info!(
            event = "host_call.started",
            host_id = %self.0.device,
            call = id.as_str(),
            kind,
            operation,
            "a call to a Host began"
        );
        // Written when the call is dropped, a call its caller gave up included.
        let mut ending = CallEnding {
            host_id: self.0.device.clone(),
            call: id.clone(),
            kind,
            operation,
            began: tokio::time::Instant::now(),
            outcome: "cancelled",
            send_ms: None,
            bytes: None,
        };
        let answered = self.wait_for(id.clone(), expected).inspect_err(|error| ending.fail(error))?;
        let message = message(id);
        ending.bytes = frame(&message).map(|frame| frame.encoded_len()).ok();
        self.send(&message).inspect_err(|error| ending.fail(error))?;
        ending.send_ms = Some(ending.began.elapsed().as_millis() as u64);
        let answer = answered.receive().await.inspect_err(|error| ending.fail(error))?;
        ending.outcome = "ok";
        Ok(answer)
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

    pub(crate) fn with_subscriptions<T>(&self, f: impl FnOnce(&mut LinkSubscriptions) -> T) -> T {
        f(&mut self.0.state.borrow_mut().subscriptions)
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
            Outbound::PackageReleased { id } => self.answer(&id, Expected::PackageRelease, Answer::Done),
            Outbound::JobRead { id, error } => match error {
                None => self.answer(&id, Expected::JobRead, Answer::Done),
                Some(error) => self.refuse(&id, HostError::failed(None, error)),
            },
            Outbound::JobMediaRead { id, media } => {
                self.answer(&id, Expected::JobMediaRead, Answer::Read(media));
            }
            Outbound::JobMedium {
                job_id,
                number,
                media_type,
                size,
                sha256,
                name,
                width,
                height,
                duration_ms,
            } => {
                let Ok(sha256) = BlobRef::try_from(sha256) else {
                    self.disconnect("a job medium's digest is no SHA-256");
                    return;
                };
                // A runner announces a job's media again on each new
                // connection; the backend keeps each number once.
                self.0.device_jobs.with(&job_id, |job| {
                    let mut media = job.media.borrow_mut();
                    if media.iter().any(|medium| medium.number == number) {
                        return;
                    }
                    media.push(JobMedium {
                        number,
                        media_type,
                        size,
                        sha256,
                        name,
                        width,
                        height,
                        duration_ms,
                    });
                });
            }
            Outbound::Prefetched { release } => {
                self.answer(&prefetch_id(&release), Expected::Prefetch, Answer::Done);
            }
            Outbound::PrefetchFailed { release, reason } => {
                self.refuse(&prefetch_id(&release), HostError::failed(None, reason));
            }
            Outbound::SyncDone { id, error } => match error {
                None => self.answer(&id, Expected::Sync, Answer::Done),
                Some(error) => self.refuse(&id, HostError::failed(None, error)),
            },
            // A repeated hello on a bound connection says nothing new.
            Outbound::Hello { .. } => {}
            Outbound::Installed { artifacts } => {
                self.0.installed.send_replace(Some(artifacts));
            }
            Outbound::Downloading { downloads } => {
                self.0.downloading.send_replace(downloads);
            }
            Outbound::Pong { jobs } => {
                self.0.state.borrow_mut().pong_jobs = jobs;
                self.0.pongs.send_replace(());
            }
            Outbound::Revoke {} => self.revoke(),
            Outbound::DirectAnswer { id, sdp } => {
                let answer = Answer::Direct(DirectAnswer::Answer(sdp));
                self.answer(&id, Expected::DirectOffer, answer);
            }
            Outbound::DirectStream {
                stream,
                name,
                conversation,
                open,
            } => self.direct_stream(stream, name, conversation, open),
            Outbound::DirectStreamWatched { stream, watched } => self.direct_stream_watched(stream, watched),
            Outbound::DirectCandidate { peer, candidate } => {
                self.tell_peer(&peer, PeerEvent::Candidate(candidate));
            }
            Outbound::DirectPong { peer, id } => self.tell_peer(&peer, PeerEvent::Pong(id)),
            Outbound::DirectDial {
                id,
                peer,
                device,
                sdp,
            } => self.0.policy.forward_event(ForwardEvent::Dial {
                id,
                peer,
                device,
                sdp,
            }),
            Outbound::DirectClosed { peer } => {
                self.0.policy.forward_event(ForwardEvent::Closed { peer });
            }
            Outbound::ForwardState { forward, state } => {
                self.0.policy.forward_event(ForwardEvent::State { forward, state });
            }
            Outbound::Worktrees { directory, worktrees } => self.0.policy.worktrees(directory, worktrees),
            Outbound::ForwardOpen { id, forward } => {
                self.0.policy.forward_event(ForwardEvent::Open { id, forward });
            }
            Outbound::HttpAnswered { id, status, headers } => {
                self.answer(&id, Expected::Http, Answer::Http(HttpHead { status, headers }));
            }
            Outbound::HttpFailed { id, message } => self.refuse(&id, HostError::failed(None, message)),
            Outbound::DirectRefused { id, code, message } => {
                let answer = Answer::Direct(DirectAnswer::Refused(code, message));
                self.answer(&id, Expected::DirectOffer, answer);
            }
            Outbound::FsOk(reply) => {
                let op = reply.result.op();
                let answer = Answer::Fs(reply.result, reply.read_at);
                self.answer(&reply.id, Expected::Fs(op), answer);
            }
            Outbound::FsError {
                id,
                read_at,
                code,
                message,
            } => self.refuse(&id, fs_error(code, message).with_read_at(read_at)),
            Outbound::GitOk(reply) => {
                let op = reply.result.op();
                let answer = Answer::Git(reply.result, reply.read_at);
                self.answer(&reply.id, Expected::Git(op), answer);
            }
            Outbound::GitError {
                id,
                read_at,
                code,
                message,
            } => {
                self.refuse(&id, HostError::failed(Some(code), message).with_read_at(read_at));
            }
            Outbound::FsLive { id, since, epoch } => {
                self.with_subscriptions(|subscriptions| subscriptions.live(&id, since, epoch));
            }
            Outbound::FsChanged { changes } => {
                tracing::debug!(
                    event = "file_watch.fired",
                    host_id = %self.0.device,
                    changes = changes.len(),
                    "a runner reported changes of watched paths"
                );
                self.with_subscriptions(|subscriptions| subscriptions.changed(changes));
            }
            Outbound::FsFailed { id, reason } => {
                self.with_subscriptions(|subscriptions| subscriptions.failed(&id, reason));
            }
            Outbound::LogLines { lines, missed } => {
                if let Some(log) = &self.0.host_log {
                    log.store(lines, missed);
                }
            }
            Outbound::ResidentOpened { stream_id } => {
                self.answer(&stream_id, Expected::Resident, Answer::Done);
            }
            Outbound::ResidentError {
                stream_id,
                code,
                message,
            } => self.refuse(
                &stream_id,
                HostError::failed(Some(code.to_string()), message),
            ),
            Outbound::ResidentDone {
                stream_id,
                exit_code,
                stderr,
            } => self.end_resident(&stream_id, Ok(crate::ResidentEnd { exit_code, stderr })),
            Outbound::SpawnOutput {
                spawn_id,
                stream,
                bytes,
            } => {
                if let Some(spawn) = self.0.state.borrow().spawns.get(&spawn_id) {
                    spawn.shared.push(ProcessOutput {
                        stream: stream_kind(stream),
                        bytes: bytes.0.into(),
                    });
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
                offset,
                bytes,
            } => {
                self.0.device_jobs.output(
                    &job_id,
                    JobOutput {
                        stream: stream_kind(stream),
                        offset,
                        bytes: bytes.0.into(),
                    },
                );
            }
            Outbound::JobRunningHint {
                job_id,
                invocation_id,
                hint,
            } => {
                self.0.device_jobs.with(&job_id, |job| {
                    job.shared.hint(invocation_id, hint);
                });
            }
            Outbound::JobOutliving { job_id, tasks } => {
                self.0.device_jobs.with(&job_id, |job| {
                    job.shared.outlived_by(tasks);
                });
            }
            Outbound::JobContextEnded { job_id } => {
                tracing::info!(
                    event = "job_context.ended",
                    host_id = %self.0.device,
                    job_id = job_id.as_str(),
                    "a runner's job context ended"
                );
                self.0.device_jobs.context_ended(&job_id);
            }
            Outbound::JobExit {
                job_id,
                exit_code,
                signal,
                spawn_error,
                output,
                files,
                path_changes,
                files_truncated,
            } => {
                // A start that failed before its script ran may have named a
                // manifest the runner does not hold: the next job sends it
                // again. Any failed start counts, at the cost of one
                // manifest sent again.
                if spawn_error.is_some() {
                    let pinned = self
                        .0
                        .device_jobs
                        .with(&job_id, |job| {
                            job.commands
                                .as_ref()
                                .map(|commands| commands.hash().to_owned())
                        })
                        .flatten();
                    if pinned.is_some() && self.sent_manifest() == pinned {
                        self.set_sent_manifest(None);
                    }
                }
                for call in self.0.session.calls_of(&job_id) {
                    call.stop(Stop::Ended(format!(
                        "calling job {job_id} exited before its RPC completed"
                    )));
                }
                let end = JobEnd {
                    status: process_end(exit_code, signal, spawn_error),
                    output,
                    files,
                    path_changes,
                    files_truncated,
                        };
                // The exit of a job the backend has no command for, or one
                // it heard already over a connection that ended, releases
                // the job's directory.
                if !self.0.device_jobs.exited(&job_id, end) {
                    self.post(&Inbound::JobRelease { job_id });
                }
            }
            Outbound::RpcCall {
                job_id,
                call_id,
                // The path names the root first, which runs it.
                root: _,
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
                let call = self.0.session.call(&call_id);
                if let Some(call) = call {
                    call.live_input(Bytes::from(bytes.0));
                }
            }
            Outbound::RpcStdinEnd { call_id } => {
                let call = self.0.session.call(&call_id);
                if let Some(call) = call {
                    call.end_live_input();
                }
            }
            Outbound::RpcCancel { call_id } => {
                let call = self.0.session.call(&call_id);
                if let Some(call) = call {
                    call.stop(Stop::Cancelled);
                }
            }
            Outbound::PipeDone { pipe_id, ok, error } => {
                // The HTTP exchange's end is what settles a pipe; a device
                // reports a failed transfer early, and a sink that stopped
                // reading before the end while it still holds its exchange,
                // which drains the pipe. A pipe the backend already ended
                // fails on the device too, which says nothing new.
                if ok {
                    self.0.pipes.drain_from_device(&pipe_id, &self.0.device);
                } else {
                    let reason = error.unwrap_or_else(|| "device transfer failed".into());
                    if self
                        .0
                        .pipes
                        .fail_from_device(&pipe_id, &self.0.device, &reason)
                    {
                        tracing::info!(event = "pipe.failed", host_id = %self.0.device, pipe = %pipe_id, %reason, "a pipe failed on the device");
                    }
                }
            }
            Outbound::ArtifactResolve {
                id,
                owner,
                sha256,
                target,
            } => self.resolve_artifact(id, owner, sha256, target),
            Outbound::NumbersReserve {
                id,
                conversation_id,
                sequence,
                count,
            } => self.reserve_numbers(id, conversation_id, sequence, count),
            Outbound::PackageMessage {
                package,
                conversation_id,
                message,
            } => self.package_event(PackageEvent::Message {
                package,
                conversation: conversation_id,
                message,
            }),
            Outbound::PackageLost { package } => self.package_event(PackageEvent::Lost { package }),
        }
    }

    /// Queues `event` for the policy, behind the ones before it.
    fn package_event(&self, event: PackageEvent) {
        // The receiver lives as long as the connection's package task, which
        // ends with the connection.
        if let Err(lost) = self.0.package_events.send(event) {
            let (package, kind) = match &lost.0 {
                PackageEvent::Message { package, .. } => (package.as_str(), "message"),
                PackageEvent::Lost { package } => (package.as_str(), "lost"),
            };
            tracing::warn!(
                event = "package_event.dropped",
                host_id = %self.0.device,
                package,
                kind,
                cause = "link_ended",
                "a package's event was dropped, the connection's package task ended"
            );
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

    fn end_resident(&self, stream: &str, end: Result<crate::ResidentEnd, HostError>) {
        let done = self
            .0
            .state
            .borrow_mut()
            .residents
            .get_mut(stream)
            .and_then(|resident| resident.done.take());
        if let Some(done) = done {
            // A stream's owner that stopped waiting has closed it.
            let _ = done.send(end);
        }
    }

    /// Revokes the device at its runner's request. A revoked device's
    /// connection ends with `revoked`, which its end sends. A revocation
    /// that failed is answered with nothing: the runner, which waits a
    /// bounded time, removes itself all the same.
    fn revoke(&self) {
        let revocation = self.0.policy.revoke_device();
        let device = self.0.device.clone();
        self.spawn("revocation", async move {
            if let Err(error) = revocation.await {
                tracing::warn!(event = "runner_revocation.failed", host_id = %device, %error, "the runner's revocation failed");
            }
        });
    }

    /// Answers the runner's request for an executable's location, for the
    /// live work that runs it and only while it does
    /// (`crates/runner-command-packages/src/cache.rs`).
    fn resolve_artifact(&self, id: String, owner: ArtifactOwner, sha256: String, target: String) {
        let granted = self.grant(&owner);
        let refusal = match &granted {
            None => Some(NO_LIVE_WORK),
            Some(_) => {
                let mut state = self.0.state.borrow_mut();
                if !state.artifact_requests.insert(id.clone()) {
                    Some("Duplicate artifact resolution request id")
                } else {
                    None
                }
            }
        };
        let link = self.clone();
        self.spawn("artifact resolution", async move {
            if let Some(refusal) = refusal {
                link.answer_artifact(id, Err(refusal.into())).await;
                return;
            }
            let Some(granted) = granted else {
                return;
            };
            let grant = match granted {
                Granted::Now(grant) => grant,
                Granted::Admitting(mut admitted) => {
                    let decided = admitted
                        .wait_for(|admitted| !matches!(admitted, Admitted::Waiting))
                        .await
                        .map(|admitted| admitted.clone());
                    match decided {
                        Ok(Admitted::Yes(grant)) => grant,
                        Ok(_) => {
                            link.0.state.borrow_mut().artifact_requests.remove(&id);
                            link.answer_artifact(id, Err(NO_LIVE_WORK.into())).await;
                            return;
                        }
                        // The stream closed before the policy decided: its
                        // request gets no answer, as one it had in flight.
                        Err(_) => {
                            link.0.state.borrow_mut().artifact_requests.remove(&id);
                            return;
                        }
                    }
                }
            };
            let attached = grant
                .attached
                .iter()
                .find(|attached| attached.artifact.sha256 == sha256)
                .map(|attached| attached.location.clone());
            let artifact = grant
                .packages
                .iter()
                .find_map(|package| package.carries(&target, &sha256));
            let location = match (attached, artifact) {
                (Some(location), _) => garde::Validate::validate(&location)
                    .map(|()| location)
                    .map_err(|report| report.to_string()),
                (None, None) => {
                    Err("Artifact does not belong to the live work's packages".to_owned())
                }
                (None, Some(artifact)) => tokio::select! {
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

    /// Keeps the direct streams the runner reports open, by their ids, as
    /// streams the backend opened (`crates/runner-protocol/src/direct.rs`).
    ///
    /// Each opening asks the policy to admit the stream. Once admitted, the
    /// stream holds its conversation active and its artifact requests are
    /// answered. Its closing, or the connection's end, ends both and cancels
    /// the requests in flight.
    fn direct_stream(&self, stream: String, name: String, conversation: String, open: bool) {
        if !open {
            let closed = self.0.state.borrow_mut().direct_streams.remove(&stream);
            if let Some(closed) = closed {
                closed.cancel.cancel();
            }
            return;
        }
        let cancel = self.0.closed.child_token();
        {
            let mut state = self.0.state.borrow_mut();
            if state.direct_streams.contains_key(&stream) {
                tracing::warn!(event = "direct_stream.reported_twice", host_id = %self.0.device, %stream, "a direct stream was reported open twice");
                return;
            }
            let opened = DirectStream {
                conversation: conversation.clone(),
                cancel: cancel.clone(),
                admitted: watch::Sender::new(Admitted::Waiting),
                watched: true,
                lease: None,
            };
            state.direct_streams.insert(stream.clone(), opened);
        }
        let admission = self.0.policy.direct_stream(conversation.clone(), name);
        let link = self.clone();
        self.spawn("direct stream admission", async move {
            let admission = tokio::select! {
                // A stream that closed meanwhile holds nothing.
                _ = cancel.cancelled() => return,
                admission = admission => admission,
            };
            let mut state = link.0.state.borrow_mut();
            let Some(open) = state.direct_streams.get_mut(&stream) else {
                return;
            };
            match admission {
                Ok(admission) => {
                    // One its page stopped watching meanwhile holds nothing.
                    open.lease = open.watched.then_some(admission.lease);
                    open.admitted.send_replace(Admitted::Yes(Grant {
                        packages: admission.package.into_iter().collect(),
                        resolver: admission.resolver,
                        attached: Vec::new(),
                        cancel,
                    }));
                }
                Err(error) => {
                    tracing::warn!(event = "direct_stream.refused", host_id = %link.0.device, %conversation, %error, "a direct stream was not admitted");
                    open.admitted.send_replace(Admitted::No);
                }
            }
        });
    }

    /// Whether the page watches the open direct stream `stream` now: only a
    /// watched one holds its conversation active
    /// (`crates/runner-protocol/src/direct.rs`). A stream not yet admitted
    /// takes its lease at its admission while it is watched.
    fn direct_stream_watched(&self, stream: String, watched: bool) {
        let (conversation, cancel) = {
            let mut state = self.0.state.borrow_mut();
            let Some(open) = state.direct_streams.get_mut(&stream) else {
                return;
            };
            open.watched = watched;
            if !watched {
                open.lease = None;
                return;
            }
            let admitted = matches!(*open.admitted.borrow(), Admitted::Yes(_));
            if !admitted || open.lease.is_some() {
                return;
            }
            (open.conversation.clone(), open.cancel.clone())
        };
        let lease = self.0.policy.watch_stream(conversation.clone());
        let link = self.clone();
        self.spawn("direct stream watch", async move {
            let lease = tokio::select! {
                // A stream that closed meanwhile holds nothing.
                _ = cancel.cancelled() => return,
                lease = lease => lease,
            };
            let mut state = link.0.state.borrow_mut();
            let Some(open) = state.direct_streams.get_mut(&stream) else {
                return;
            };
            match lease {
                // Unwatched again, or watched twice, it keeps what it holds.
                Ok(lease) if open.watched && open.lease.is_none() => open.lease = Some(lease),
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(event = "direct_stream.rewatch_failed", host_id = %link.0.device, %conversation, %error, "a direct stream watched again holds nothing");
                }
            }
        });
    }

    /// Answers a resident's request for conversation numbers, which
    /// the policy decides (`crates/command-protocol/src/numbers.rs`).
    fn reserve_numbers(
        &self,
        id: String,
        conversation: String,
        sequence: ResidentSequence,
        count: u32,
    ) {
        let admitted = {
            let mut state = self.0.state.borrow_mut();
            state.numbers_requests.insert(id.clone())
        };
        let reserved =
            admitted.then(|| self.0.policy.reserve_numbers(conversation, sequence, count));
        let link = self.clone();
        self.spawn("numbers reservation", async move {
            let first = match reserved {
                None => Err("Duplicate numbers request id".to_owned()),
                Some(reserved) => {
                    let first = reserved.await;
                    link.0.state.borrow_mut().numbers_requests.remove(&id);
                    first
                }
            };
            let (first, error) = match first {
                Ok(first) => (Some(first), None),
                Err(error) => (None, Some(error)),
            };
            let answer = Inbound::NumbersReserved { id, first, error };
            if let Err(error) = link.send(&answer) {
                tracing::warn!(event = "numbers_answer.send_failed", host_id = %link.0.device, %error, "a numbers answer was not sent");
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
        if let Err(error) = self.send(&answer) {
            tracing::warn!(event = "artifact_location.send_failed", host_id = %self.0.device, %error, "an artifact location was not sent");
        }
    }

    /// What `owner` may install, when it is live work on the connection.
    fn grant(&self, owner: &ArtifactOwner) -> Option<Granted> {
        let state = self.0.state.borrow();
        match owner {
            ArtifactOwner::Job(owner) => self
                .0
                .device_jobs
                .with(&owner.job_id, |job| {
                    let commands = job.commands.as_ref()?;
                    (commands.hash() == owner.manifest_hash).then(|| {
                        Granted::Now(Grant {
                            packages: commands.packages(),
                            resolver: commands.resolver(),
                            attached: Vec::new(),
                            cancel: job.cancel.clone(),
                        })
                    })
                })
                .flatten(),
            ArtifactOwner::Programs(owner) => {
                let programs = if owner.next { &state.next_programs } else { &state.programs };
                let programs = programs.as_ref()?;
                Some(Granted::Now(Grant {
                    packages: programs.packages(),
                    resolver: programs.resolver(),
                    attached: Vec::new(),
                    cancel: self.0.closed.clone(),
                }))
            }
            ArtifactOwner::Stream(owner) => {
                if let Some(direct) = state.direct_streams.get(&owner.stream_id) {
                    return Some(Granted::Admitting(direct.admitted.subscribe()));
                }
                let resident = state.residents.get(&owner.stream_id)?;
                Some(Granted::Now(Grant {
                    packages: vec![resident.package.clone()],
                    resolver: resident.resolver.clone(),
                    attached: resident.attached.clone(),
                    cancel: resident.cancel.clone(),
                }))
            }
        }
    }

    /// Ends everything in flight on the connection. The method is idempotent.
    ///
    /// - Replies fail as offline.
    /// - Processes end as lost.
    /// - Resident and direct streams close (a direct stream's token is a child
    ///   of `closed`).
    /// - The device's pipes fail, but a call's: its calls go on with their
    ///   session.
    ///
    /// The device's jobs wait for its next connection
    /// (`crates/runner/src/connection/kept.rs`).
    pub(crate) fn teardown(&self, reason: &str) -> Aborted {
        if self.0.end.borrow().is_some() {
            return Aborted::default();
        }
        *self.0.end.borrow_mut() = Some(reason.into());
        self.0.closed.cancel();
        let mut state = std::mem::take(&mut *self.0.state.borrow_mut());
        state.subscriptions.end();
        let aborted = Aborted {
            host_calls: state.waiting.len(),
            spawns: state.spawns.len(),
            residents: state.residents.len(),
            tasks: 0,
        };
        for (_, waiting) in state.waiting {
            // A requester that gave up waits for nothing.
            let _ = waiting.answer.send(Err(HostError::offline(reason)));
        }
        self.0.device_jobs.detach(self);
        for (_, spawn) in state.spawns {
            spawn.shared.finish(ProcessEnd::Lost(reason.into()));
        }
        for (_, mut resident) in state.residents {
            resident.cancel.cancel();
            if let Some(done) = resident.done.take() {
                // An owner that stopped waiting has closed its stream.
                let _ = done.send(Err(HostError::offline(reason)));
            }
        }
        self.0.pipes.device_gone(&self.0.device);
        self.0.tasks.close();
        aborted
    }
}

/// The line that ends a call to a Host.
struct CallEnding {
    host_id: String,
    call: String,
    kind: &'static str,
    operation: &'static str,
    began: tokio::time::Instant,
    outcome: &'static str,
    send_ms: Option<u64>,
    bytes: Option<usize>,
}

impl CallEnding {
    fn fail(&mut self, error: &HostError) {
        self.outcome = match error.kind {
            HostErrorKind::Offline | HostErrorKind::Unavailable => "offline",
            HostErrorKind::Interrupted => "cancelled",
            HostErrorKind::Failed { .. } | HostErrorKind::TooLarge | HostErrorKind::Protocol => "refused",
        };
    }
}

impl Drop for CallEnding {
    fn drop(&mut self) {
        tracing::info!(
            event = "host_call.ended",
            host_id = self.host_id.as_str(),
            call = self.call.as_str(),
            kind = self.kind,
            operation = self.operation,
            outcome = self.outcome,
            ms = self.began.elapsed().as_millis() as u64,
            send_ms = self.send_ms,
            bytes = self.bytes,
            "a call to a Host ended"
        );
    }
}

/// What a connection's end cut short, for its `link.ended` line.
#[derive(Default)]
pub(crate) struct Aborted {
    /// The Host's calls that waited for an answer.
    host_calls: usize,
    spawns: usize,
    residents: usize,
    /// The tasks of the backend's that outlasted [`END_GRACE`].
    tasks: usize,
}

/// What a connection's state offers the Host facets.
pub(crate) struct StateView<'a>(&'a mut State);

impl StateView<'_> {

    pub(crate) fn add_spawn(&mut self, id: String, entry: SpawnEntry) {
        self.0.spawns.insert(id, entry);
    }

    pub(crate) fn remove_spawn(&mut self, id: &str) -> Option<SpawnEntry> {
        self.0.spawns.remove(id)
    }

    pub(crate) fn add_resident(&mut self, id: String, entry: ResidentEntry) {
        self.0.residents.insert(id, entry);
    }

    pub(crate) fn remove_resident(&mut self, id: &str) -> Option<ResidentEntry> {
        self.0.residents.remove(id)
    }
}

/// What live work may install: now, or once the policy admitted the direct
/// stream it is.
enum Granted {
    Now(Grant),
    Admitting(watch::Receiver<Admitted>),
}

#[derive(Clone)]
struct Grant {
    packages: Vec<PackageDescriptor>,
    resolver: Rc<dyn crate::ArtifactResolver>,
    /// Artifacts the work may install beside its packages', located
    /// already.
    attached: Vec<crate::AttachedArtifact>,
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
    controls: mpsc::UnboundedReceiver<wire::Frame>,
    inputs: mpsc::UnboundedReceiver<wire::Frame>,
    ping: Option<Duration>,
    /// Whether the socket's writes move; without it a write has no deadline.
    writes: Option<std::sync::Arc<wire::WriteProgress>>,
}

impl LinkDriver {
    /// Ends the connection once the socket's writes make no progress for
    /// [`wire::LINK_WRITE_TIMEOUT`].
    pub fn watching_writes(mut self, writes: std::sync::Arc<wire::WriteProgress>) -> Self {
        self.writes = Some(writes);
        self
    }

    /// Serves the connection: routes every frame from `incoming`, writes the
    /// frames the Hosts queue to `outgoing`, and pings, until either side
    /// ends it. Everything in flight then ends with the reason, and the
    /// backend's work for the connection gets [`END_GRACE`] to finish
    /// before it is aborted.
    pub async fn serve<I, O>(self, incoming: I, outgoing: O) -> LinkEnd
    where
        I: Stream<Item = Result<Vec<u8>, SocketEnd>>,
        O: Sink<Vec<u8>>,
        O::Error: Display,
    {
        let LinkDriver {
            link,
            mut controls,
            mut inputs,
            ping,
            writes,
        } = self;
        let began = tokio::time::Instant::now();
        let frames_in = std::cell::Cell::new(0u64);
        let frames_out = std::cell::Cell::new(0u64);
        let session = link.0.session.clone();
        // Wakes the writer when messages arrived that wait for an
        // acknowledgement.
        let owed = tokio::sync::Notify::new();
        let reading = async {
            tokio::pin!(incoming);
            loop {
                let frame = match incoming.next().await {
                    None => return LinkEnd::Closed("runner disconnected".into()),
                    Some(Err(SocketEnd::Broken(error))) => {
                        return LinkEnd::Closed(format!("runner disconnected: {error}"));
                    }
                    Some(Err(SocketEnd::Refused(refusal))) => {
                        return LinkEnd::RunnerRefused(refusal);
                    }
                    Some(Ok(frame)) => frame,
                };
                link.heard();
                frames_in.set(frames_in.get() + 1);
                let read = session.numbers().borrow_mut().read(&frame);
                let (payload, numbered) = match read {
                    Ok(Read::Own(payload)) => (payload, false),
                    Ok(Read::Message(payload)) => (payload, true),
                    // A message that was not written again says nothing, and
                    // counts in its turn.
                    Ok(Read::Skipped) => {
                        session.numbers().borrow_mut().handled();
                        owed.notify_one();
                        continue;
                    }
                    Ok(Read::Duplicate | Read::Ack) => {
                        owed.notify_one();
                        continue;
                    }
                    Err(error) => {
                        return LinkEnd::Closed(format!("runner disconnected: its session broke: {error}"));
                    }
                };
                match wire::decode::<Outbound>(payload) {
                    Ok(message) => link.receive(message),
                    Err(error) => return LinkEnd::Refused(wire::refusal(payload, &error)),
                }
                // Counted once routed, which nothing awaits.
                if numbered {
                    session.numbers().borrow_mut().handled();
                }
                owed.notify_one();
            }
        };
        let writing = async {
            tokio::pin!(outgoing);
            macro_rules! transmit {
                ($bytes:expr) => {{
                    let sending = outgoing.send($bytes);
                    tokio::pin!(sending);
                    let sent = match &writes {
                        Some(writes) => tokio::select! {
                            sent = &mut sending => Some(sent),
                            () = writes.stalled() => None,
                        },
                        None => Some(sending.await),
                    };
                    match sent {
                        Some(Ok(())) => frames_out.set(frames_out.get() + 1),
                        Some(Err(error)) => {
                            return LinkEnd::Closed(format!("runner disconnected: {error}"));
                        }
                        None => {
                            tracing::warn!(
                                event = "link.lost",
                                host_id = %link.0.device,
                                cause = "write",
                                stalled_ms = wire::LINK_WRITE_TIMEOUT.as_millis() as u64,
                                queued_control = controls.len(),
                                queued_input = inputs.len(),
                                "a write to the runner made no progress"
                            );
                            return LinkEnd::Disconnected(format!(
                                "a write made no progress for {} seconds",
                                wire::LINK_WRITE_TIMEOUT.as_secs()
                            ));
                        }
                    }
                }};
            }
            // What the runner did not receive goes again, before anything
            // new.
            loop {
                let frame = session.numbers().borrow_mut().next_resend();
                let Some(frame) = frame else { break };
                transmit!(frame);
            }
            loop {
                let ack_due = session.numbers().borrow().ack_due();
                // Control first: it waits only for the frame being written.
                let frame = tokio::select! {
                    biased;
                    () = async {
                        match ack_due {
                            Some(due) => tokio::time::sleep_until(due).await,
                            None => std::future::pending().await,
                        }
                    } => {
                        let ack = session.numbers().borrow_mut().ack_frame();
                        if let Some(ack) = ack {
                            transmit!(ack);
                        }
                        continue;
                    }
                    () = owed.notified(), if ack_due.is_none() => continue,
                    Some(frame) = controls.recv() => frame,
                    frame = session.next_message() => frame,
                    Some(frame) = inputs.recv() => frame,
                    else => return LinkEnd::Closed("runner disconnected".into()),
                };
                // Numbered where it is written, so the numbers are the order
                // on the wire, and kept until the runner confirms it.
                let carriage = frame.carriage();
                let bytes = match carriage {
                    wire::Carriage::Connection => wire::own_frame(frame.into_bytes()),
                    wire::Carriage::Outlasting => {
                        session.numbers().borrow_mut().stamp(frame.into_bytes(), true)
                    }
                    wire::Carriage::Scoped => {
                        session.numbers().borrow_mut().stamp(frame.into_bytes(), false)
                    }
                };
                transmit!(bytes);
            }
        };
        let liveness = async {
            let Some(every) = ping else {
                return std::future::pending().await;
            };
            let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
            loop {
                // A paused connection is lost at no time: wake again later.
                let lost_at = if link.0.liveness.get() == Liveness::Paused {
                    tokio::time::Instant::now() + wire::LINK_SILENCE
                } else {
                    link.0.heard.get() + wire::LINK_SILENCE
                };
                tokio::select! {
                    _ = ticks.tick() => {
                        if link.0.liveness.get() == Liveness::Watching
                            && link.send(&Inbound::Ping {}).is_err()
                        {
                            return LinkEnd::Closed("runner disconnected".into());
                        }
                    }
                    () = tokio::time::sleep_until(lost_at) => {
                        if link.0.liveness.get() == Liveness::Watching
                            && link.0.heard.get() + wire::LINK_SILENCE <= tokio::time::Instant::now()
                        {
                            tracing::warn!(
                                event = "link.lost",
                                host_id = %link.0.device,
                                cause = "silence",
                                silent_ms = wire::LINK_SILENCE.as_millis() as u64,
                                "nothing was heard from the runner"
                            );
                            return LinkEnd::Disconnected(format!(
                                "liveness: nothing heard for {} seconds",
                                wire::LINK_SILENCE.as_secs()
                            ));
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
        let mut aborted = link.teardown(&end.reason());
        if tokio::time::timeout(END_GRACE, link.0.tasks.wait()).await.is_err() {
            aborted.tasks = link.0.tasks.len();
            link.0.overdue.cancel();
            // Every task ends at the cancellation, each logging its name.
            link.0.tasks.wait().await;
        }
        tracing::info!(
            event = "link.ended",
            host_id = %link.0.device,
            cause = match &end {
                LinkEnd::Closed(_) => "closed_by_runner",
                LinkEnd::Refused(_) => "refused_by_backend",
                LinkEnd::RunnerRefused(_) => "refused_by_runner",
                LinkEnd::Disconnected(_) => "disconnected_by_backend",
            },
            session_ms = began.elapsed().as_millis() as u64,
            frames_in = frames_in.get(),
            frames_out = frames_out.get(),
            host_calls_aborted = aborted.host_calls,
            spawns_aborted = aborted.spawns,
            residents_aborted = aborted.residents,
            tasks_aborted = aborted.tasks,
            "a runner connection's life ended"
        );
        end
    }
}

/// `message` as the frame the connection sends; a message over the wire's
/// limit cannot be one.
fn frame(message: &Inbound) -> Result<wire::Frame, HostError> {
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
    Ok(frame)
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
        return ProcessEnd::NotStarted(SpawnError {
            kind: match error.kind {
                wire::SpawnErrorKind::ExecutableNotFound => SpawnErrorKind::ExecutableNotFound,
                wire::SpawnErrorKind::PermissionDenied => SpawnErrorKind::PermissionDenied,
                wire::SpawnErrorKind::CwdUnusable => SpawnErrorKind::CwdUnusable,
                wire::SpawnErrorKind::IsDirectory => SpawnErrorKind::IsDirectory,
                wire::SpawnErrorKind::Other => SpawnErrorKind::Other,
            },
            detail: error.detail,
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
