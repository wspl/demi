//! The runner wire: MessagePack frames over the runner's WebSocket
//! (`runner.md` § Connection and identity).

mod encoding;
mod messages;
mod replies;

pub use encoding::{Timestamp, WireBytes};
pub use messages::{
    ArtifactOwner, ChangeKind, DirEntry, FileStat, GitChange, GitChanges, HelloErrorCode,
    HostIdentity, Inbound, JobArtifactOwner, JobFileChange, LogLine, NetErrorCode, Outbound,
    OutputStream, PipeRef, Readdir, RetainedOutput, RunnerInfo, ServiceErrorCode, SpawnError,
    SpawnErrorKind, StreamArtifactOwner, VolumeName,
};
pub use replies::{FsOk, FsResult, GitOk, GitResult};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// The wire's version, which a runner's hello names.
pub const VERSION: u32 = 18;
/// The largest frame either end sends.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
/// The most output of each stream a job's live view carries.
pub const JOB_VIEW_BYTES: usize = 32 * 1024;
/// The most bytes of one live stdin frame.
pub const STDIN_CHUNK_BYTES: usize = 64 * 1024;
/// The most lines one `log_read` returns.
pub const LOG_READ_LINES: usize = 1000;
/// The most of an invocation's standard error a `service_done` carries.
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
pub fn encode<M: Serialize + garde::Validate<Context = ()>>(message: &M) -> Result<Frame, WireError> {
    message
        .validate()
        .map_err(|report| WireError::Invalid(report.to_string()))?;
    Ok(Frame(rmp_serde::to_vec_named(message)?))
}

/// Decodes and validates one message; bytes after it are refused.
pub fn decode<M: DeserializeOwned + garde::Validate<Context = ()>>(bytes: &[u8]) -> Result<M, WireError> {
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
