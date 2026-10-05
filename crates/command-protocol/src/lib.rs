//! The command wire between an execution host and a command program
//! (`crates-and-packages.md` § command-protocol): its values and its bounded
//! incremental response framing.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use thiserror::Error;

mod artifacts;
mod conversation;
mod edits;
mod invocation;
mod media;
mod numbers;
mod package;
#[cfg(feature = "testing")]
pub mod testing;

pub use artifacts::{
    ArtifactAnswer, ArtifactAsk, ArtifactForm, ArtifactInstall, ArtifactReply, ArtifactRequest,
    ArtifactsInstalled, InstalledArtifact,
};
pub use conversation::{ConversationRequest, ConversationStatus};
pub use edits::{
    EDIT_FILE_BYTES, EDIT_JOB_BYTES, EDIT_JOB_FILES, EDIT_JOB_SEGMENTS, EditContext, EditCopies,
    EditFile, EditJournal, EditKind, is_text,
};
pub use invocation::{
    COMMAND_LOCALE_LANGUAGES, CONVERSATION_NAME_CHARS, CommandCaller, CommandContext, CommandError,
    CommandLocale, Completion, Invocation, LocalInvocation, conversation_name, without_nul,
};
pub use media::{MAX_MEDIUM_BYTES, StdoutTarget, sniff_media_type};
pub use numbers::{MAX_NUMBERS, NumbersAnswer, NumbersRequest, ServiceSequence, StreamOpen};
pub use package::{
    ArtifactLocation, ArtifactPath, ArtifactUrl, PackageArtifact, PackageDescriptor,
    ServiceInfo, TARGETS, VERSION, canonical_digest, digest,
    host_target, is_digest, is_target, target, target_artifact, target_artifacts,
};

/// The most bytes of invocation metadata.
pub const MAX_METADATA_BYTES: usize = 256 * 1024;
/// The most bytes of one response record's payload or one input chunk.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;
pub const INFO_PATH: &str = "/v1/info";
pub const INVOKE_PATH: &str = "/v1/invoke";
pub const CONVERSATION_PATH: &str = "/v1/conversation";
pub const NUMBERS_PATH: &str = "/v1/numbers";
pub const ARTIFACTS_PATH: &str = "/v1/artifacts";
pub const SHUTDOWN_PATH: &str = "/v1/shutdown";

/// The metadata that opens an invocation stream: the native protocol's
/// [`Invocation`], or the local command client's [`LocalInvocation`], which
/// shares its framing, input demand and completion.
pub trait Metadata: serde::Serialize + serde::de::DeserializeOwned + Send + 'static {
    fn validate(&self) -> Result<(), ProtocolError>;
    fn operation(&self) -> &str;

    /// Whether the invocation may return media (`commands.md` § Return
    /// media): only one a job's command makes, which says where its stdout
    /// goes.
    fn returns_media(&self) -> bool;

    fn encode(&self) -> Result<Bytes, ProtocolError> {
        self.validate()?;
        encode_metadata(self)
    }
}

impl Metadata for Invocation {
    fn validate(&self) -> Result<(), ProtocolError> {
        garde::Validate::validate(self).map_err(ProtocolError::from)
    }

    fn operation(&self) -> &str {
        &self.operation
    }

    fn returns_media(&self) -> bool {
        self.stdout.is_some()
    }
}

impl Metadata for LocalInvocation {
    fn validate(&self) -> Result<(), ProtocolError> {
        garde::Validate::validate(self).map_err(ProtocolError::from)
    }

    fn operation(&self) -> &str {
        &self.operation
    }

    /// The runner routes the media of the commands it dispatches; a local
    /// caller receives only their output.
    fn returns_media(&self) -> bool {
        false
    }
}

impl ConversationRequest {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        garde::Validate::validate(self).map_err(ProtocolError::from)
    }

    pub fn encode(&self) -> Result<Bytes, ProtocolError> {
        self.validate()?;
        encode_metadata(self)
    }
}

impl StreamOpen {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        garde::Validate::validate(self).map_err(ProtocolError::from)
    }

    pub fn encode(&self) -> Result<Bytes, ProtocolError> {
        self.validate()?;
        encode_metadata(self)
    }
}

impl ConversationStatus {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        garde::Validate::validate(self).map_err(ProtocolError::from)
    }
}

/// Frame the command wire's metadata with the shared bounded length prefix.
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
    /// A returned medium of `size` bytes begins; its bytes follow in
    /// [`Record::MediumBytes`], with no other record between them
    /// (`native-runtime.md` § Response records and completion).
    Medium { size: u64 },
    /// Bytes of the medium that began last.
    MediumBytes(Bytes),
}

/// The payload of a medium record.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MediumHeader {
    size: u64,
}

impl Record {
    pub fn encode(&self) -> Result<Bytes, ProtocolError> {
        let (kind, payload) = match self {
            Self::Stdout(bytes) => (1, bytes.clone()),
            Self::Stderr(bytes) => (2, bytes.clone()),
            Self::Completion(value) => (3, Bytes::from(serde_json::to_vec(value)?)),
            Self::InputPull => (4, Bytes::new()),
            Self::Medium { size } => {
                if *size > MAX_MEDIUM_BYTES {
                    return Err(ProtocolError::TooLarge);
                }
                (5, Bytes::from(serde_json::to_vec(&MediumHeader { size: *size })?))
            }
            Self::MediumBytes(bytes) => (6, bytes.clone()),
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
    /// A value breaks its type's rules; the text names each field and rule.
    #[error("invalid command protocol value: {0}")]
    Invalid(String),
    #[error("the package has no artifact for {0}")]
    MissingTarget(String),
    #[error("unknown response record kind")]
    UnknownRecord,
    #[error("response contains data after completion")]
    AfterCompletion,
    #[error("response ended without complete final status")]
    Incomplete,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl From<garde::Report> for ProtocolError {
    fn from(report: garde::Report) -> Self {
        Self::Invalid(report.to_string().trim_end().to_owned())
    }
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
        if !(1..=6).contains(&kind) {
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
            4 => {
                return Err(ProtocolError::Invalid(
                    "an input pull record carries no payload".into(),
                ));
            }
            5 => {
                let header: MediumHeader = serde_json::from_slice(&payload)?;
                if header.size > MAX_MEDIUM_BYTES {
                    return Err(ProtocolError::TooLarge);
                }
                Record::Medium { size: header.size }
            }
            6 => Record::MediumBytes(payload),
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
