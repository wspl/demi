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
//! ```
//!
//! # A second hello
//!
//! A hello for a device that already has a connection is more often the same
//! runner coming back than a second one, since a network that drops a
//! connection without closing it leaves the backend holding a socket nobody
//! answers. So the backend pings the held connection and waits up to 5 seconds:
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
//! A hello that carries the runner's session on ([`Resume`]) is the same runner
//! coming back to the same start of the backend, so the held connection gives
//! way at once without the ping: the runner already counts it lost.
//!
//! # The session
//!
//! The link is a session that outlives its connections ([`Session`]). Each
//! direction numbers the messages it writes, where it writes them to the
//! connection, and keeps each until the other end confirms it. The runner's
//! hello says which session and how many of its messages the runner
//! received (`resume`), and the backend's answer names the session it carries
//! (`session`) and says how many of the runner's it received; a name other than
//! the one the runner holds is a new session. Each end then writes again what
//! follows.
//!
//! - The hello, its answer, the ping and the pong belong to the connection and
//!   carry no number.
//! - A message counts as received once its end handled it, so one read and then
//!   dropped as the connection ended is written again.
//! - What waits while the other end is gone depends on the message. A call's
//!   messages wait in a lane for the next connection. A command's output,
//!   previews, file changes and the Host's log lines are not queued, as the next
//!   connection reads what it needs from the Host's kept output
//!   (`crates/runner/src/connection/kept.rs`); sent again, a message of output
//!   is harmless, since the backend takes output by its offset.
//! - A Host request in flight fails as offline when the connection drops, as
//!   it does without a session.
//! - A different runner start or backend start begins a new session: what the
//!   old one kept is dropped, and its calls end with the reason.
//!
//! # Two classes of message
//!
//! Every message the backend sends is control or input ([`Inbound::is_input`]):
//! input is a stream of bytes someone wrote, a command's or a process's stdin
//! and the stderr of a backend command relayed to its caller with its exit;
//! control is everything else. The backend writes control before input, so a
//! stop, a ping or a small request waits only for the frame being written.
//! Nothing on the link has a queue limit, and nothing is cancelled or dropped
//! for one: what crosses is control and typed or model-written input, never
//! bulk bytes, so a queue grows only as fast as people and models write, and a
//! connection that makes no progress is ended ([`LINK_SILENCE`],
//! [`LINK_WRITE_TIMEOUT`]) with what it queued.
//!
//! # A connection's end
//!
//! A connection's end ends what it carries but the device's session: a Host
//! request fails as offline, a process ends as lost, and an `rpc` call goes on,
//! since it belongs to the session. The backend's work for the connection, such
//! as a conversation's release, gets [`END_GRACE`] to finish and the rest is
//! aborted. Only then is the device offline and a new hello for it answered.
//!
//! The calls of a session end with its end, with their job, or with the device.
//! A call that was stopped has [`END_GRACE`] too, for a handler committing its
//! change.
//!
//! # Message size
//!
//! A message is at most [`MAX_MESSAGE_BYTES`], eight times the largest regular
//! message (a 5,000-file working tree list, a shell command as long as a model
//! can write, about 0.5 MiB each).
//!
//! One connection carries everything, so a message in flight holds up the rest:
//! at most 4 MiB of it, however slow the uplink, while its bytes move.
//!
//! Bulk bytes therefore never travel in a message: file contents and command IO
//! go through pipes ([`PipeFrame`]). Only a message that breaks the protocol
//! closes the connection.

mod encoding;
mod kept;
mod messages;
mod pipe_frames;
mod replies;
mod session;
mod write_watch;

pub use encoding::{Timestamp, WireBytes};
pub use kept::{
    JOB_KEPT_BYTES, JOB_KEPT_PART_BYTES, JOB_KEPT_READ_BYTES, KeptRecord, decode_records,
    encode_record,
};
pub use messages::{
    ArtifactOwner, AutostartUnavailable, ChangeKind, Descriptor, DirEntry, DirectoryFile, FileHash, FileRead, FileStat, OpenedFile, file_version, GitBranches, GitChange, GitChanges,
    GitCommit, GitCommitEntry, GitCommitFile, GitLog, GitLogEntry, commit_time,
    HelloErrorCode, HostArtifact, HostIdentity, Inbound, JobArtifactOwner, ProgramsArtifactOwner, JobFileChange, JobPathChange, KeptEnd, KeptJob, Kernel, LinkTarget, LinkedWorktree,
    LookPath, Looked, SymbolicLink, OperatingSystem, Outbound, OutputLengths, OutputStream, PipeRef, FileOffer,
    Resume, RunnerInfo, RunnerPlatform, ResidentErrorCode, Signal, SpawnError, SpawnErrorKind,
    StreamArtifactOwner, WriteExists,
};
pub use demi_shared_types::{HostLogLine, LogLevel, LogLine, LogOrigin, ReadAt, is_event_name};
pub use pipe_frames::{
    CALL_OUTPUT_ROUTE, PIPE_DATA_BYTES, PIPE_IDLE, PIPE_SESSION_MESSAGE_BYTES, PipeEnd, PipeFrame, PipeMove, PipePath, PipeSink, moved_frame, PipeFrameDecoder,
    PipeFrameError, decode_message, encode_data, end_frame, idle_frame,
};
pub use replies::{FsOk, FsResult, GitOk, GitResult};
pub use session::{
    ACK_DELAY, Carriage, Lane, NUMBERED_HEAD_BYTES, Read, Reporter, Session, SessionError, own_frame,
};
pub use write_watch::{Watched, WriteProgress};

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// The wire's version, which a runner's hello names. A hello of another
/// version is refused, whatever release headers the socket opened with.
pub const VERSION: u32 = 64;
/// How long a stopped job has after `TERM` before a stop sends `KILL`, the
/// backend's stop and the runner's own alike (`crates/runner-shell/src/job.rs`).
pub const STOP_GRACE: Duration = Duration::from_secs(5);
/// How often a runner pings the backend (`crates/runner/src/connection/kept.rs`).
pub const RUNNER_PING_INTERVAL: Duration = Duration::from_secs(30);
/// How long either end lets a connection say nothing, no message, ping or
/// pong, before it counts the connection lost: two ping intervals, so a
/// network that stopped passing data while the socket stays open is found
/// within a minute.
pub const LINK_SILENCE: Duration = Duration::from_secs(60);
/// How long a connection's writes may wait for room with no byte accepted
/// before the writer counts the connection lost ([`WriteProgress`]): a frame
/// however large is no stall while its bytes move.
pub const LINK_WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// The most background tasks a `job_outliving` names, and the most bytes of
/// each one's command line: enough to tell a model what keeps its command
/// running without crowding its output out of the result, which a model's
/// context cannot hold without limit.
pub const JOB_OUTLIVING_TASKS: usize = 32;
pub const JOB_TASK_LINE_BYTES: usize = 256;
/// The largest frame either end sends: each frame is held whole in memory.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
/// The largest WebSocket message of the runner's link: the largest frame
/// with the head a numbered one carries ([`Session`]).
pub const LINK_FRAME_BYTES: usize = MAX_MESSAGE_BYTES + NUMBERED_HEAD_BYTES;
/// How much of the start of each stream a job always sends, and how much of
/// its newest bytes beyond them it sends while nobody follows it: the model's
/// view of a running command (`crates/runner-jobs/src/tasks.rs`), which a
/// model's context cannot hold without limit.
pub const JOB_VIEW_BYTES: usize = 8 * 1024;
/// The most bytes one message beyond a stream's first [`JOB_VIEW_BYTES`]
/// carries while the backend follows the job: 4,096 characters of up to four
/// bytes each.
pub const JOB_LIVE_BYTES: usize = 16 * 1024;
/// How often a followed job sends each stream's newest bytes beyond its
/// first [`JOB_VIEW_BYTES`], at most: with [`JOB_LIVE_BYTES`] it bounds how
/// much a followed job sends in a second.
pub const JOB_LIVE_INTERVAL: Duration = Duration::from_millis(250);
/// How often a job nobody follows sends how long a stream grew beyond its
/// first [`JOB_VIEW_BYTES`] and its newest [`JOB_VIEW_BYTES`], at most, for
/// the same reason as [`JOB_LIVE_INTERVAL`].
pub const JOB_GROWTH_INTERVAL: Duration = Duration::from_secs(2);
/// The most bytes of one binary message of a stream pipe's WebSocket
/// (`crates/runner-host/src/lib.rs`): the runner splits larger writes, so a
/// reader holds one message at a time and a ping is not stuck behind a large
/// one.
pub const STREAM_PIPE_MESSAGE_BYTES: usize = 64 * 1024;
/// The most bytes of a WebSocket close frame's reason: the protocol's control
/// frames carry 125 bytes, two of them the status code.
const CLOSE_REASON_BYTES: usize = 123;
/// The most bytes of one live stdin frame: a control message waits only for
/// the frame being written, so this bounds how long input can delay it.
pub const STDIN_CHUNK_BYTES: usize = 64 * 1024;
/// How long the runner gathers the subscriptions that changed before it
/// reports them, each once (`crates/runner-host/src/subscriptions.rs`).
pub const WATCH_GATHER: Duration = Duration::from_millis(100);
/// How long a subscription's lease runs from the `fs_subscribe` or the
/// `fs_renew` that started it, as each end counts it (`crates/runner-host/src/subscriptions.rs`).
pub const SUBSCRIPTION_LEASE: Duration = Duration::from_secs(10 * 60);
/// The most subscriptions one `fs_changed` names; more changed at once go
/// in as many messages as they take, which keeps each within
/// [`MAX_MESSAGE_BYTES`].
pub const CHANGES_PER_MESSAGE: usize = 16 * 1024;
/// The most bytes of the lines one `log_lines` carries, a line over it being
/// alone in its frame. A frame that is being written cannot be interrupted, and
/// the messages behind it wait for it, so a frame of the Host's log is small
/// (`crates/runner/src/host_log.rs`).
pub const LOG_FRAME_BYTES: usize = 32 * 1024;
/// The most of an invocation's standard error a `resident_done` carries, in
/// Unicode scalar values (`crates/shared-types/src/lib.rs`). The runner keeps the
/// last this many bytes, which are never more characters.
pub const RESIDENT_STDERR_CHARS: usize = 16 * 1024;

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

/// An encoded message.
#[derive(Clone)]
pub struct Frame(
    Vec<u8>,
    Option<std::sync::Arc<dyn std::any::Any + Send + Sync>>,
    Carriage,
);

impl Frame {
    /// The bytes to write. Whatever the frame was [held by](Self::held_by)
    /// is released here, as the writer takes it.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// Makes the frame hold `guard` until it is written, or dropped unwritten.
    /// A producer of machine-written bytes holds a permit this way, so that
    /// each of its streams has one frame in the writer's queue at most and
    /// the queue needs no length limit to keep from growing on a slow uplink.
    pub fn held_by(mut self, guard: impl Send + Sync + 'static) -> Self {
        self.1 = Some(std::sync::Arc::new(guard));
        self
    }

    /// How the frame travels the link's session.
    pub fn carriage(&self) -> Carriage {
        self.2
    }

    /// The encoded size, which [`MAX_MESSAGE_BYTES`] bounds.
    pub fn encoded_len(&self) -> usize {
        self.0.len()
    }
}

/// Validates and encodes a message. The size limit is the sender's to apply
/// ([`within_limit`]), because an oversized reply fails its own request.
pub fn encode<M: Message>(message: &M) -> Result<Frame, WireError> {
    message
        .validate()
        .map_err(|report| WireError::Invalid(report.to_string()))?;
    Ok(Frame(
        rmp_serde::to_vec_named(message)?,
        None,
        message.carriage(),
    ))
}

/// A message of the runner wire, which says how it travels the link's session.
pub trait Message: Serialize + garde::Validate<Context = ()> {
    fn carriage(&self) -> Carriage;
}

impl Message for Inbound {
    fn carriage(&self) -> Carriage {
        Inbound::carriage(self)
    }
}

impl Message for Outbound {
    fn carriage(&self) -> Carriage {
        Outbound::carriage(self)
    }
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

/// The type of the message a [`refusal`]'s `reason` names, or `message` when
/// it names none: what a log says of a refusal, which never holds the
/// message's content, as the decoding error's text may.
pub fn refused_kind(reason: &str) -> &str {
    reason
        .strip_prefix("cannot decode ")
        .and_then(|rest| rest.split(':').next())
        .filter(|kind| !kind.is_empty() && !kind.contains(' '))
        .unwrap_or("message")
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
