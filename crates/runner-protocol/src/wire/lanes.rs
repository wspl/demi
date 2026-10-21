//! Which stream of the link each message travels on
//! (`crates/shared-link/src/lib.rs`).
//!
//! A message has a [`Lane`]: the stream it shares with the messages of the
//! same owner, and a [`Mode`]: how it uses that stream. Both ends derive them
//! from the message alone, so no sender chooses and no receiver has to be
//! told. The link keeps the order of one lane's messages from one end and
//! promises nothing between lanes:
//!
//! - [`LaneKind::Command`]: one command, named by its job, raw process, rpc
//!   call or service stream, from its start to its end, in both directions.
//!   The backend opens the stream with the command's start, and its input goes
//!   on it after the start; the runner's output, its calls and its exit come
//!   back on it. A job's rpc calls travel on the job's lane, so a call reaches
//!   the backend before the job's exit.
//! - [`LaneKind::Request`]: one request and its reply, named by the request's
//!   id, which the reply comes back on ([`Mode::Ask`] and [`Mode::Answer`]); a
//!   request nobody answers is a stream of its own ([`Mode::Tell`]). A
//!   command's stop, follow and release are such requests, so that none waits
//!   behind input the command does not read, and one can reach the runner
//!   before the start it follows: the runner keeps a stop for the start
//!   (`crates/runner/src/connection/owner.rs`).
//! - [`LaneKind::Connection`]: what belongs to no command or request: the
//!   notices, the subscriptions, and the signaling of direct channels (an
//!   answer before the candidates that follow it) and of forwards.
//! - [`LaneKind::Heartbeat`]: the link's own liveness probes, which no message
//!   uses.
//!
//! What must follow a message on another lane is sent after it was answered:
//! a job start that names a manifest goes after the runner reported the
//! manifest installed (`manifest_installed`).

use super::{Inbound, Outbound};

/// What a lane carries; see the module's documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LaneKind {
    Connection,
    Command,
    Request,
    Heartbeat,
}

impl LaneKind {
    /// The kind's byte on the link.
    pub fn code(self) -> u8 {
        match self {
            Self::Connection => 0,
            Self::Command => 1,
            Self::Request => 2,
            Self::Heartbeat => 3,
        }
    }

    /// The kind a byte names.
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Connection),
            1 => Some(Self::Command),
            2 => Some(Self::Request),
            3 => Some(Self::Heartbeat),
            _ => None,
        }
    }
}

/// One stream's name: its kind and the id of its owner, which the
/// connection's lane has none of.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Lane {
    pub kind: LaneKind,
    pub id: String,
}

impl Lane {
    pub fn connection() -> Self {
        Self::of(LaneKind::Connection, "")
    }

    pub fn heartbeat() -> Self {
        Self::of(LaneKind::Heartbeat, "")
    }

    /// The lane of the command `id`.
    pub fn command(id: &str) -> Self {
        Self::of(LaneKind::Command, id)
    }

    fn of(kind: LaneKind, id: &str) -> Self {
        Self {
            kind,
            id: id.to_owned(),
        }
    }
}

/// How a message uses its lane's stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// On the lane's stream, which the end that sends first opens and the
    /// other end answers on: a command's messages and the connection's.
    Stream,
    /// A request that opens a stream and waits for its reply on it.
    Ask,
    /// The reply to an [`Mode::Ask`], on the stream the request came on.
    Answer,
    /// A request nobody answers, on a stream of its own.
    Tell,
}

/// Where a message goes: its lane, how it uses the lane's stream, and whether
/// it is the last its sender puts on that stream, so that the sender's half
/// of the stream ends with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub lane: Lane,
    pub last: bool,
    pub mode: Mode,
}

impl Route {
    fn connection() -> Self {
        Self {
            lane: Lane::connection(),
            last: false,
            mode: Mode::Stream,
        }
    }

    fn command(id: &str, last: bool) -> Self {
        Self {
            lane: Lane::of(LaneKind::Command, id),
            last,
            mode: Mode::Stream,
        }
    }

    fn request(id: &str, mode: Mode) -> Self {
        Self {
            lane: Lane::of(LaneKind::Request, id),
            last: true,
            mode,
        }
    }

    fn ask(id: &str) -> Self {
        Self::request(id, Mode::Ask)
    }

    fn answer(id: &str) -> Self {
        Self::request(id, Mode::Answer)
    }

    /// A request about the command `id` that nobody answers.
    fn tell(id: &str) -> Self {
        Self::request(&format!("tell:{id}"), Mode::Tell)
    }

    /// The request that installs the manifest `hash`.
    fn manifest(hash: &str, mode: Mode) -> Self {
        Self::request(&format!("manifest:{hash}"), mode)
    }
}

/// A message that knows its [`Route`].
pub trait Routed {
    fn route(&self) -> Route;
}

impl Routed for Inbound {
    fn route(&self) -> Route {
        if let Some(id) = self.fs_request_id().or_else(|| self.git_request_id()) {
            return Route::ask(id);
        }
        match self {
            Self::Sync { id }
            | Self::JobRead { id, .. }
            | Self::JobMediaRead { id, .. }
            | Self::LogRead { id, .. }
            | Self::ConversationRelease { id, .. }
            | Self::PackageRelease { id, .. }
            | Self::HttpRequest { id, .. } => Route::ask(id),
            Self::ArtifactLocation { id, .. }
            | Self::NumbersReserved { id, .. }
            | Self::ForwardOpened { id, .. }
            | Self::ForwardRefused { id, .. } => Route::answer(id),
            Self::Prefetch { release, .. } => Route::ask(release),
            Self::Manifest { hash, .. } => Route::manifest(hash, Mode::Ask),
            Self::Spawn { spawn_id, .. }
            | Self::SpawnStdin { spawn_id, .. }
            | Self::SpawnStdinEnd { spawn_id } => Route::command(spawn_id, false),
            Self::JobStart { job_id, .. }
            | Self::JobStdin { job_id, .. }
            | Self::JobStdinEnd { job_id } => Route::command(job_id, false),
            Self::SpawnKill { spawn_id: id, .. }
            | Self::JobKill { job_id: id, .. }
            | Self::JobFollow { job_id: id, .. }
            | Self::JobRelease { job_id: id } => Route::tell(id),
            Self::ServiceOpen { stream_id, .. } => Route::command(stream_id, false),
            Self::RpcStdinPull { call_id }
            | Self::RpcPipes { call_id, .. }
            | Self::RpcOutput { call_id, .. } => Route::command(call_id, false),
            Self::RpcExit { call_id, .. } => Route::command(call_id, true),
            Self::HelloOk { .. }
            | Self::ClaimPending { .. }
            | Self::Claimed { .. }
            | Self::HelloError { .. }
            | Self::Revoked { .. }
            | Self::DirectOffer { .. }
            | Self::DirectDialed { .. }
            | Self::DirectDialRefused { .. }
            | Self::FsSubscribe { .. }
            | Self::FsUnsubscribe { .. }
            | Self::FsRenew { .. }
            | Self::DirectCandidate { .. }
            | Self::DirectPing { .. }
            | Self::DirectClose { .. }
            | Self::Forwards { .. }
            | Self::ForwardRetry { .. }
            | Self::ForwardConnect { .. } => Route::connection(),
            // Matched by the request id above.
            Self::FsReadFile { .. }
            | Self::FsHashFile { .. }
            | Self::FsWriteFile { .. }
            | Self::FsDownloadFile { .. }
            | Self::FsWriteDirectory { .. }
            | Self::FsRemoveAll { .. }
            | Self::FsReadFiles { .. }
            | Self::FsLook { .. }
            | Self::FsExists { .. }
            | Self::FsStat { .. }
            | Self::FsLstat { .. }
            | Self::FsReaddir { .. }
            | Self::FsMkdir { .. }
            | Self::FsRm { .. }
            | Self::FsCp { .. }
            | Self::FsMv { .. }
            | Self::FsChmod { .. }
            | Self::FsSymlink { .. }
            | Self::FsLink { .. }
            | Self::FsReadlink { .. }
            | Self::FsRealpath { .. }
            | Self::FsDirectory { .. }
            | Self::FsUtimes { .. }
            | Self::GitChanges { .. }
            | Self::GitShow { .. }
            | Self::GitBranches { .. }
            | Self::GitLog { .. }
            | Self::GitCommit { .. } => unreachable!("an fs or git call has a request id"),
        }
    }
}

impl Routed for Outbound {
    fn route(&self) -> Route {
        match self {
            Self::FsOk(ok) => Route::answer(&ok.id),
            Self::GitOk(ok) => Route::answer(&ok.id),
            Self::NumbersReserve { id, .. }
            | Self::ArtifactResolve { id, .. }
            | Self::ForwardOpen { id, .. } => Route::ask(id),
            Self::ConversationReleased { id, .. }
            | Self::PackageReleased { id }
            | Self::SyncDone { id, .. }
            | Self::FsError { id, .. }
            | Self::GitError { id, .. }
            | Self::JobMediaRead { id, .. }
            | Self::JobRead { id, .. }
            | Self::LogLines { id, .. }
            | Self::LogError { id, .. }
            | Self::HttpAnswered { id, .. }
            | Self::HttpFailed { id, .. } => Route::answer(id),
            Self::Prefetched { release } | Self::PrefetchFailed { release, .. } => {
                Route::answer(release)
            }
            Self::ManifestInstalled { hash, .. } => Route::manifest(hash, Mode::Answer),
            Self::SpawnOutput { spawn_id, .. } => Route::command(spawn_id, false),
            Self::SpawnExit { spawn_id, .. } => Route::command(spawn_id, true),
            Self::JobOutput { job_id, .. }
            | Self::JobRunningHint { job_id, .. }
            | Self::JobOutliving { job_id, .. }
            | Self::JobMedium { job_id, .. } => Route::command(job_id, false),
            Self::JobExit { job_id, .. } => Route::command(job_id, true),
            // A call's messages share its job's lane, which the runner puts
            // them on (`Frame::in_command`) since only the call names it.
            Self::RpcCall { job_id, .. } => Route::command(job_id, false),
            Self::RpcStdin { call_id, .. }
            | Self::RpcStdinEnd { call_id }
            | Self::RpcCancel { call_id } => Route::command(call_id, false),
            Self::ServiceOpened { stream_id }
            | Self::ServiceError { stream_id, .. } => Route::command(stream_id, false),
            Self::ServiceDone { stream_id, .. } => Route::command(stream_id, true),
            Self::PackageMessage { .. }
            | Self::PackageLost { .. }
            | Self::Revoke {}
            | Self::Hello { .. }
            | Self::JobCount { .. }
            | Self::PipeDone { .. }
            | Self::DirectAnswer { .. }
            | Self::DirectRefused { .. }
            | Self::DirectDial { .. }
            | Self::FsLive { .. }
            | Self::FsChanged { .. }
            | Self::FsFailed { .. }
            | Self::DirectCandidate { .. }
            | Self::DirectPong { .. }
            | Self::DirectStream { .. }
            | Self::DirectStreamWatched { .. }
            | Self::DirectClosed { .. }
            | Self::ForwardState { .. }
            | Self::Installed { .. } => Route::connection(),
        }
    }
}
