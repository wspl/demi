//! Command-service wire values and bounded incremental response framing.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use thiserror::Error;

mod package;
// Generated checks use uniform borrowed expressions across owned fields and references.
#[allow(
    clippy::needless_borrow,
    clippy::deref_addrof,
    clippy::len_zero,
    clippy::nonminimal_bool,
    clippy::collapsible_if,
    clippy::redundant_closure_call
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/protocol.rs"));
}
pub use generated::{
    ArtifactLocation, CONVERSATION_PATH, CommandError, Completion, ConversationRequest,
    ConversationStatus, EDIT_FILE_BYTES, EDIT_JOB_BYTES, EDIT_JOB_FILES, EDIT_JOB_SEGMENTS,
    EditContext, EditCopies, EditFile, EditJournal, INFO_PATH, INVOKE_PATH, Invocation,
    MAX_METADATA_BYTES, MAX_RECORD_BYTES, PackageDescriptor,
    PackageDescriptorTargetsValue as PackageArtifact, SHUTDOWN_PATH, ServiceInfo, TARGETS, VERSION,
};
pub use package::{canonical_digest, host_target, target_artifact};

impl EditContext {
    pub fn validate(&self) -> Result<(), String> {
        generated::edit_context_validate(self)?;
        if !std::path::Path::new(&self.directory).is_absolute()
            || !std::path::Path::new(&self.lock).is_absolute()
        {
            return Err("edit context paths must be absolute".into());
        }
        Ok(())
    }
}

impl EditJournal {
    pub fn validate(&self) -> Result<(), String> {
        generated::edit_journal_validate(self)
    }
}

impl Invocation {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        generated::invocation_validate(self).map_err(|_| ProtocolError::InvalidMetadata)
    }

    pub fn encode(&self) -> Result<Bytes, ProtocolError> {
        self.validate()?;
        encode_metadata(self)
    }
}

impl ConversationRequest {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        generated::conversation_request_validate(self)
            .map_err(|_| ProtocolError::InvalidMetadata)?;
        if self.operation == "release" && self.conversation.is_none() {
            return Err(ProtocolError::InvalidMetadata);
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Bytes, ProtocolError> {
        self.validate()?;
        encode_metadata(self)
    }
}

impl ConversationStatus {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        generated::conversation_status_validate(self).map_err(|_| ProtocolError::InvalidMetadata)
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
