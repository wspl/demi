//! Command-service wire values and bounded incremental response framing.

use std::collections::BTreeMap;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod package;
mod package_generated;
pub use package::{canonical_digest, target_artifact};
pub use package_generated::{
    PackageDescriptor, PackageDescriptorTargetsValue as PackageArtifact, TARGETS,
};

pub const VERSION: u32 = 1;
pub const MAX_METADATA_BYTES: usize = 256 * 1024;
pub const MAX_RECORD_BYTES: usize = 64 * 1024;
pub const INFO_PATH: &str = "/v1/info";
pub const INVOKE_PATH: &str = "/v1/invoke";
pub const SHUTDOWN_PATH: &str = "/v1/shutdown";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ServiceInfo {
    pub protocol_version: u32,
    pub operations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Invocation {
    pub operation: String,
    pub invocation_id: String,
    pub args: serde_json::Value,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
}

impl Invocation {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.operation.is_empty() || self.invocation_id.is_empty() || self.cwd.is_empty() {
            return Err(ProtocolError::InvalidMetadata);
        }
        if !self.args.is_object() {
            return Err(ProtocolError::InvalidMetadata);
        }
        if self.cwd.contains('\0')
            || self.env.iter().any(|(key, value)| {
                key.is_empty() || key.contains(['\0', '=']) || value.contains('\0')
            })
        {
            return Err(ProtocolError::InvalidMetadata);
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Bytes, ProtocolError> {
        self.validate()?;
        let json = serde_json::to_vec(self)?;
        if json.len() > MAX_METADATA_BYTES {
            return Err(ProtocolError::TooLarge);
        }
        let mut bytes = BytesMut::with_capacity(4 + json.len());
        bytes.put_u32(json.len() as u32);
        bytes.extend_from_slice(&json);
        Ok(bytes.freeze())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Completion {
    pub exit_code: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CommandError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Stdout(Bytes),
    Stderr(Bytes),
    Completion(Completion),
}

impl Record {
    pub fn encode(&self) -> Result<Bytes, ProtocolError> {
        let (kind, payload) = match self {
            Self::Stdout(bytes) => (1, bytes.clone()),
            Self::Stderr(bytes) => (2, bytes.clone()),
            Self::Completion(value) => (3, Bytes::from(serde_json::to_vec(value)?)),
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
        if !(1..=3).contains(&kind) {
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
