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
    rc::{Rc, Weak},
    time::Duration,
};

use bytes::Bytes;
use demi_command_protocol::{ArtifactLocation, CommandContext, PackageDescriptor, ServiceSequence};
use demi_host_interface::{
    HostError, HostErrorKind, HostIdentity, HostKey, JobCaller, ProcessEnd, ProcessOutput,
    RpcError, RpcInvocation, RpcPort, SpawnError, SpawnErrorKind,
};
use demi_runner_protocol::direct::{Introduction, OfferRefusal, StunUrl};
use demi_runner_protocol::wire::{
    self, ArtifactOwner, FileRead, FsResult, GitResult, HostArtifact, Inbound, LogLine, Outbound,
    VolumeName,
};
use demi_shared_gates::{GateLease, SerialGate};
use demi_shared_types::{BlobRef, StreamKind};
use futures_util::{Sink, SinkExt, Stream, StreamExt, future::LocalBoxFuture};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    device_jobs::{DeviceJobs, Hello},
    file_watch::LinkWatches,
    output_records::{decode_output, stream_kind},
    pipes::Pipes,
    relay::{CallEntry, Stop},
};

/// How often the backend asks a runner whether it is there.
pub const PING_INTERVAL: Duration = Duration::from_secs(30);

/// The frames the backend queues for a runner before a sender waits.
pub const OUTBOUND_FRAMES: usize = 64;

/// The artifact requests a connection answers at a time.
const ARTIFACT_REQUESTS: usize = 32;
/// Why an artifact request names no work the backend knows.
const NO_LIVE_WORK: &str = "No matching live job or stream";
/// The numbers requests a connection answers at a time.
const NUMBERS_REQUESTS: usize = 32;

/// How long a job the backend has no command for has after `TERM` before
/// it is killed (`runner.md` § Cancellation and completion).
const UNKNOWN_KILL_AFTER: Duration = Duration::from_secs(5);

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

    /// The bytes of the blob `blob` of the user's namespace, which a
    /// handler returns as a medium (`commands.md` § Return media); none when
    /// the namespace does not hold it.
    fn read_blob(&self, blob: BlobRef) -> LocalBoxFuture<'static, Result<Option<Bytes>, String>>;

    /// A managed guest asks for a larger volume.
    fn grow_volume(
        &self,
        volume: VolumeName,
        bytes: u64,
    ) -> LocalBoxFuture<'static, Result<(), String>>;

    /// The device's runner asks that the device be revoked, as `run
    /// uninstall` does (`runner.md` § Installation, pairing and removal):
    /// once it is, its connection ends with `revoked`; or why it could not
    /// be.
    fn revoke_device(&self) -> LocalBoxFuture<'static, Result<(), String>>;

    /// A job the backend recorded running, which no agent has taken up yet,
    /// makes an `rpc` call: the agent that ran it takes it up, as a report
    /// of it does (`runtime.md` § Command reports), so the call is served.
    /// Answers once the take-up was asked for, or why it cannot be.
    fn take_up_job(&self, job: String) -> LocalBoxFuture<'static, Result<(), String>>;

    /// A native service on the device asks for `count` numbers of
    /// `conversation`'s `sequence` (`native-runtime.md` § Conversation
    /// numbers): the first of them, reserved, or why there are none.
    fn reserve_numbers(
        &self,
        conversation: String,
        sequence: ServiceSequence,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<u64, String>>;

    /// A page's direct channel opened the user stream `name` of
    /// `conversation` on the device (`direct-channel.md` § Operations on the
    /// channel): what the backend knows the stream with while it is open, as
    /// a stream of that name it opened, or why it does not know it.
    fn direct_stream(
        &self,
        conversation: String,
        name: String,
    ) -> LocalBoxFuture<'static, Result<DirectAdmission, String>>;
}

/// What the backend knows an open direct stream with: the lease that holds
/// its conversation active, as an open relay stream's does, and what a relay
/// stream of its name may install: the executable of the package the name
/// binds, none for a name that binds none, from where `resolver` locates it
/// (`native-runtime.md` § Install artifacts).
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
    pub pipes: Pipes,
    pub policy: Rc<dyn LinkPolicy>,
    /// How often to ping; none turns liveness off.
    pub ping: Option<Duration>,
    /// The device's jobs, which outlive the connection.
    pub jobs: DeviceJobs,
    /// What the runner's hello says of its jobs, which the connection takes
    /// up (`runner.md` § Command lifetime).
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
    /// why (`runner.md` § Connection and identity).
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
    identity: HostIdentity,
    /// The device's jobs, which outlive the connection.
    device_jobs: DeviceJobs,
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
    /// Changes with each pong, which a probe of the connection waits for.
    pongs: watch::Sender<()>,
    /// What the runner last reported its artifact cache holds; none until
    /// it reports.
    installed: watch::Sender<Option<Vec<HostArtifact>>>,
}

#[derive(Default)]
struct State {
    /// What waits for a reply, by request or stream id.
    waiting: HashMap<String, Waiting>,
    spawns: HashMap<String, SpawnEntry>,
    services: HashMap<String, ServiceEntry>,
    calls: HashMap<String, Rc<CallEntry>>,
    artifact_requests: HashSet<String>,
    numbers_requests: HashSet<String>,
    /// The manifest this connection last carried, which later jobs share.
    manifest: Option<String>,
    /// The runner's own count of its jobs, from its last pong.
    pong_jobs: u64,
    /// The file watches the connection's Hosts follow.
    watches: LinkWatches,
    /// The direct streams open on the device, by the id the runner gave
    /// each.
    direct_streams: HashMap<String, DirectStream>,
    /// Where what the runner says of each peer goes: to the page's
    /// signaling socket, by the peer's id.
    direct_peers: HashMap<String, mpsc::UnboundedSender<PeerEvent>>,
}

/// A direct stream the runner reported open: what its close cancels, the
/// policy's admission of it, and once admitted the lease that holds its
/// conversation active.
struct DirectStream {
    cancel: CancellationToken,
    admitted: watch::Sender<Admitted>,
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
    Service,
    JobRead,
    JobMediaRead,
    DirectOffer,
}

pub(crate) enum Answer {
    Fs(FsResult),
    Git(GitResult),
    Log { lines: Vec<LogLine>, next: u64 },
    /// Which files of a read of several were read.
    Read(Vec<FileRead>),
    /// The runner's answer to a page's offer.
    Direct(DirectAnswer),
    Done,
}

/// What the runner says of a page's peer besides its answer
/// (`direct-channel.md` § Making the channel, § Measuring the paths).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerEvent {
    /// A candidate it found after its answer.
    Candidate(String),
    /// Its answer to the page's relay probe.
    Pong(u32),
}

/// What a runner said to a page's offer (`direct-channel.md` § Making the
/// channel).
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
}

struct SharedState<E, C> {
    output: VecDeque<C>,
    end: Option<E>,
    /// The running declared commands' hints, first registered first.
    hints: Vec<(String, String)>,
}

/// One message of a job's output (`runner.md` § Pipes and output): the
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
            }),
            changed: watch::Sender::new(0),
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
    pub path_changes: Vec<demi_command_protocol::PathChange>,
    pub files_truncated: bool,
    /// Why the job counts as lost although its runner kept its output: it
    /// stopped the job once its connection stayed away (`runner.md`
    /// § Command lifetime).
    pub lost: Option<String>,
}

impl JobEnd {
    pub(crate) fn lost(reason: &str) -> Self {
        Self {
            status: ProcessEnd::Lost(reason.into()),
            output: None,
            files: Vec::new(),
            path_changes: Vec::new(),
            files_truncated: false,
            lost: None,
        }
    }
}

/// A medium a job's command returned to the job, as the runner announced it
/// once it kept it (`runner.md` § Pipes and output).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobMedium {
    pub number: u32,
    pub media_type: String,
    pub size: u64,
    /// The SHA-256 of its bytes, which names its blob.
    pub sha256: BlobRef,
    /// A document's name, and an image's or video's size in pixels and a
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

pub(crate) struct ServiceEntry {
    pub(crate) package: PackageDescriptor,
    pub(crate) resolver: Rc<dyn crate::ArtifactResolver>,
    pub(crate) attached: Vec<crate::AttachedArtifact>,
    pub(crate) cancel: CancellationToken,
    pub(crate) done: Option<oneshot::Sender<Result<crate::ServiceEnd, HostError>>>,
}

impl Link {
    pub fn downgrade(&self) -> WeakLink {
        WeakLink(Rc::downgrade(&self.0))
    }

    /// A connection, and the driver its owner serves the socket with. The
    /// jobs its runner's hello lists go on over it, are stopped when the
    /// backend has no command for them, or are lost (`runner.md` § Command
    /// lifetime).
    pub fn new(options: LinkOptions) -> (Link, LinkDriver) {
        let (outbound, frames) = mpsc::channel(OUTBOUND_FRAMES);
        let link = Link(Rc::new(Inner {
            device: options.device,
            identity: options.identity,
            device_jobs: options.jobs.clone(),
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
            installed: watch::Sender::new(None),
            pongs: watch::Sender::new(()),
        }));
        let adoption = options.jobs.adopt(&link, options.hello);
        for job in adoption.stop {
            link.stop_unknown(job);
        }
        for (job, lengths) in adoption.resync {
            let reading = link.clone();
            link.spawn(async move { reading.resync(&job, lengths).await });
        }
        let driver = LinkDriver {
            link: link.clone(),
            frames,
            ping: options.ping,
            tap: None,
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
        self.spawn(async move {
            tokio::select! {
                () = link.ended() => {}
                () = tokio::time::sleep(UNKNOWN_KILL_AFTER) => link.post(&Inbound::JobKill {
                    job_id: job,
                    signal: Some(wire::Signal::Kill),
                }),
            }
        });
    }

    /// Reads the kept output of `job`, which goes on over this connection,
    /// and delivers what its consumer did not receive: the output the
    /// runner printed while no connection served (`runner.md` § Command
    /// lifetime). A read that fails delivers only what arrives from now on.
    async fn resync(&self, job: &str, lengths: wire::OutputLengths) {
        let read = async {
            let reader = crate::remote_host::filled_job_read(self, job).await?;
            let bytes = crate::remote_host::collect_pipe(reader, wire::JOB_KEPT_READ_BYTES).await?;
            decode_output(&bytes, None).map_err(|error| {
                HostError::new(
                    HostErrorKind::Protocol,
                    format!("the job's kept output does not decode: {error}"),
                )
            })
        };
        match read.await {
            Ok(output) => self.0.device_jobs.resynced(job, Some(&output), lengths),
            Err(error) => {
                tracing::info!(device = %self.0.device, %job, "the job's kept output was not read again: {error}");
                self.0.device_jobs.resynced(job, None, lengths);
            }
        }
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
    /// What the runner reports its artifact cache holds, from now on
    /// (`native-runtime.md` § Installed artifacts): none until its first
    /// report.
    pub fn watch_installed(&self) -> watch::Receiver<Option<Vec<HostArtifact>>> {
        self.0.installed.subscribe()
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

    /// Starts liveness again, with no ping outstanding.
    pub fn resume_liveness(&self) {
        self.0.liveness.set(Liveness::Idle);
    }

    /// Whether the runner answers a ping within `within`, as a new hello for
    /// the device asks of the connection that holds it (`runner.md`
    /// § Connection and identity): any pong that arrives meanwhile is an
    /// answer. A connection that is closing does not answer; one whose
    /// liveness is paused does, since its silence is no death.
    pub async fn answers(&self, within: Duration) -> bool {
        if self.0.liveness.get() == Liveness::Paused {
            return true;
        }
        // Subscribing marks the pongs so far as seen.
        let mut pongs = self.0.pongs.subscribe();
        let asked = async {
            if self.send(&Inbound::Ping {}).await.is_err() {
                return false;
            }
            // The connection holds the sender, so the watch never closes.
            pongs.changed().await.is_ok()
        };
        tokio::select! {
            answered = tokio::time::timeout(within, asked) => answered.unwrap_or(false),
            () = self.ended() => false,
        }
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

    /// Introduces a page to the runner with its offer for peer `peer`,
    /// which replaces the peer of that id, and waits for the runner's
    /// answer; the caller bounds the wait. The runner asks the STUN servers
    /// `stun` for its public addresses; what it finds after its answer comes
    /// through what [`Link::direct_candidates`] answers.
    pub async fn direct_offer(
        &self,
        peer: &str,
        sdp: String,
        introduction: Introduction,
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

    /// The device's jobs, which outlive the connection.
    pub(crate) fn device_jobs(&self) -> &DeviceJobs {
        &self.0.device_jobs
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

    /// Sends `frame` to the runner as it is, for a test of a message the
    /// runner cannot decode.
    #[cfg(feature = "testing")]
    pub async fn send_raw(&self, frame: Vec<u8>) -> Result<(), HostError> {
        self.send_frame(frame).await
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

    pub(crate) fn with_watches<T>(&self, f: impl FnOnce(&mut LinkWatches) -> T) -> T {
        f(&mut self.0.state.borrow_mut().watches)
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
                self.0.device_jobs.with(&job_id, |job| {
                    job.media.borrow_mut().push(JobMedium {
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
            Outbound::SyncDone { id, error } => match error {
                None => self.answer(&id, Expected::Sync, Answer::Done),
                Some(error) => self.refuse(&id, HostError::failed(None, error)),
            },
            // A repeated hello on a bound connection says nothing new.
            Outbound::Hello { .. } => {}
            Outbound::Installed { artifacts } => {
                self.0.installed.send_replace(Some(artifacts));
            }
            Outbound::Pong { jobs } => {
                self.0.state.borrow_mut().pong_jobs = jobs;
                if self.0.liveness.get() == Liveness::Waiting {
                    self.0.liveness.set(Liveness::Idle);
                }
                self.0.pongs.send_replace(());
            }
            Outbound::VolumeGrow { id, volume, bytes } => self.grow_volume(id, volume, bytes),
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
            Outbound::DirectCandidate { peer, candidate } => {
                self.tell_peer(&peer, PeerEvent::Candidate(candidate));
            }
            Outbound::DirectPong { peer, id } => self.tell_peer(&peer, PeerEvent::Pong(id)),
            Outbound::DirectRefused { id, code, message } => {
                let answer = Answer::Direct(DirectAnswer::Refused(code, message));
                self.answer(&id, Expected::DirectOffer, answer);
            }
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
            Outbound::FsWatchReady { id } => self.with_watches(|watches| watches.ready(&id)),
            Outbound::FsWatchChanged { id, paths, entries, ignored } => {
                self.with_watches(|watches| watches.changed(&id, paths, entries, ignored));
            }
            Outbound::FsWatchLost { id } => self.with_watches(|watches| watches.lost(&id)),
            Outbound::FsWatchFailed { id, reason } => {
                self.with_watches(|watches| watches.failed(&id, reason));
            }
            Outbound::LogLines { id, lines, next } => {
                self.answer(&id, Expected::Log, Answer::Log { lines, next });
            }
            Outbound::LogError { id, message } => {
                self.refuse(&id, HostError::failed(None, message))
            }
            Outbound::ServiceOpened { stream_id } => {
                self.answer(&stream_id, Expected::Service, Answer::Done);
            }
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
                let end = JobEnd {
                    status: process_end(exit_code, signal, spawn_error),
                    output,
                    files,
                    path_changes,
                    files_truncated,
                    lost: None,
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
            Outbound::NumbersReserve {
                id,
                conversation_id,
                sequence,
                count,
            } => self.reserve_numbers(id, conversation_id, sequence, count),
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

    /// Revokes the device at its runner's request. A revoked device's
    /// connection ends with `revoked`, which its end sends. A revocation
    /// that failed is answered with nothing: the runner, which waits a
    /// bounded time, removes itself all the same.
    fn revoke(&self) {
        let revocation = self.0.policy.revoke_device();
        let device = self.0.device.clone();
        self.0.tasks.spawn_local(async move {
            if let Err(error) = revocation.await {
                tracing::warn!(device = %device, "the runner's revocation failed: {error}");
            }
        });
    }

    /// Answers the runner's request for an executable's location, for the
    /// live work that runs it and only while it does
    /// (`native-runtime.md` § Install artifacts).
    fn resolve_artifact(&self, id: String, owner: ArtifactOwner, sha256: String, target: String) {
        let granted = self.grant(&owner);
        let refusal = match &granted {
            None => Some(NO_LIVE_WORK),
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
    /// streams the backend opened (`direct-channel.md` § Operations on the
    /// channel): each opening asks the policy to admit the stream, and once
    /// admitted the stream holds its conversation active and its artifact
    /// requests are answered; its closing, or the connection's end, ends both
    /// and cancels the requests in flight.
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
                tracing::warn!(device = %self.0.device, %stream, "a direct stream was reported open twice");
                return;
            }
            let opened = DirectStream {
                cancel: cancel.clone(),
                admitted: watch::Sender::new(Admitted::Waiting),
                lease: None,
            };
            state.direct_streams.insert(stream.clone(), opened);
        }
        let admission = self.0.policy.direct_stream(conversation.clone(), name);
        let link = self.clone();
        self.spawn(async move {
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
                    open.lease = Some(admission.lease);
                    open.admitted.send_replace(Admitted::Yes(Grant {
                        packages: admission.package.into_iter().collect(),
                        resolver: admission.resolver,
                        attached: Vec::new(),
                        cancel,
                    }));
                }
                Err(error) => {
                    tracing::warn!(device = %link.0.device, %conversation, "a direct stream was not admitted: {error}");
                    open.admitted.send_replace(Admitted::No);
                }
            }
        });
    }

    /// Answers a native service's request for conversation numbers, which
    /// the policy decides (`native-runtime.md` § Conversation numbers).
    fn reserve_numbers(
        &self,
        id: String,
        conversation: String,
        sequence: ServiceSequence,
        count: u32,
    ) {
        let admitted = {
            let mut state = self.0.state.borrow_mut();
            state.numbers_requests.len() < NUMBERS_REQUESTS
                && state.numbers_requests.insert(id.clone())
        };
        let reserved =
            admitted.then(|| self.0.policy.reserve_numbers(conversation, sequence, count));
        let link = self.clone();
        self.0.tasks.spawn_local(async move {
            let first = match reserved {
                None => Err("Numbers request limit or duplicate id".to_owned()),
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
            if let Err(error) = link.send(&answer).await {
                tracing::warn!(device = %link.0.device, "numbers answer not sent: {error}");
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
            ArtifactOwner::Stream(owner) => {
                if let Some(direct) = state.direct_streams.get(&owner.stream_id) {
                    return Some(Granted::Admitting(direct.admitted.subscribe()));
                }
                let service = state.services.get(&owner.stream_id)?;
                Some(Granted::Now(Grant {
                    packages: vec![service.package.clone()],
                    resolver: service.resolver.clone(),
                    attached: service.attached.clone(),
                    cancel: service.cancel.clone(),
                }))
            }
        }
    }

    /// Ends everything in flight on the connection: replies fail as offline,
    /// processes end as lost, calls stop, and the device's pipes fail; its
    /// jobs wait for the device's next connection (`runner.md` § Command
    /// lifetime). Idempotent.
    pub(crate) fn teardown(&self, reason: &str) {
        if self.0.end.borrow().is_some() {
            return;
        }
        *self.0.end.borrow_mut() = Some(reason.into());
        self.0.closed.cancel();
        let mut state = std::mem::take(&mut *self.0.state.borrow_mut());
        state.watches.end();
        for (_, waiting) in state.waiting {
            // A requester that gave up waits for nothing.
            let _ = waiting.answer.send(Err(HostError::offline(reason)));
        }
        self.0.device_jobs.detach(self);
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
        I: Stream<Item = Result<Vec<u8>, SocketEnd>>,
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
                    Some(Err(SocketEnd::Broken(error))) => {
                        return LinkEnd::Closed(format!("runner disconnected: {error}"));
                    }
                    Some(Err(SocketEnd::Refused(refusal))) => {
                        return LinkEnd::RunnerRefused(refusal);
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
                    Err(error) => return LinkEnd::Refused(wire::refusal(&frame, &error)),
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
        link.teardown(&end.reason());
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
