//! The data the web app, the agent and the backend share.
//!
//! The crate holds types, their checks and pure lookups; no IO. The data
//! covers:
//!
//! - transcript blocks, user and tool content, tool views, and agent messages;
//! - models and their selection, and token usage;
//! - the session's phase, queue and pending steers;
//! - provider failure facts, and what the product shows of a provider entry
//!   (its model catalog, wire format, authentication and runtime states,
//!   accounts and quota snapshots);
//! - the file-type table, and model media sniffing;
//! - the JSON forms of bytes and times, and the wall clock.
//!
//! It holds no concrete provider name, catalog source name (the catalog carries
//! portable facts only, never a label such as `codex-backend`, `models.dev` or
//! `cache`), shell runtime or Host detail, user-interface concept, transport
//! URL or backend identifier.
//!
//! # One definition of each shape
//!
//! Every message and stored document that crosses a process boundary is defined
//! once, as a Rust type of a contract crate. Its JSON Schema, the web app's
//! TypeScript type and its Zod validator are derived from it by `xtask
//! contracts`. A second declaration of the same shape, in Rust or in
//! TypeScript, is a defect even while the two agree.
//!
//! # Serde convention
//!
//! Every type here follows one serde convention, which the generated validators
//! refuse the same values by:
//!
//! - Fields are camelCase. Enums are internally tagged, by `type` (`op` for
//!   transcript patches, `status` for nested outcomes, `kind` for views).
//! - An optional field is omitted when absent and refuses `null`. A nullable
//!   field is always written, as `null` when empty, and refuses absence.
//! - A time is an RFC 3339 string in UTC with exactly three fractional digits
//!   ([`Timestamp`]), so the text of two times orders as the times do.
//! - An integer the web app reads is bounded to JavaScript's safe range in the
//!   Rust type too (`range(max = MAX_SAFE_INTEGER)`), so an end that decodes it
//!   refuses what the web app cannot hold. Generation fails for a field without
//!   the bound. On a type only the backend sends nothing in Rust checks the
//!   attribute: it states the bound the web app's schema checks.
//! - A string bound counts Unicode scalar values (garde's `chars` mode) so both
//!   ends give one limit. Zod and JSON Schema count code points, while garde's
//!   default counts bytes and JavaScript's `length` UTF-16 units.
//! - A type the backend receives, from the web app or from its own storage,
//!   refuses unknown fields. A type only the web app receives accepts them
//!   (`crates/xtask/src/contracts/mod.rs`).
//!
//! # Decoding
//!
//! A value from outside the process is decoded once, at its boundary, by
//! [`decode`] or the boundary's own decode function:
//!
//! 1. serde decodes it into the type, with closed sets as enums and identities
//!    as newtypes that cannot hold an invalid value;
//! 2. garde checks bounds, lengths and patterns.
//!
//! A value is valid because a decode function checked it, never because it has
//! a type. Corrupt data is refused, never repaired or defaulted.
//!
//! Fixed contracts are checked by their types and garde, never by a round trip
//! through JSON Schema. That belongs to the values whose schema is data: a
//! command's arguments and a plugin's page call parameters.

mod account;
mod agent_message;
pub mod audio;
mod block;
mod comment;
mod bytes;
mod catalog;
mod content;
mod cost;
mod context_limit;
mod failure;
mod file_types;
mod ids;
mod kept;
mod media;
mod model;
#[cfg(feature = "mp4")]
pub mod mp4;
pub mod names;
pub mod output;
mod provider_state;
mod quota;
mod read_at;
mod schema;
mod session;
mod time;
mod view;
#[cfg(feature = "whole-image")]
pub mod whole_image;
mod workflow;

pub use account::{AgentAccount, Rotation};
pub use agent_message::{
    AgentMessage, AgentMessageEvent, CompletionId, CompletionOutcome, NotCompletionId,
    PermissionOutcome, Sender,
};
pub use block::{
    AbortBlock, AccountSwitchBlock, AgentMessageBlock, AutomationMark, Block, CodeFile, CommandEnd, CommandReport, CompactionBoundaryBlock, CompactionMarkerBlock, Entry,
    ContextBlock, ErrorBlock, ErrorDevice, EXECUTION_SOURCE, SKILLS_SOURCE, INSTRUCTIONS_SOURCE, InstructionEntry, SIDE_QUESTIONS_SOURCE, RedactedThinkingBlock, ReportEvent, ResponseBlock, ResumeBlock, ShellCommand, SteerBlock, StoppedBy,
    REPORTING_SOURCE, WORKING_DIRECTORY_SOURCE,
    TextBlock, ThinkingBlock, ToolCallBlock, ToolCallStatus, UserBlock, WakeupBlock, client_block,
    client_blocks, WakeupPlacement,
};
pub use bytes::B64Bytes;
pub use comment::{MessageComment, comments_content, markdown_quote};
pub use cost::{Cost, Currency};
pub use catalog::{ModelCost, ProviderModel, ProviderModelList, ServiceTier, UnavailableSetting};
pub use content::{
    Attachment, BlobRef, DocumentSource, GoneCause, MediaSource, PixelSize, ToolMediaSource,
    ToolResultContentBlock, UserContentBlock, attachment_tag, char_offset, is_blank, push_escaped, thousands, trim,
    xml_escaped,
};
pub use failure::{
    ContinueWith, FailureSource, NotExposedPort, OfflineDevice, ProviderErrorDiagnostics, ProviderFailureFacts, SignedOutAccount,
    VendorTrace,
};
pub use context_limit::{
    CONTEXT_LIMIT_STEPS, ContextChoice, ContextLimitStep, ContextWordError, applied_context_limit,
    context_limits, context_word, effective_context_window, is_context_limit, offered_contexts,
    offers_context, session_context_window,
};
pub use file_types::{
    IMAGE_CONTENT_POLICY, PREVIEW_TYPES, PreviewType, preview_media_type, shows_in_place,
};
#[doc(hidden)]
pub use ids::__private;
pub use ids::{
    AutomationId, AutomationRunId, BlockId, CommandId, DeviceId, EmptyId, NodeId, OperationId, RequestId,
    Sequence, TurnId,
};
pub use media::{
    MODEL_MEDIA_TYPES, ModelMediaKind, ModelMediaType, model_accepts_media_type,
    model_media_type_for, model_reads_images,
};
pub use model::{
    ATTACHMENT_FILE_EXTENSIONS, FileExtension, IMAGE_FILE_EXTENSIONS, modality_extensions, Model, ModelSelection, THINKING_OFF,
    ThinkingCapability, ThinkingConfig, ThinkingSummary, TokenUsage, VIDEO_FILE_EXTENSIONS,
    file_extension_support, model_accepts_images, model_accepts_video,
};
pub use provider_state::{
    AccountInfo, AudioKind, AudioShape, AuthState, ImageShape, LoginPending, LoginStage, MediaKind, MediaShapes, RefusedCode,
    RuntimeState, VideoResolution, VideoShape, WireApi,
};
pub use quota::{
    QuotaKind, QuotaPlan, QuotaScope, QuotaSeverity, QuotaSnapshot, QuotaUnit, QuotaWindow, QuotaWindowName,
    SnapshotSource,
};
pub use kept::{KeptMessage, choose_kept, kept_words_budget, text_tokens};
pub use read_at::ReadAt;
pub use schema::Nullable;
pub use workflow::WorkflowChoice;
pub use session::{
    ContextRow, ContextRowKind, ContextUsage, PendingCall, PendingSteer,
    QueuedMessage, SessionPhase,
};
pub use time::{Clock, SystemClock, Timestamp, TimestampError};
pub use view::{
    BinaryStdout, ChangedFile, EditCopies, EditKind, EditSegment, EditedFile, FileEdit, LineCounts,
    OutputChunk, OutputView, PathChange, ShellToolView, ShellViewStatus, StreamKind, StreamView, ToolView,
};

use serde::de::DeserializeOwned;

/// The largest integer JavaScript represents exactly, `2^53 - 1`. Every
/// integer the web app reads is bounded by it.
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
    /// The value has the shape but breaks one of the type's rules, such as
    /// a field's bound, which the message names with each field and rule.
    #[error("{0}")]
    Invalid(String),
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
        Self::Invalid(report.to_string().trim_end().to_owned())
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

/// [`decode`] for JSON bytes, such as a message off a socket.
pub fn decode_slice<T>(json: &[u8]) -> Result<T, DecodeError>
where
    T: DeserializeOwned + garde::Validate<Context = ()>,
{
    let decoded: T = serde_json::from_slice(json)?;
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
