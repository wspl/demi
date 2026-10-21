//! The runner wire: MessagePack frames over the runner's WebSocket, which the
//! runner opens outbound to the backend.
//!
//! Both ends decode every message into these types and validate it at entry
//! ([`decode`]). A message that fails closes the connection, since its sender
//! broke the protocol, and the closing side names the message's type and the
//! decoding error in the close frame's reason ([`refusal`]).
//!
//! ```text
//!   runner                                              backend
//!     | GET /api/runner + release, target, token headers --> |  release check
//!     | <---------------------- 409 + RunnerUpdate            |  (release module)
//!     | ==================== WebSocket ====================== |
//!     | hello ---------------------------------------------> |  token lookup
//!     | <------------------------- hello_ok or hello_error    |
//!     | ================= yamux session (the link) ============ |
//! ```
//!
//! The hello and its answer are ordinary WebSocket messages, one message
//! each, as before the link: a runner of an earlier version is answered
//! `unsupported_protocol` the way it always was. The yamux session starts on
//! the same socket once the backend accepted the hello (`hello_ok`, or
//! `claimed` for a runner just paired); a runner that waits for its user to
//! claim it is told `claim_pending` meanwhile.
//!
//! # The link
//!
//! The WebSocket carries one yamux session, the link
//! (`crates/shared-link/src/lib.rs`), and the messages keep their encoding.
//! A message names its stream, its [`Lane`], by its type ([`Routed`]):
//!
//! - A command is a stream, named by its job, raw process, rpc call or service
//!   stream. The backend opens it with the command's start, and the command's
//!   input follows on it; the runner sends the command's output, its rpc calls
//!   and its exit back on it. A stop, a follow or a release is a request on a
//!   stream of its own, so none waits behind input nobody reads.
//! - A request is a stream too: the request one way, its reply the other.
//! - Each end has one stream for what belongs to no command or request: the
//!   notices, the subscriptions, and the signaling of direct channels and
//!   forwards.
//!
//! A stream has a window of its own, so a stream that is full holds only its
//! own sender; a command's starts large, so its first output crosses in a
//! round trip.
//! The order of one stream's messages is the order they were sent, and the
//! link promises none between streams. What must follow another message is
//! sent after it was answered: the backend starts a job that names a manifest
//! after the runner reported that manifest installed (`manifest_installed`).
//!
//! A command's input is flow-controlled by the stream's window alone: a command
//! that reads no input fills the window of its own stream, which stalls its
//! writer and nothing else, since a stop is a request on a stream of its own.
//!
//! The link also tells whether the other end is there, with a heartbeat on a
//! stream of its own and any byte heard counting as the peer's presence: an
//! end that hears nothing for [`RUNNER_PING_TIMEOUT`] has lost its connection.
//!
//! # A second hello
//!
//! A hello for a device that already has a connection is more often the same
//! runner coming back than a second one, since a network that drops a
//! connection without closing it leaves the backend holding a socket nobody
//! answers. So the backend pings the held connection's link and waits up to 5
//! seconds:
//!
//! - A connection that answers keeps the device, and the new hello is refused
//!   with `already_connected`. A connection whose liveness checks are paused,
//!   as a Cloud's are while it is checkpointed, counts as answering.
//! - A connection that does not answer is closed, and the new hello is
//!   answered.
//!
//! A laptop whose Wi-Fi blinked reconnects within about 5 seconds instead of
//! being refused until the heartbeat gives the old socket up.
//!
//! # A connection's end
//!
//! A connection's end ends everything it carries. The backend's work for it,
//! such as an `rpc` handler committing its change, gets [`END_GRACE`] to finish
//! and the rest is aborted. Only then is the device offline and a new hello for
//! it answered.
//!
//! Otherwise a `demi browser` call whose handler waits for the very device its
//! connection served would keep that connection from ending, and every new
//! connection of the device would wait behind it until the backend restarted.
//!
//! # Message size
//!
//! A message is at most [`MAX_MESSAGE_BYTES`], eight times the largest regular
//! message (a 5,000-file working tree list, a shell command as long as a model
//! can write, about 0.5 MiB each).
//!
//! A message larger than a stream's window crosses in pieces, and a stream
//! that waits for room holds only its own sender. A receiver holds at most
//! 32 MiB of messages the application has not taken, however many streams
//! carry them; a peer that opens more streams than a link has, or sends a
//! frame longer than a message may be, loses its link.
//!
//! Bulk bytes still never travel in a message: file contents and command IO go
//! through pipes ([`PipeFrame`]). A full stream never closes the connection,
//! only a message that breaks the protocol does.

mod encoding;
mod kept;
mod lanes;
mod messages;
mod pipe_frames;
mod replies;

pub use encoding::{Timestamp, WireBytes};
pub use kept::{
    JOB_KEPT_BYTES, JOB_KEPT_PART_BYTES, JOB_KEPT_READ_BYTES, KeptRecord, decode_records,
    encode_record,
};
pub use lanes::{Lane, LaneKind, Mode, Route, Routed};
pub use messages::{
    ArtifactOwner, AutostartUnavailable, ChangeKind, Descriptor, DirEntry, DirectoryFile, FileHash, FileRead, FileStat, OpenedFile, file_version, GitBranches, GitChange, GitChanges,
    GitCommit, GitCommitEntry, GitCommitFile, GitLog, GitLogEntry, commit_time,
    HelloErrorCode, HostArtifact, HostIdentity, Inbound, JobArtifactOwner, JobFileChange, JobPathChange, KeptEnd, KeptJob, Kernel, LinkTarget, LogLine,
    LookPath, Looked, MAX_INSTALLED, SymbolicLink, OperatingSystem, Outbound, OutputLengths, OutputStream, PipeRef, FileOffer,
    RunnerInfo, RunnerPlatform, ServiceErrorCode, Signal, SpawnError, SpawnErrorKind,
    StreamArtifactOwner, WriteExists,
};
pub use demi_shared_types::ReadAt;
pub use pipe_frames::{
    PIPE_DATA_BYTES, PIPE_IDLE, PipeEnd, PipeFrame, PipeFrameDecoder, PipeFrameError,
    encode_data, end_frame, idle_frame,
};
pub use replies::{FsOk, FsResult, GitOk, GitResult};

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// The wire's version, which a runner's hello names. A hello of another
/// version is refused, whatever release headers the socket opened with.
pub const VERSION: u32 = 56;
/// How long a stopped job has after `TERM` before a stop sends `KILL`, the
/// backend's stop and the runner's own alike (`crates/runner-shell/src/job.rs`).
pub const STOP_GRACE: Duration = Duration::from_secs(5);
/// How often each end of the link pings the other, and how long a link that
/// is heard from at all counts as lost
/// (`crates/shared-link/src/lib.rs`).
pub const RUNNER_PING_INTERVAL: Duration = Duration::from_secs(30);
pub const RUNNER_PING_TIMEOUT: Duration = Duration::from_secs(60);
/// How often a runner reports how many jobs it runs, besides when it comes
/// online (`crates/runner/src/connection/owner.rs`).
pub const JOB_COUNT_INTERVAL: Duration = Duration::from_secs(30);
/// The most background tasks a `job_outliving` names, and the most bytes of
/// each one's command line: enough to tell a model what keeps its command
/// running without crowding its output out of the result.
pub const JOB_OUTLIVING_TASKS: usize = 32;
pub const JOB_TASK_LINE_BYTES: usize = 256;
/// The largest frame either end sends.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
/// How much of the start of each stream a job always sends, and how much of
/// its newest bytes beyond them it sends while nobody follows it: the model's
/// view of a running command (`crates/runner-jobs/src/tasks.rs`).
pub const JOB_VIEW_BYTES: usize = 8 * 1024;
/// The most bytes one message beyond a stream's first [`JOB_VIEW_BYTES`]
/// carries while the backend follows the job: 4,096 characters of up to four
/// bytes each.
pub const JOB_LIVE_BYTES: usize = 16 * 1024;
/// How often a followed job sends each stream's newest bytes beyond its
/// first [`JOB_VIEW_BYTES`], at most.
pub const JOB_LIVE_INTERVAL: Duration = Duration::from_millis(250);
/// How often a job nobody follows sends how long a stream grew beyond its
/// first [`JOB_VIEW_BYTES`] and its newest [`JOB_VIEW_BYTES`], at most.
pub const JOB_GROWTH_INTERVAL: Duration = Duration::from_secs(2);
/// The most bytes of one binary message of a stream pipe's WebSocket
/// (`crates/runner-host/src/lib.rs`): the runner splits larger writes.
pub const STREAM_PIPE_MESSAGE_BYTES: usize = 64 * 1024;
/// The most bytes of a WebSocket close frame's reason.
const CLOSE_REASON_BYTES: usize = 123;
/// The most bytes of one live stdin frame.
pub const STDIN_CHUNK_BYTES: usize = 64 * 1024;
/// How long the runner gathers the subscriptions that changed before it
/// reports them, each once (`crates/runner-host/src/subscriptions.rs`).
pub const WATCH_GATHER: Duration = Duration::from_millis(100);
/// How long a subscription's lease runs from the `fs_subscribe` or the
/// `fs_renew` that started it, as each end counts it (`crates/runner-host/src/subscriptions.rs`).
pub const SUBSCRIPTION_LEASE: Duration = Duration::from_secs(10 * 60);
/// The most subscriptions one `fs_changed` names; more changed at once go
/// in as many messages as they take, within the frame limit.
pub const CHANGES_PER_MESSAGE: usize = 16 * 1024;
/// The most lines one `log_read` returns.
pub const LOG_READ_LINES: usize = 1000;
/// The most of an invocation's standard error a `service_done` carries, in
/// Unicode scalar values (`crates/shared-types/src/lib.rs`). The runner keeps the
/// last this many bytes, which are never more characters.
pub const SERVICE_STDERR_CHARS: usize = 16 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error(transparent)]
    Decode(#[from] rmp_serde::decode::Error),
    #[error(transparent)]
    Encode(#[from] rmp_serde::encode::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("invalid runner message: {0}")]
    Invalid(String),
}

/// An encoded message, with the stream of the link it travels on.
#[derive(Clone)]
pub struct Frame {
    bytes: Vec<u8>,
    route: Route,
}

impl Frame {
    /// Bytes that are no message, for a test of a peer that refuses them;
    /// they travel on the connection's lane.
    pub fn raw(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            route: Route {
                lane: Lane::connection(),
                last: false,
                mode: Mode::Stream,
            },
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// The encoded size, which [`MAX_MESSAGE_BYTES`] bounds.
    pub fn encoded_len(&self) -> usize {
        self.bytes.len()
    }

    /// Where the message goes.
    pub fn route(&self) -> &Route {
        &self.route
    }

    /// The same message on the lane of the command `id`, for a message whose
    /// sender knows an owner the message does not name: an rpc call's
    /// messages travel on their job's lane, after the job's output and
    /// before its exit.
    pub fn in_command(mut self, id: &str) -> Self {
        self.route.lane = Lane::command(id);
        self
    }

    /// A frame of an encoded message and its route, which `into_parts` took.
    pub fn from_parts(bytes: Vec<u8>, route: Route) -> Self {
        Self { bytes, route }
    }

    /// The encoded message and its route.
    pub fn into_parts(self) -> (Vec<u8>, Route) {
        (self.bytes, self.route)
    }
}

/// Validates and encodes a message. The size limit is the sender's to apply
/// ([`within_limit`]), because an oversized reply fails its own request.
pub fn encode<M: Serialize + Routed + garde::Validate<Context = ()>>(
    message: &M,
) -> Result<Frame, WireError> {
    message
        .validate()
        .map_err(|report| WireError::Invalid(report.to_string()))?;
    Ok(Frame {
        bytes: rmp_serde::to_vec_named(message)?,
        route: message.route(),
    })
}

/// Decodes and validates one message; bytes after it are refused.
pub fn decode<M: DeserializeOwned + garde::Validate<Context = ()>>(
    bytes: &[u8],
) -> Result<M, WireError> {
    let mut decoder = rmp_serde::Deserializer::new(std::io::Cursor::new(bytes));
    let message = M::deserialize(&mut decoder)?;
    if decoder.position() != bytes.len() as u64 {
        return Err(WireError::Invalid("trailing MessagePack data".into()));
    }
    message
        .validate()
        .map_err(|report| WireError::Invalid(report.to_string()))?;
    Ok(message)
}

/// Why a message cannot be decoded, naming its type and the decoding error,
/// as the side that closes the connection over it says
///: `cannot decode job_start:
/// unknown field `shell``. A message whose type cannot be read is named
/// as a message.
pub fn refusal(bytes: &[u8], error: &WireError) -> String {
    /// The tag every message of the wire carries.
    #[derive(serde::Deserialize)]
    struct Typed {
        #[serde(rename = "type")]
        kind: String,
    }
    match rmp_serde::from_slice::<Typed>(bytes) {
        Ok(Typed { kind }) => format!("cannot decode {kind}: {error}"),
        Err(_) => format!("cannot decode a message: {error}"),
    }
}

/// `reason` as a WebSocket close frame carries it: its first
/// [`CLOSE_REASON_BYTES`], cut at a character's boundary.
pub fn close_reason(reason: &str) -> &str {
    &reason[..reason.floor_char_boundary(CLOSE_REASON_BYTES)]
}

/// A reply over [`MAX_MESSAGE_BYTES`] fails its own request instead of the
/// connection: `refuse` builds that
/// request's error from the reason.
pub fn within_limit(
    reply: Frame,
    refuse: impl FnOnce(String) -> Result<Frame, WireError>,
) -> Result<Frame, WireError> {
    if reply.encoded_len() <= MAX_MESSAGE_BYTES {
        return Ok(reply);
    }
    refuse(format!(
        "the reply is {} bytes, over the {}-byte message limit",
        reply.encoded_len(),
        MAX_MESSAGE_BYTES
    ))
}
