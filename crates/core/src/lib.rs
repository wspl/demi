//! The data the browser, the agent and the backend share
//! (`crates-and-packages.md` § core): transcript blocks, user and tool
//! content, models and their selection, token usage, tool views, agent
//! messages, the session's phase, queue and pending steers, provider failure
//! facts, what the product shows of a provider entry (its model catalog, wire
//! format, authentication and runtime states, accounts and quota snapshots), the
//! file-type table, model media sniffing, the JSON forms of bytes and times,
//! and the wall clock. It holds types, their checks and pure lookups; no IO.
//!
//! Every type follows the encoding conventions of `contracts.md`: camelCase
//! fields, enums tagged by `type` (`kind` for views), optional fields that
//! refuse `null`, nullable fields that refuse absence. A type the backend
//! receives, from the browser or from its own storage, refuses unknown
//! fields; a type only the browser receives accepts them.

mod agent_message;
mod block;
mod bytes;
mod catalog;
mod content;
mod failure;
mod file_types;
mod ids;
mod media;
mod model;
mod provider_state;
mod quota;
mod schema;
mod session;
mod time;
mod view;

pub use agent_message::{
    AgentMessage, AgentMessageEvent, CompletionId, CompletionOutcome, NotCompletionId, Sender,
};
pub use block::{
    AbortBlock, AgentMessageBlock, Block, CompactionBoundaryBlock, CompactionMarkerBlock,
    ContextBlock, ErrorBlock, RedactedThinkingBlock, ResponseBlock, ResumeBlock, SteerBlock,
    TextBlock, ThinkingBlock, ToolCallBlock, ToolCallStatus, UserBlock, WakeupBlock,
    WakeupPlacement,
};
pub use bytes::B64Bytes;
pub use catalog::{ModelCost, ProviderModel, ProviderModelList, ServiceTier};
pub use content::{
    Attachment, BlobRef, DocumentSource, MediaSource, ToolMediaSource, ToolResultContentBlock,
    UserContentBlock, attachment_tag, is_blank, trim,
};
pub use failure::{FailureSource, ProviderErrorDiagnostics, ProviderFailureFacts};
pub use file_types::{PREVIEW_TYPES, PreviewType, preview_media_type, shows_in_place};
#[doc(hidden)]
pub use ids::__private;
pub use ids::{BlockId, CommandId, EmptyId, NodeId, OperationId, ShellId, TurnId, WakeupId};
pub use media::{
    MODEL_MEDIA_TYPES, ModelMediaKind, ModelMediaType, model_accepts_media_type,
    model_media_type_for, sniff_model_media_type,
};
pub use model::{
    ATTACHMENT_FILE_EXTENSIONS, FileExtension, Model, ModelSelection, ThinkingCapability,
    ThinkingConfig, ThinkingSummary, TokenUsage, VIDEO_FILE_EXTENSIONS, file_extension_support,
    model_accepts_video,
};
pub use provider_state::{AccountInfo, AuthState, LoginPending, RuntimeState, WireApi};
pub use quota::{
    QuotaPlan, QuotaScope, QuotaSeverity, QuotaSnapshot, QuotaUnit, QuotaWindow, SnapshotSource,
};
pub use schema::Nullable;
pub use session::{PendingSteer, QueuedMessage, SessionPhase};
pub use time::{Clock, SystemClock, Timestamp, TimestampError};
pub use view::{
    BinaryStdout, EditKind, EditedFile, KeptEdit, OutputChunk, OutputView, ShellToolView,
    ShellViewStatus, StreamKind, StreamView, ToolView,
};

use serde::de::DeserializeOwned;

/// The largest integer JavaScript represents exactly, `2^53 - 1`. Every
/// integer the browser reads is bounded by it.
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// Why a value that entered the process was refused.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The text is not JSON.
    #[error("not JSON: {0}")]
    Syntax(serde_json::Error),
    /// The JSON does not have the type's shape.
    #[error(transparent)]
    Shape(serde_json::Error),
    /// The value has the shape but breaks one of the type's rules; the
    /// report names each field and rule.
    #[error("{}", .0.to_string().trim_end())]
    Invalid(garde::Report),
}

impl From<serde_json::Error> for DecodeError {
    fn from(error: serde_json::Error) -> Self {
        match error.classify() {
            serde_json::error::Category::Data => Self::Shape(error),
            serde_json::error::Category::Syntax
            | serde_json::error::Category::Eof
            | serde_json::error::Category::Io => Self::Syntax(error),
        }
    }
}

impl From<garde::Report> for DecodeError {
    fn from(report: garde::Report) -> Self {
        Self::Invalid(report)
    }
}

/// Decodes JSON text that entered the process, such as a stored transcript
/// row, into `T` and checks its rules. This is the decode function of every
/// boundary that receives one of this crate's types.
pub fn decode<T>(json: &str) -> Result<T, DecodeError>
where
    T: DeserializeOwned + garde::Validate<Context = ()>,
{
    let decoded: T = serde_json::from_str(json)?;
    decoded.validate()?;
    Ok(decoded)
}

/// [`decode`] for a JSON value that is already parsed, such as one field of a
/// larger document.
pub fn decode_value<T>(value: serde_json::Value) -> Result<T, DecodeError>
where
    T: DeserializeOwned + garde::Validate<Context = ()>,
{
    let decoded: T = serde_json::from_value(value)?;
    decoded.validate()?;
    Ok(decoded)
}
