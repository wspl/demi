//! The relay's wire: its routes and headers, the socket's messages, and the
//! copies of the provider contract's types they carry
//! (`crates/provider-common/src/contract.rs`).
//!
//! The contract's types are the backend's own and change with it; these are
//! the wire's, and change only when [`VERSION`] does. Each converts to its
//! contract type and back. The shared types (`demi-shared-types`), which
//! storage and the web app already read as JSON, cross as they are.
//!
//! Every message is one JSON text frame.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use demi_provider_common::{
    CallEvent, CallOutcome, EntriesOf, ErrorCode, InferenceItem, InferenceRequest, MediaBytes, Medium,
    OpaqueStep, PromptCache, ProviderEvent, ProviderFailure, RequestBlock, RequestLimits, RequestPurpose, ResultPart,
    ToolCall, ToolDefinition, ToolResultKinds, UserPart, VendorCall,
};
use demi_shared_types::{B64Bytes, Entry, OpaqueStatus, OpaqueView, ProviderModelList, ThinkingConfig, TokenUsage};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::delta::Delta;

/// The wire's version, which both ends require to be their own: it goes up
/// when a message or a type of this module changes.
pub const VERSION: u32 = 1;

/// `WS`: the socket of one borrowed session.
pub const SOCKET_ROUTE: &str = "/api/relay/socket";

/// `GET`: the lent provider as [`Lent`] describes it.
pub const PROVIDER_ROUTE: &str = "/api/relay/provider";

/// The header of every relay request that names the borrower's [`VERSION`].
pub const VERSION_HEADER: &str = "demi-relay-version";

/// The header of every relay request that names the borrowing server, as the
/// host of its public URL, which the lender shows beside the key it used.
pub const SERVER_HEADER: &str = "demi-relay-server";

/// The most bytes of one message: OpenAI's limit of one request's body, the
/// largest of the vendors', since a request the vendor would refuse need not
/// cross.
pub const MESSAGE_BYTES: usize = RequestLimits::OPENAI.body_bytes.unwrap() as usize;

/// The answer to a relay request of another [`VERSION`], with HTTP 409.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OtherVersion {
    pub version: u32,
}

/// What [`PROVIDER_ROUTE`] answers: the provider's name as its lender calls
/// it, its catalog, and what one request of each model may hold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Lent {
    pub name: String,
    pub catalog: ProviderModelList,
    pub limits: Vec<ModelLimits>,
}

/// What one request of a model may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Limits {
    pub body_bytes: Option<u64>,
    pub images: Option<u32>,
    pub image_results: bool,
    pub video_results: bool,
    pub document_results: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelLimits {
    pub model: String,
    pub limits: Limits,
}

impl From<RequestLimits> for Limits {
    fn from(limits: RequestLimits) -> Self {
        Self {
            body_bytes: limits.body_bytes,
            images: limits.images,
            image_results: limits.tool_results.image,
            video_results: limits.tool_results.video,
            document_results: limits.tool_results.document,
        }
    }
}

impl From<Limits> for RequestLimits {
    fn from(limits: Limits) -> Self {
        Self {
            body_bytes: limits.body_bytes,
            images: limits.images,
            tool_results: ToolResultKinds {
                image: limits.image_results,
                video: limits.video_results,
                document: limits.document_results,
            },
        }
    }
}

/// A message of the borrower's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum ClientMessage {
    /// Runs `request`; the run's events name `run`, which grows with each.
    Run { run: u64, request: WireRequest },
    /// Cancels the run `run`, which ends with no further event but its end.
    Cancel { run: u64 },
    /// The session's action ended.
    Idle,
}

/// A message of the lender's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum ServerMessage {
    Event { run: u64, event: WireEvent },
    /// The run `run` ended: no event of it follows.
    End { run: u64 },
}

/// An [`InferenceRequest`] on the wire, its long parts as what changed since
/// the session's last request (`crate::delta`). It carries no account: the
/// lender keeps the session's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireRequest {
    pub session_id: String,
    pub affinity: Option<String>,
    pub conversation: Option<String>,
    pub turn_id: String,
    pub request_id: String,
    pub model_id: String,
    pub output_limit: Option<NonZeroU32>,
    pub output_cap: Option<NonZeroU32>,
    /// None: the last request's.
    pub system_prompt: Option<String>,
    /// None: the last request's.
    pub tools: Option<Vec<WireTool>>,
    pub items: Delta<WireItem>,
    pub blocks: Delta<WireBlock>,
    pub thinking: Option<ThinkingConfig>,
    pub service_tier_id: Option<String>,
    pub prompt_cache: WirePromptCache,
    pub purpose: WirePurpose,
    pub agent: Option<u64>,
}

impl WireRequest {
    /// The request with `whole`'s fields but for those `crate::delta` gives.
    pub(crate) fn of(
        request: &InferenceRequest,
        system_prompt: Option<String>,
        tools: Option<Vec<WireTool>>,
        items: Delta<WireItem>,
        blocks: Delta<WireBlock>,
    ) -> Self {
        Self {
            session_id: request.session_id.clone(),
            affinity: request.affinity.clone(),
            conversation: request.conversation.clone(),
            turn_id: request.turn_id.clone(),
            request_id: request.request_id.clone(),
            model_id: request.model_id.clone(),
            output_limit: request.output_limit,
            output_cap: request.output_cap,
            system_prompt,
            tools,
            items,
            blocks,
            thinking: request.thinking.clone(),
            service_tier_id: request.service_tier_id.clone(),
            prompt_cache: request.prompt_cache.into(),
            purpose: request.purpose.into(),
            agent: request.agent,
        }
    }

    /// The contract's request, with the parts the delta resolved.
    pub(crate) fn into_request(
        self,
        system_prompt: String,
        tools: Arc<[ToolDefinition]>,
        items: Arc<[InferenceItem]>,
        blocks: Arc<[RequestBlock]>,
        cancel: CancellationToken,
    ) -> InferenceRequest {
        InferenceRequest {
            session_id: self.session_id,
            affinity: self.affinity,
            conversation: self.conversation,
            turn_id: self.turn_id,
            request_id: self.request_id,
            model_id: self.model_id,
            output_limit: self.output_limit,
            output_cap: self.output_cap,
            system_prompt,
            items,
            blocks,
            tools,
            thinking: self.thinking,
            service_tier_id: self.service_tier_id,
            prompt_cache: self.prompt_cache.into(),
            cancel,
            account: None,
            purpose: self.purpose.into(),
            agent: self.agent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum WirePromptCache {
    Off,
    Session { answered_items: usize },
}

impl From<PromptCache> for WirePromptCache {
    fn from(cache: PromptCache) -> Self {
        match cache {
            PromptCache::Off => Self::Off,
            PromptCache::Session { answered_items } => Self::Session { answered_items },
        }
    }
}

impl From<WirePromptCache> for PromptCache {
    fn from(cache: WirePromptCache) -> Self {
        match cache {
            WirePromptCache::Off => Self::Off,
            WirePromptCache::Session { answered_items } => Self::Session { answered_items },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WirePurpose {
    Turn,
    Compaction,
    Title,
    SuggestedReplies,
}

impl From<RequestPurpose> for WirePurpose {
    fn from(purpose: RequestPurpose) -> Self {
        match purpose {
            RequestPurpose::Turn => Self::Turn,
            RequestPurpose::Compaction => Self::Compaction,
            RequestPurpose::Title => Self::Title,
            RequestPurpose::SuggestedReplies => Self::SuggestedReplies,
        }
    }
}

impl From<WirePurpose> for RequestPurpose {
    fn from(purpose: WirePurpose) -> Self {
        match purpose {
            WirePurpose::Turn => Self::Turn,
            WirePurpose::Compaction => Self::Compaction,
            WirePurpose::Title => Self::Title,
            WirePurpose::SuggestedReplies => Self::SuggestedReplies,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Map<String, serde_json::Value>,
}

impl From<&ToolDefinition> for WireTool {
    fn from(tool: &ToolDefinition) -> Self {
        Self {
            name: tool.name.clone(),
            description: tool.description.clone(),
            input_schema: tool.input_schema.clone(),
        }
    }
}

impl From<WireTool> for ToolDefinition {
    fn from(tool: WireTool) -> Self {
        Self {
            name: tool.name,
            description: tool.description,
            input_schema: tool.input_schema,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireBlock {
    pub start: usize,
    pub end: usize,
    pub entries: Vec<Entry>,
    pub kept_past_summary: bool,
}

impl From<&RequestBlock> for WireBlock {
    fn from(block: &RequestBlock) -> Self {
        Self {
            start: block.items.start,
            end: block.items.end,
            entries: block.entries.clone(),
            kept_past_summary: block.kept_past_summary,
        }
    }
}

impl From<WireBlock> for RequestBlock {
    fn from(block: WireBlock) -> Self {
        Self {
            items: block.start..block.end,
            entries: block.entries,
            kept_past_summary: block.kept_past_summary,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum WireItem {
    UserMessage { content: Vec<WirePart> },
    UserSteer { content: Vec<WirePart> },
    AssistantText { model_id: String, text: String },
    AssistantThinking { model_id: String, text: String, signature: Option<String>, kept_past_summary: bool },
    Opaque { model_id: String, entries: Vec<Entry>, kept_past_summary: bool },
    ToolUse { model_id: String, tool_use_id: String, tool_name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, output: Vec<WireResult>, is_error: bool },
}

impl From<&InferenceItem> for WireItem {
    fn from(item: &InferenceItem) -> Self {
        match item.clone() {
            InferenceItem::UserMessage { content } => Self::UserMessage {
                content: content.into_iter().map(Into::into).collect(),
            },
            InferenceItem::UserSteer { content } => Self::UserSteer {
                content: content.into_iter().map(Into::into).collect(),
            },
            InferenceItem::AssistantText { model_id, text } => Self::AssistantText { model_id, text },
            InferenceItem::AssistantThinking { model_id, text, signature, kept_past_summary } => {
                Self::AssistantThinking { model_id, text, signature, kept_past_summary }
            }
            InferenceItem::Opaque { model_id, entries, kept_past_summary } => {
                Self::Opaque { model_id, entries, kept_past_summary }
            }
            InferenceItem::ToolUse { model_id, tool_use_id, tool_name, input } => {
                Self::ToolUse { model_id, tool_use_id, tool_name, input }
            }
            InferenceItem::ToolResult { tool_use_id, output, is_error } => Self::ToolResult {
                tool_use_id,
                output: output.into_iter().map(Into::into).collect(),
                is_error,
            },
        }
    }
}

impl From<WireItem> for InferenceItem {
    fn from(item: WireItem) -> Self {
        match item {
            WireItem::UserMessage { content } => Self::UserMessage {
                content: content.into_iter().map(Into::into).collect(),
            },
            WireItem::UserSteer { content } => Self::UserSteer {
                content: content.into_iter().map(Into::into).collect(),
            },
            WireItem::AssistantText { model_id, text } => Self::AssistantText { model_id, text },
            WireItem::AssistantThinking { model_id, text, signature, kept_past_summary } => {
                Self::AssistantThinking { model_id, text, signature, kept_past_summary }
            }
            WireItem::Opaque { model_id, entries, kept_past_summary } => {
                Self::Opaque { model_id, entries, kept_past_summary }
            }
            WireItem::ToolUse { model_id, tool_use_id, tool_name, input } => {
                Self::ToolUse { model_id, tool_use_id, tool_name, input }
            }
            WireItem::ToolResult { tool_use_id, output, is_error } => Self::ToolResult {
                tool_use_id,
                output: output.into_iter().map(Into::into).collect(),
                is_error,
            },
        }
    }
}

/// A medium's bytes and type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireBytes {
    pub data: B64Bytes,
    pub media_type: String,
}

impl From<MediaBytes> for WireBytes {
    fn from(bytes: MediaBytes) -> Self {
        Self {
            data: bytes.data,
            media_type: bytes.media_type,
        }
    }
}

impl From<WireBytes> for MediaBytes {
    fn from(bytes: WireBytes) -> Self {
        Self {
            data: bytes.data,
            media_type: bytes.media_type,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum WireMedium {
    Bytes { bytes: WireBytes },
    Url { url: String },
}

impl From<Medium> for WireMedium {
    fn from(medium: Medium) -> Self {
        match medium {
            Medium::Bytes(bytes) => Self::Bytes { bytes: bytes.into() },
            Medium::Url(url) => Self::Url { url },
        }
    }
}

impl From<WireMedium> for Medium {
    fn from(medium: WireMedium) -> Self {
        match medium {
            WireMedium::Bytes { bytes } => Self::Bytes(bytes.into()),
            WireMedium::Url { url } => Self::Url(url),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum WirePart {
    Text { text: String },
    Image { medium: WireMedium },
    Video { medium: WireMedium },
    Document { bytes: WireBytes, file_name: String },
}

impl From<UserPart> for WirePart {
    fn from(part: UserPart) -> Self {
        match part {
            UserPart::Text(text) => Self::Text { text },
            UserPart::Image(medium) => Self::Image { medium: medium.into() },
            UserPart::Video(medium) => Self::Video { medium: medium.into() },
            UserPart::Document { bytes, file_name } => Self::Document {
                bytes: bytes.into(),
                file_name,
            },
        }
    }
}

impl From<WirePart> for UserPart {
    fn from(part: WirePart) -> Self {
        match part {
            WirePart::Text { text } => Self::Text(text),
            WirePart::Image { medium } => Self::Image(medium.into()),
            WirePart::Video { medium } => Self::Video(medium.into()),
            WirePart::Document { bytes, file_name } => Self::Document {
                bytes: bytes.into(),
                file_name,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum WireResult {
    Text { text: String },
    Image { bytes: WireBytes },
    Video { bytes: WireBytes },
    Document { bytes: WireBytes, file_name: String },
}

impl From<ResultPart> for WireResult {
    fn from(part: ResultPart) -> Self {
        match part {
            ResultPart::Text(text) => Self::Text { text },
            ResultPart::Image(bytes) => Self::Image { bytes: bytes.into() },
            ResultPart::Video(bytes) => Self::Video { bytes: bytes.into() },
            ResultPart::Document { bytes, file_name } => Self::Document {
                bytes: bytes.into(),
                file_name,
            },
        }
    }
}

impl From<WireResult> for ResultPart {
    fn from(part: WireResult) -> Self {
        match part {
            WireResult::Text { text } => Self::Text(text),
            WireResult::Image { bytes } => Self::Image(bytes.into()),
            WireResult::Video { bytes } => Self::Video(bytes.into()),
            WireResult::Document { bytes, file_name } => Self::Document {
                bytes: bytes.into(),
                file_name,
            },
        }
    }
}

/// A [`ProviderEvent`] that crosses: none of the account's, its quota's or a
/// failure's but its code (`crate::lend`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum WireEvent {
    ThinkingStart,
    ThinkingDelta { text: String },
    ThinkingSignature { signature: String },
    Opaque { call_id: Option<String>, status: OpaqueStatus, view: OpaqueView, entries: Vec<Entry> },
    TextDelta { text: String },
    ToolCallStart { tool_use_id: String, tool_name: String },
    ToolCallInput { tool_use_id: String, partial_json: String },
    ToolCall { tool_use_id: String, tool_name: String, input: serde_json::Value },
    Response { usage: Option<TokenUsage> },
    CacheLifetime { seconds: u64 },
    /// The run failed; `code` is the failure's ([`ErrorCode::as_str`]).
    Error { code: Option<String> },
    Entries { of: WireEntriesOf, entries: Vec<Entry> },
    CallStarted,
    CallEnded {
        vendor_id: Option<String>,
        usage: Option<TokenUsage>,
        cache_write_1h_tokens: u64,
        /// None: answered; else the refusal's code, if it has one.
        refused: Option<Option<String>>,
        /// The model that answered instead of the one asked.
        rerouted_to: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum WireEntriesOf {
    Item { index: usize },
    Output { index: usize },
}

impl WireEvent {
    /// The event the borrower may see of `event`; none for what stays with
    /// the lender: the account's, its quota's.
    pub fn lent(event: ProviderEvent) -> Option<Self> {
        Some(match event {
            ProviderEvent::ThinkingStart => Self::ThinkingStart,
            ProviderEvent::ThinkingDelta(text) => Self::ThinkingDelta { text },
            ProviderEvent::ThinkingSignature(signature) => Self::ThinkingSignature { signature },
            ProviderEvent::Opaque(step) => Self::Opaque {
                call_id: step.call_id,
                status: step.status,
                view: step.view,
                entries: step.entries,
            },
            ProviderEvent::TextDelta(text) => Self::TextDelta { text },
            ProviderEvent::ToolCallStart { tool_use_id, tool_name } => Self::ToolCallStart { tool_use_id, tool_name },
            ProviderEvent::ToolCallInput { tool_use_id, partial_json } => {
                Self::ToolCallInput { tool_use_id, partial_json }
            }
            ProviderEvent::ToolCall(call) => Self::ToolCall {
                tool_use_id: call.tool_use_id,
                tool_name: call.tool_name,
                input: call.input,
            },
            ProviderEvent::Response(usage) => Self::Response { usage },
            ProviderEvent::CacheLifetime(lifetime) => Self::CacheLifetime {
                seconds: lifetime.as_secs(),
            },
            ProviderEvent::Error(failure) => Self::Error {
                code: failure.code.map(|code| code.as_str().to_owned()),
            },
            ProviderEvent::Entries { of, entries } => Self::Entries {
                of: match of {
                    EntriesOf::Item(index) => WireEntriesOf::Item { index },
                    EntriesOf::Output(index) => WireEntriesOf::Output { index },
                },
                entries,
            },
            ProviderEvent::Call(CallEvent::Started) => Self::CallStarted,
            ProviderEvent::Call(CallEvent::Ended(call)) => Self::CallEnded {
                vendor_id: call.vendor_id,
                usage: call.usage,
                cache_write_1h_tokens: call.cache_write_1h_tokens,
                refused: match call.outcome {
                    CallOutcome::Answered => None,
                    CallOutcome::Refused(code) => Some(code.map(|code| code.as_str().to_owned())),
                },
                rerouted_to: call.rerouted_to,
            },
            ProviderEvent::Call(CallEvent::Observed(_))
            | ProviderEvent::Account(_)
            | ProviderEvent::AccountSwitch(_) => return None,
        })
    }

    /// The event the borrower's session takes; a failure says `failed`, the
    /// message the borrower composes for its code.
    pub fn borrowed(self, failed: impl FnOnce(Option<&ErrorCode>) -> String) -> ProviderEvent {
        match self {
            Self::ThinkingStart => ProviderEvent::ThinkingStart,
            Self::ThinkingDelta { text } => ProviderEvent::ThinkingDelta(text),
            Self::ThinkingSignature { signature } => ProviderEvent::ThinkingSignature(signature),
            Self::Opaque { call_id, status, view, entries } => ProviderEvent::Opaque(OpaqueStep {
                call_id,
                status,
                view,
                entries,
            }),
            Self::TextDelta { text } => ProviderEvent::TextDelta(text),
            Self::ToolCallStart { tool_use_id, tool_name } => ProviderEvent::ToolCallStart { tool_use_id, tool_name },
            Self::ToolCallInput { tool_use_id, partial_json } => {
                ProviderEvent::ToolCallInput { tool_use_id, partial_json }
            }
            Self::ToolCall { tool_use_id, tool_name, input } => ProviderEvent::ToolCall(ToolCall {
                tool_use_id,
                tool_name,
                input,
            }),
            Self::Response { usage } => ProviderEvent::Response(usage),
            Self::CacheLifetime { seconds } => ProviderEvent::CacheLifetime(Duration::from_secs(seconds)),
            Self::Error { code } => {
                let code = code.map(|code| code.parse::<ErrorCode>().expect("a code parses"));
                ProviderEvent::Error(ProviderFailure {
                    message: failed(code.as_ref()),
                    code,
                    diagnostics: None,
                    retry_after: None,
                })
            }
            Self::Entries { of, entries } => ProviderEvent::Entries {
                of: match of {
                    WireEntriesOf::Item { index } => EntriesOf::Item(index),
                    WireEntriesOf::Output { index } => EntriesOf::Output(index),
                },
                entries,
            },
            Self::CallStarted => ProviderEvent::Call(CallEvent::Started),
            Self::CallEnded { vendor_id, usage, cache_write_1h_tokens, refused, rerouted_to } => {
                ProviderEvent::Call(CallEvent::Ended(VendorCall {
                    vendor_id,
                    usage,
                    cache_write_1h_tokens,
                    outcome: match refused {
                        None => CallOutcome::Answered,
                        Some(code) => {
                            CallOutcome::Refused(code.map(|code| code.parse().expect("a code parses")))
                        }
                    },
                    rerouted_to,
                }))
            }
        }
    }
}
