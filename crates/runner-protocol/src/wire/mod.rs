//! The runner wire: MessagePack frames over the runner's WebSocket
//! (`runner.md` § Connection and identity).

mod encoding;
mod kept;
mod messages;
mod replies;

pub use encoding::{Timestamp, WireBytes};
pub use kept::{
    JOB_KEPT_BYTES, JOB_KEPT_PART_BYTES, JOB_KEPT_READ_BYTES, KeptRecord, decode_records,
    encode_record,
};
pub use messages::{
    ArtifactOwner, ChangeKind, Descriptor, DirEntry, DirectoryFile, FileHash, FileRead, FileStat, OpenedFile, file_version, GitChange, GitChanges,
    HelloErrorCode, HostArtifact, HostIdentity, Inbound, JobArtifactOwner, JobFileChange, LogLine,
    LookPath, Looked, MAX_INSTALLED, OperatingSystem, Outbound, OutputLengths, OutputStream, PipeRef,
    RunnerInfo, RunnerPlatform, ServiceErrorCode, Signal, SpawnError, SpawnErrorKind,
    StreamArtifactOwner, VolumeName, WriteExists,
};
pub use replies::{FsOk, FsResult, GitOk, GitResult};

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// The wire's version, which a runner's hello names.
pub const VERSION: u32 = 37;
/// The largest frame either end sends.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
/// How much of the start of each stream a job always sends, and how much of
/// its newest bytes beyond them it sends while nobody follows it: the model's
/// view of a running command (`runner.md` § Pipes and output).
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
/// (`runner.md` § Host operations): the runner splits larger writes.
pub const STREAM_PIPE_MESSAGE_BYTES: usize = 64 * 1024;
/// The most bytes of the reason a stream pipe's close frame carries, the
/// most a WebSocket close frame holds.
pub const STREAM_PIPE_REASON_BYTES: usize = 123;
/// The most bytes of one live stdin frame.
pub const STDIN_CHUNK_BYTES: usize = 64 * 1024;
/// The most paths one `fs_watch_changed` carries: more changed at once
/// are reported as lost (`runner.md` § Watching files).
pub const MAX_WATCH_PATHS: usize = 1000;
/// How long a watch gathers changed paths before it reports them, each once.
pub const WATCH_GATHER: Duration = Duration::from_millis(100);
/// The most lines one `log_read` returns.
pub const LOG_READ_LINES: usize = 1000;
/// The most of an invocation's standard error a `service_done` carries, in
/// Unicode scalar values (`contracts.md` § Validation at entry). The runner keeps the
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

/// An encoded message.
pub struct Frame(Vec<u8>);

impl Frame {
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// The encoded size, which [`MAX_MESSAGE_BYTES`] bounds.
    pub fn encoded_len(&self) -> usize {
        self.0.len()
    }
}

/// Validates and encodes a message. The size limit is the sender's to apply
/// ([`within_limit`]), because an oversized reply fails its own request.
pub fn encode<M: Serialize + garde::Validate<Context = ()>>(
    message: &M,
) -> Result<Frame, WireError> {
    message
        .validate()
        .map_err(|report| WireError::Invalid(report.to_string()))?;
    Ok(Frame(rmp_serde::to_vec_named(message)?))
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

/// A reply over [`MAX_MESSAGE_BYTES`] fails its own request instead of the
/// connection (`runner.md` § Connection and identity): `refuse` builds that
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
