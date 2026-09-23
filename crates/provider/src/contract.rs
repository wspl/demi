//! The provider contract (`providers.md` § Provider contract): a shared
//! [`Provider`] for each entry and account, and a [`ProviderRuntime`] that runs
//! one session's requests on the user's shard.

use std::{num::NonZeroU32, sync::Arc};

use demi_core::{
    AuthState, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList, RuntimeState,
    ThinkingConfig, Timestamp, TokenUsage, ToolResultContentBlock, UserContentBlock,
};
use futures_util::{
    future::{BoxFuture, LocalBoxFuture},
    stream::LocalBoxStream,
};
use tokio_util::sync::CancellationToken;

use crate::{ProviderFailure, credentials::SubscriptionAccounts, quota::ProviderQuota};

/// One provider entry and account, shared by every user and request of the
/// entry. It is used from any thread: the backend's request handlers read its
/// status, models and quota, and each user's shard builds runtimes from it.
pub trait Provider: Send + Sync + 'static {
    /// The entry's id, as the backend configured it.
    fn id(&self) -> &str;

    /// The entry's label.
    fn display_name(&self) -> &str;

    fn capabilities(&self) -> Capabilities;

    /// Whether the credential is present and usable. Never makes an
    /// inference request.
    fn auth_status(&self) -> BoxFuture<'_, AuthState>;

    /// Whether the provider can run requests.
    fn runtime_state(&self) -> RuntimeState;

    /// A fresh read of the provider's model directory. The provider keeps no
    /// catalog of its own: the backend's catalog cache is the only one
    /// (`models.md` § Catalog cache).
    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>>;

    /// Reads a failure record this provider produced (`failures-and-recovery.md`
    /// § Reading a failure). `received_at` is when the failure was received; a
    /// relative wait counts from it.
    fn read_failure(
        &self,
        diagnostics: &ProviderErrorDiagnostics,
        received_at: Timestamp,
    ) -> ProviderFailureFacts;

    /// The vendor quota of the provider's account, for a family that reports
    /// one.
    fn quota(&self) -> Option<&ProviderQuota> {
        None
    }

    /// The account operations of a subscription family.
    fn accounts(&self) -> Option<&dyn SubscriptionAccounts> {
        None
    }

    /// Builds a runtime for one session. The backend calls it on the shard
    /// that will own the runtime.
    fn runtime(&self, env: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError>;
}

/// What a provider requires of the Host its conversation runs on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// The runtime starts a process on a Host, so the backend places the
    /// process and admission refuses a Host without process execution.
    pub process_host: bool,
}

/// What the user's shard gives a runtime it builds.
#[derive(Debug, Clone)]
pub struct RuntimeEnv {
    /// The shard's HTTP client. A runtime never builds its own: hyper ties a
    /// pooled connection to the Tokio runtime that created it
    /// (`concurrency.md` § Programs and threads).
    pub http: reqwest::Client,
}

/// Why a provider could not build a runtime.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeError {
    /// The provider starts a process and the environment offers no Host to
    /// start it on.
    #[error("{provider} needs a Host that runs processes")]
    ProcessHostRequired { provider: String },
}

/// Why a provider's model directory could not be read. The backend shows the
/// message as the entry's warning and keeps its last catalog.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    /// The account has no usable credential.
    #[error("{0}")]
    Unauthenticated(String),
    /// The directory did not answer, or refused.
    #[error("{0}")]
    Unavailable(String),
    /// The directory answered with a payload that cannot be read.
    #[error("{0}")]
    Invalid(String),
}

/// A session's inference runtime. It runs one request at a time: `run`
/// borrows the runtime until its stream is dropped. It lives on the user's
/// shard and has no `Send` bound.
pub trait ProviderRuntime {
    /// Runs one inference attempt; see [`ProviderEvent`] for how a run ends.
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_>;

    /// An independent runtime with the same configuration. It shares what the
    /// provider shares and none of this runtime's execution state.
    fn fresh(&self) -> Box<dyn ProviderRuntime>;

    /// Releases what outlives a run, such as a kept process. A runtime
    /// dropped without closing still has its process killed, without
    /// waiting.
    fn close(&mut self) -> LocalBoxFuture<'_, ()>;
}

/// The events of one run.
pub type ProviderRun<'a> = LocalBoxStream<'a, ProviderEvent>;

/// One inference request of a session.
#[derive(Debug, Clone)]
pub struct InferenceRequest {
    /// The owning session.
    pub session_id: String,
    /// The turn the request belongs to; continuations inside a turn share it.
    pub turn_id: String,
    /// This request, unique per attempt.
    pub request_id: String,
    pub model_id: String,
    /// The current model's output limit; null when the model names none
    /// (`models.md` § Output limit).
    pub output_limit: Option<NonZeroU32>,
    pub system_prompt: String,
    /// The transcript as the model receives it (`runtime.md` § Replay).
    pub items: Arc<[InferenceItem]>,
    pub tools: Arc<[ToolDefinition]>,
    pub thinking: Option<ThinkingConfig>,
    pub service_tier_id: Option<String>,
    /// Cancels the run: it stops its work at once and ends without a further
    /// event.
    pub cancel: CancellationToken,
}

/// One entry of the transcript as a provider replays it.
#[derive(Debug, Clone, PartialEq)]
pub enum InferenceItem {
    UserMessage {
        content: Vec<UserContentBlock>,
    },
    /// A steer or an agent message that joined the running turn.
    UserSteer {
        content: Vec<UserContentBlock>,
    },
    AssistantText {
        model_id: String,
        text: String,
    },
    /// Reasoning with the vendor's signature when it signed it; a provider
    /// replays only signatures it recognizes as its vendor's.
    AssistantThinking {
        model_id: String,
        text: String,
        signature: Option<String>,
    },
    /// Opaque reasoning data, replayed as received.
    AssistantRedactedThinking {
        model_id: String,
        data: String,
    },
    ToolUse {
        model_id: String,
        tool_use_id: String,
        tool_name: String,
        /// The JSON value the provider supplied, or a string when it was not
        /// valid JSON.
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        output: Vec<ToolResultContentBlock>,
        is_error: bool,
    },
}

/// A tool the model may call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// The JSON Schema of the tool's input.
    pub input_schema: serde_json::Map<String, serde_json::Value>,
}

/// An event of a run (`providers.md` § A run). A run ends after `Response`,
/// after `Error`, after the last `ToolCall` of a batch when the vendor needs
/// the results before it can finish, or when its request's token is
/// cancelled. Cancellation ends it without a further event, and dropping the
/// stream is cancellation. Every failure is its `Error`; nothing fails outside
/// the stream.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderEvent {
    /// The model opened a reasoning block.
    ThinkingStart,
    ThinkingDelta(String),
    /// The vendor's signature over the reasoning, sent back with it later.
    ThinkingSignature(String),
    /// Reasoning the vendor sends only in encrypted form, sent back as
    /// received.
    RedactedThinking(String),
    TextDelta(String),
    ToolCall(ToolCall),
    /// The usage of the run's final API call: the context the next request
    /// carries, never a total over the turn.
    Response(TokenUsage),
    /// The run's failure; it is the run's last event.
    Error(ProviderFailure),
}

/// A tool call the model requested.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub tool_use_id: String,
    pub tool_name: String,
    /// The JSON value the vendor supplied, or a string when it was not valid
    /// JSON.
    pub input: serde_json::Value,
}
