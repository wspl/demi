//! Command-service wire values and bounded incremental response framing.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use thiserror::Error;

mod conversation;
mod edits;
mod invocation;
mod package;

pub use conversation::{ConversationRequest, ConversationStatus};
pub use edits::{
    EDIT_FILE_BYTES, EDIT_JOB_BYTES, EDIT_JOB_FILES, EDIT_JOB_SEGMENTS, EditContext, EditCopies,
    EditFile, EditJournal, EditKind,
};
pub use invocation::{
    COMMAND_LOCALE_LANGUAGES, CommandCaller, CommandContext, CommandError, CommandLocale,
    Completion, Invocation, LocalInvocation, without_nul,
};
pub use package::{
    ArtifactLocation, ArtifactPath, ArtifactUrl, PackageArtifact, PackageDescriptor, ServiceInfo,
    TARGETS, VERSION, canonical_digest, digest, host_target, is_digest, is_target, target,
    target_artifact,
};

/// The most bytes of invocation metadata.
pub const MAX_METADATA_BYTES: usize = 256 * 1024;
/// The most bytes of one response record's payload or one input chunk.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;
pub const INFO_PATH: &str = "/v1/info";
pub const INVOKE_PATH: &str = "/v1/invoke";
pub const CONVERSATION_PATH: &str = "/v1/conversation";
pub const SHUTDOWN_PATH: &str = "/v1/shutdown";

/// The metadata that opens an invocation stream: the native protocol's
/// [`Invocation`], or the local command client's [`LocalInvocation`], which
/// shares its framing, input demand and completion.
pub trait Metadata: serde::Serialize + serde::de::DeserializeOwned + Send + 'static {
    fn validate(&self) -> Result<(), ProtocolError>;
    fn operation(&self) -> &str;

    fn encode(&self) -> Result<Bytes, ProtocolError> {
        self.validate()?;
        encode_metadata(self)
    }
}

impl Metadata for Invocation {
    fn validate(&self) -> Result<(), ProtocolError> {
        garde::Validate::validate(self).map_err(|_| ProtocolError::InvalidMetadata)
    }

    fn operation(&self) -> &str {
        &self.operation
    }
}

impl Metadata for LocalInvocation {
    fn validate(&self) -> Result<(), ProtocolError> {
        garde::Validate::validate(self).map_err(|_| ProtocolError::InvalidMetadata)
    }

    fn operation(&self) -> &str {
        &self.operation
    }
}

impl ConversationRequest {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        garde::Validate::validate(self).map_err(|_| ProtocolError::InvalidMetadata)
    }

    pub fn encode(&self) -> Result<Bytes, ProtocolError> {
        self.validate()?;
        encode_metadata(self)
    }
}

impl ConversationStatus {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        garde::Validate::validate(self).map_err(|_| ProtocolError::InvalidMetadata)
    }
}

/// Frame command-service metadata with the shared bounded length prefix.
fn encode_metadata(value: &impl serde::Serialize) -> Result<Bytes, ProtocolError> {
    let json = serde_json::to_vec(value)?;
    if json.len() > MAX_METADATA_BYTES {
        return Err(ProtocolError::TooLarge);
    }
    let mut bytes = BytesMut::with_capacity(4 + json.len());
    bytes.put_u32(json.len() as u32);
    bytes.extend_from_slice(&json);
    Ok(bytes.freeze())
}

#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    Stdout(Bytes),
    Stderr(Bytes),
    Completion(Completion),
    /// Permission for exactly one bounded stdin chunk, or stdin EOF.
    InputPull,
}

impl Record {
    pub fn encode(&self) -> Result<Bytes, ProtocolError> {
        let (kind, payload) = match self {
            Self::Stdout(bytes) => (1, bytes.clone()),
            Self::Stderr(bytes) => (2, bytes.clone()),
            Self::Completion(value) => (3, Bytes::from(serde_json::to_vec(value)?)),
            Self::InputPull => (4, Bytes::new()),
        };
        if payload.len() > MAX_RECORD_BYTES {
            return Err(ProtocolError::TooLarge);
        }
        let mut bytes = BytesMut::with_capacity(5 + payload.len());
        bytes.put_u8(kind);
        bytes.put_u32(payload.len() as u32);
        bytes.extend_from_slice(&payload);
        Ok(bytes.freeze())
    }
}

/// Input chunk boundaries survive HTTP/2 DATA fragmentation. Each InputPull
/// permits one frame; END_STREAM represents EOF and never cancellation.
pub fn encode_input(bytes: Bytes) -> Result<Bytes, ProtocolError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(ProtocolError::TooLarge);
    }
    let mut frame = BytesMut::with_capacity(4 + bytes.len());
    frame.put_u32(bytes.len() as u32);
    frame.extend_from_slice(&bytes);
    Ok(frame.freeze())
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("command protocol payload exceeds limit")]
    TooLarge,
    #[error("invalid invocation metadata")]
    InvalidMetadata,
    #[error("unknown response record kind")]
    UnknownRecord,
    #[error("response contains data after completion")]
    AfterCompletion,
    #[error("response ended without complete final status")]
    Incomplete,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Buffers at most one record. Callers retain and incrementally pass unread bytes.
#[derive(Default)]
pub struct RecordDecoder {
    pending: BytesMut,
    completed: bool,
}

impl RecordDecoder {
    pub fn decode(&mut self, input: &mut Bytes) -> Result<Option<Record>, ProtocolError> {
        if self.completed {
            return if input.is_empty() {
                Ok(None)
            } else {
                Err(ProtocolError::AfterCompletion)
            };
        }
        self.take(input, 5);
        if self.pending.len() < 5 {
            return Ok(None);
        }
        let kind = self.pending[0];
        if !(1..=4).contains(&kind) {
            return Err(ProtocolError::UnknownRecord);
        }
        let length = u32::from_be_bytes(self.pending[1..5].try_into().unwrap()) as usize;
        if length > MAX_RECORD_BYTES {
            return Err(ProtocolError::TooLarge);
        }
        self.take(input, 5 + length);
        if self.pending.len() < 5 + length {
            return Ok(None);
        }
        self.pending.advance(5);
        let payload = self.pending.split().freeze();
        let record = match kind {
            1 => Record::Stdout(payload),
            2 => Record::Stderr(payload),
            3 => {
                let completion = serde_json::from_slice(&payload)?;
                self.completed = true;
                Record::Completion(completion)
            }
            4 if payload.is_empty() => Record::InputPull,
            4 => return Err(ProtocolError::InvalidMetadata),
            _ => unreachable!(),
        };
        Ok(Some(record))
    }

    pub fn finish(self) -> Result<(), ProtocolError> {
        if !self.completed || !self.pending.is_empty() {
            return Err(ProtocolError::Incomplete);
        }
        Ok(())
    }

    fn take(&mut self, input: &mut Bytes, target: usize) {
        let count = target.saturating_sub(self.pending.len()).min(input.len());
        self.pending.extend_from_slice(&input.split_to(count));
    }
}
