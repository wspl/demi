//! What the CLI writes on its standard output, as Demi reads it: one type for
//! each line Demi reads, decoded by its `type` tag, and the events a line
//! becomes.
//!
//! A line, content block, stream event or delta of a type Demi does not read is
//! skipped, so a CLI release that adds one does not break a run. One Demi reads
//! that is malformed fails the run.

use demi_provider_common::wire::{
    Reported, ReportedString, Tagged,
    messages::{CallUsage, MessagesUsage},
};
use demi_provider_common::{CallEvent, CallOutcome, ErrorCode, ProviderEvent, ToolCall, tagged_wire};
use std::collections::HashMap;

use demi_shared_types::TokenUsage;
use serde::Deserialize;

use crate::failure::USAGE_LIMIT;

/// The `error` the CLI names a call with whose sign-in the vendor refused.
const AUTHENTICATION_FAILED: &str = "authentication_failed";

tagged_wire! {
    /// One line of the CLI's output.
    pub(crate) enum Line {
        "assistant" => Assistant(AssistantLine),
        "user" => User(UserLine),
        "stream_event" => StreamEvent(StreamEventLine),
        "control_request" => ControlRequest(ControlRequestLine),
        "control_response" => ControlResponse(ControlResponseLine),
        "result" => Result(ResultLine),
        "error" => Error(ErrorLine),
        "transcript_mirror" => TranscriptMirror(MirrorLine),
        "rate_limit_event" => RateLimit(RateLimitLine),
    }
}

/// The account's quota as the vendor reported it after a call, which the
/// account's snapshot reads from the line's text as every line is read, and
/// the usage ledger from the line the run takes.
#[derive(Deserialize)]
pub(crate) struct RateLimitLine {}

/// A batch of entries the CLI wrote to its session (`--session-mirror`),
/// as it wrote them.
#[derive(Deserialize)]
pub(crate) struct MirrorLine {
    #[serde(default)]
    pub(crate) entries: Vec<serde_json::Value>,
}

/// A user message the CLI printed: one written to it, printed back as the
/// CLI takes it into a turn (`--replay-user-messages`), or the results of
/// tool calls, which it prints as it sends them.
#[derive(Deserialize)]
pub(crate) struct UserLine {
    /// Set on a message written to the CLI that it printed back.
    #[serde(default, rename = "isReplay")]
    pub(crate) is_replay: bool,
}

/// A whole assistant message. With partial messages the CLI streams the
/// message first; its tool uses are read from here, since here their input
/// is whole.
#[derive(Deserialize)]
pub(crate) struct AssistantLine {
    #[serde(default)]
    message: Option<AssistantMessage>,
    /// Set on the message the CLI makes up for a call the vendor refused.
    #[serde(default)]
    error: ReportedString,
}

#[derive(Deserialize)]
struct AssistantMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    content: Option<Vec<Printed>>,
    /// The API call's counts, whole only on a message that never streamed.
    #[serde(default)]
    usage: Option<MessagesUsage>,
}

/// A content block as the CLI printed it, and as Demi reads it: `None` for
/// a type Demi does not read.
pub(crate) struct Printed {
    pub(crate) raw: serde_json::Value,
    pub(crate) block: Option<ContentBlock>,
}

impl<'de> Deserialize<'de> for Printed {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = serde_json::Value::deserialize(deserializer)?;
        let Tagged(block) =
            Tagged::<ContentBlock>::deserialize(&raw).map_err(serde::de::Error::custom)?;
        Ok(Self { raw, block })
    }
}

impl AssistantLine {
    /// Whether the line is the CLI's notice of a call the vendor refused,
    /// whose text repeats the failure the `result` line reports.
    pub(crate) fn is_refusal_notice(&self) -> bool {
        self.error.as_deref().is_some()
    }

    /// Whether the notice says the vendor refused the account's sign-in,
    /// which the CLI could not renew (`authentication_failed`, Claude Code
    /// 2.1.301).
    pub(crate) fn refused_sign_in(&self) -> bool {
        self.error.as_deref() == Some(AUTHENTICATION_FAILED)
    }

    /// The id of the message the line is part of.
    pub(crate) fn message_id(&self) -> Option<String> {
        self.message.as_ref().and_then(|message| message.id.clone())
    }

    /// The counts of the API call that wrote the message.
    pub(crate) fn usage(&self) -> Option<MessagesUsage> {
        self.message.as_ref().and_then(|message| message.usage)
    }

    pub(crate) fn content(self) -> impl Iterator<Item = Printed> {
        self.message
            .and_then(|message| message.content)
            .unwrap_or_default()
            .into_iter()
    }
}

tagged_wire! {
    /// A block of an assistant message.
    pub(crate) enum ContentBlock {
        "text" => Text(TextBlock),
        "thinking" => Thinking(ThinkingBlock),
        "redacted_thinking" => RedactedThinking(RedactedThinkingBlock),
        "tool_use" => ToolUse(ToolUseBlock),
    }
}

#[derive(Deserialize)]
pub(crate) struct TextBlock {
    #[serde(default)]
    text: String,
}

impl TextBlock {
    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

#[derive(Deserialize)]
pub(crate) struct ThinkingBlock {
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    signature: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RedactedThinkingBlock {
    #[serde(default)]
    data: String,
}

#[derive(Deserialize)]
pub(crate) struct ToolUseBlock {
    #[serde(default)]
    id: Option<ToolUseId>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
}

/// A tool use's id, which the CLI states as text or as a number.
#[derive(Deserialize)]
#[serde(untagged)]
enum ToolUseId {
    Text(String),
    Number(serde_json::Number),
}

#[derive(Deserialize)]
pub(crate) struct StreamEventLine {
    #[serde(default)]
    event: Option<Tagged<StreamEvent>>,
}

impl StreamEventLine {
    pub(crate) fn event(self) -> Option<StreamEvent> {
        self.event.and_then(|event| event.0)
    }
}

tagged_wire! {
    /// A streamed piece of a message. `message_stop` ends the message, and
    /// with it the model's batch of tool calls; `message_start` and
    /// `message_delta` report the API call's token counts.
    pub(crate) enum StreamEvent {
        "message_start" => MessageStart(MessageStart),
        "message_delta" => MessageDelta(MessageDelta),
        "content_block_start" => BlockStart(BlockStart),
        "content_block_delta" => BlockDelta(BlockDelta),
        "message_stop" => MessageStop(MessageStop),
    }
}

#[derive(Deserialize)]
pub(crate) struct BlockStart {
    #[serde(default)]
    index: Option<u64>,
    #[serde(default)]
    content_block: Option<Tagged<ContentBlock>>,
}

#[derive(Deserialize)]
pub(crate) struct BlockDelta {
    #[serde(default)]
    index: Option<u64>,
    #[serde(default)]
    delta: Option<Tagged<Delta>>,
}

#[derive(Deserialize)]
pub(crate) struct MessageStop {}

/// The start of an API call's message, with the call's input counts.
#[derive(Deserialize)]
pub(crate) struct MessageStart {
    #[serde(default)]
    message: Option<StartedMessage>,
}

#[derive(Deserialize)]
struct StartedMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    usage: Option<MessagesUsage>,
}

impl MessageStart {
    /// The id of the message, which the whole assistant lines that repeat
    /// it carry.
    pub(crate) fn message_id(&self) -> Option<String> {
        self.message.as_ref().and_then(|message| message.id.clone())
    }
}

/// The end of an API call's message, with the call's final token counts.
#[derive(Deserialize)]
pub(crate) struct MessageDelta {
    #[serde(default)]
    usage: Option<MessagesUsage>,
}

tagged_wire! {
    pub(crate) enum Delta {
        "text_delta" => Text(TextDelta),
        "thinking_delta" => Thinking(ThinkingDelta),
        "signature_delta" => Signature(SignatureDelta),
        "input_json_delta" => InputJson(InputJsonDelta),
    }
}

#[derive(Deserialize)]
pub(crate) struct InputJsonDelta {
    #[serde(default)]
    partial_json: String,
}

#[derive(Deserialize)]
pub(crate) struct TextDelta {
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
pub(crate) struct ThinkingDelta {
    #[serde(default)]
    thinking: String,
}

#[derive(Deserialize)]
pub(crate) struct SignatureDelta {
    #[serde(default)]
    signature: String,
}

/// A request the CLI makes of Demi, which it answers once, by its
/// `request_id`.
#[derive(Deserialize)]
pub(crate) struct ControlRequestLine {
    pub(crate) request_id: String,
    pub(crate) request: ControlRequestBody,
}

#[derive(Deserialize)]
pub(crate) struct ControlRequestBody {
    pub(crate) subtype: String,
    /// For `mcp_message`: the SDK MCP server the message is for.
    #[serde(default)]
    pub(crate) server_name: Option<String>,
    /// For `mcp_message`: one MCP JSON-RPC message.
    #[serde(default)]
    pub(crate) message: Option<serde_json::Value>,
}

/// The CLI's answer to a request of Demi's.
#[derive(Deserialize)]
pub(crate) struct ControlResponseLine {
    #[serde(default)]
    pub(crate) response: Option<ControlResponseBody>,
}

#[derive(Deserialize)]
pub(crate) struct ControlResponseBody {
    #[serde(default)]
    pub(crate) subtype: Option<String>,
    #[serde(default)]
    pub(crate) request_id: Option<String>,
    /// Why a request failed, as the CLI words it.
    #[serde(default)]
    pub(crate) error: ReportedString,
    /// What a request that succeeded answered, such as `get_usage`'s usage.
    #[serde(default)]
    pub(crate) response: Option<serde_json::Value>,
}

/// The line that ends the CLI's turn.
#[derive(Deserialize)]
pub(crate) struct ResultLine {
    #[serde(default)]
    is_error: Option<bool>,
    /// Error prose the CLI reports; one that is not text reads as absent, so
    /// the failure still shows as the CLI's error.
    #[serde(default)]
    result: ReportedString,
    #[serde(default)]
    errors: Reported<Vec<ReportedString>>,
    /// The vendor's HTTP status, when the failure is a call it refused.
    #[serde(default)]
    api_error_status: Option<u16>,
    /// The CLI's cause of a refused call, such as its account's usage limit
    /// ([`USAGE_LIMIT`]).
    #[serde(default)]
    api_error: ReportedString,
}

/// The token counts of the CLI's latest API call, which the response carries.
///
/// They are read from the API's stream events the CLI passes through, as the
/// Anthropic API's provider reads them ([`CallUsage`]). The `result` line's
/// `usage` is never read: it sums every call of the CLI's turn, which spans
/// every tool call the model made. Its `iterations`, which would list the
/// calls, are empty when the CLI runs with `DISABLE_TELEMETRY` (Claude Code
/// 2.1.296).
#[derive(Default)]
pub(crate) struct LastCall(CallUsage);

impl LastCall {
    pub(crate) fn observe(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::MessageStart(start) => {
                self.0.start(
                    start.message_id(),
                    start.message.as_ref().and_then(|message| message.usage),
                );
            }
            StreamEvent::MessageDelta(MessageDelta {
                usage: Some(counts),
            }) => self.0.delta(*counts),
            _ => {}
        }
    }

    /// A call whose message `id` came whole, without streaming, with the
    /// counts it stated.
    pub(crate) fn answered(&mut self, id: Option<String>, counts: Option<MessagesUsage>) {
        self.0.start(id, None);
        if let Some(counts) = counts {
            self.0.delta(counts);
        }
    }

    /// The latest call as the usage ledger keeps it, ended as `outcome`
    /// says: one row per call, never the `result` line's sum
    /// (`crates/backend-database/src/usage.rs`).
    pub(crate) fn ended(&self, outcome: CallOutcome) -> ProviderEvent {
        ProviderEvent::Call(CallEvent::Ended(self.0.call(outcome)))
    }

    /// The latest call's usage, which the run's end takes, at the turn's
    /// `result` or at a batch of tool calls, so a later run never reports it
    /// again; none when no call since ended with its counts.
    pub(crate) fn take(&mut self) -> Option<TokenUsage> {
        std::mem::take(&mut self.0).usage()
    }
}

/// A failure the CLI reports on a line of its own.
#[derive(Deserialize)]
pub(crate) struct ErrorLine {
    #[serde(default)]
    message: ReportedString,
    #[serde(default)]
    code: ReportedString,
}

/// Why a tool use cannot be a tool call.
#[derive(Debug, thiserror::Error)]
#[error("Invalid tool_use block from Claude Code")]
pub(crate) struct InvalidToolUse;

impl ContentBlock {
    /// The events of a block of a whole assistant message; a tool use is a
    /// call of the batch instead, so it gives none here.
    pub(crate) fn events(self) -> Vec<ProviderEvent> {
        match self {
            Self::Text(block) if block.text.is_empty() => Vec::new(),
            Self::Text(block) => vec![ProviderEvent::TextDelta(block.text)],
            Self::Thinking(block) => {
                let mut events = vec![ProviderEvent::ThinkingStart];
                let text = block.thinking.or(block.text).unwrap_or_default();
                if !text.is_empty() {
                    events.push(ProviderEvent::ThinkingDelta(text));
                }
                if let Some(signature) = block.signature {
                    events.push(ProviderEvent::ThinkingSignature(signature));
                }
                events
            }
            Self::RedactedThinking(block) => vec![ProviderEvent::RedactedThinking(block.data)],
            Self::ToolUse(_) => Vec::new(),
        }
    }
}

/// A block of a streamed response as its events showed it, at its
/// position among the blocks the run's events wrote.
struct Shown {
    at: usize,
    kind: ShownKind,
}

enum ShownKind {
    Thinking { text: String, signed: bool },
    Text(String),
    Redacted,
}

/// The assistant response the CLI is streaming
/// (`crates/provider-claude-code/src/mcp.rs`).
///
/// It holds the id its `message_start` names, which every whole assistant line
/// that repeats a block of it carries, and what its events showed, block by
/// block. A whole line of another message is a complete answer: the CLI's retry
/// without streaming after the stream failed, which replaces this response.
#[derive(Default)]
pub(crate) struct Streaming {
    id: Option<String>,
    shown: Vec<Shown>,
}

impl Streaming {
    pub(crate) fn new(id: Option<String>) -> Self {
        Self {
            id,
            shown: Vec::new(),
        }
    }

    /// Whether a whole assistant line of message `id` repeats a block that
    /// streamed here.
    pub(crate) fn repeats(&self, id: &Option<String>) -> bool {
        self.id == *id
    }

    /// The id of the message being streamed, when its start named one.
    pub(crate) fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// Notes an event the run yielded of this response, which went to the block
    /// at `at` ([`crate::mirror::Mirror::last_block`]).
    ///
    /// A tool use being written is no block, and a thinking start opens none
    /// until the next event.
    pub(crate) fn observe(&mut self, event: &ProviderEvent, at: Option<usize>) {
        let Some(at) = at else {
            return;
        };
        let last = self.shown.last_mut().filter(|shown| shown.at == at);
        let kind = match (event, last) {
            (ProviderEvent::ThinkingDelta(piece), Some(Shown { kind: ShownKind::Thinking { text, .. }, .. })) => {
                text.push_str(piece);
                return;
            }
            (ProviderEvent::ThinkingSignature(_), Some(Shown { kind: ShownKind::Thinking { signed, .. }, .. })) => {
                *signed = true;
                return;
            }
            (ProviderEvent::TextDelta(piece), Some(Shown { kind: ShownKind::Text(text), .. })) => {
                text.push_str(piece);
                return;
            }
            (ProviderEvent::ThinkingDelta(piece), None) => ShownKind::Thinking {
                text: piece.clone(),
                signed: false,
            },
            (ProviderEvent::ThinkingSignature(_), None) => ShownKind::Thinking {
                text: String::new(),
                signed: true,
            },
            (ProviderEvent::TextDelta(piece), None) => ShownKind::Text(piece.clone()),
            (ProviderEvent::RedactedThinking(_), _) => ShownKind::Redacted,
            _ => return,
        };
        self.shown.push(Shown { at, kind });
    }

    /// What the response showed, which a complete answer that replaces it
    /// is shown against.
    pub(crate) fn replaced(self) -> Replaced {
        Replaced {
            shown: self.shown.into_iter(),
            unrepeated: Vec::new(),
            departed: false,
        }
    }
}

/// Where a block of a complete answer went.
pub(crate) enum Answered {
    /// It repeats, or goes on with, the shown block at this position.
    Repeats(usize),
    /// It is new, and whether its events wrote a block.
    New(bool),
}

/// What an abandoned stream showed, which the complete answer that replaces it
/// is shown against.
///
/// An answer that resumes what streamed repeats the blocks that showed whole,
/// then continues the one the stream broke off in. As long as it does, only the
/// rest is shown, so the agent's blocks go on as the answer's.
///
/// From the first block that departs from what showed, the answer is shown
/// whole, after what showed, since the agent's blocks are only ever added to.
/// The shown blocks it did not repeat stay on the page, and the CLI left them
/// out of its history.
pub(crate) struct Replaced {
    shown: std::vec::IntoIter<Shown>,
    unrepeated: Vec<usize>,
    departed: bool,
}

impl Replaced {
    /// The positions of the shown blocks the answer did not repeat, once
    /// the whole answer was read.
    pub(crate) fn unrepeated(mut self) -> Vec<usize> {
        self.unrepeated.extend(self.shown.map(|shown| shown.at));
        self.unrepeated
    }

    /// The events of the answer's next block, without what of it already
    /// showed, and where the block went.
    pub(crate) fn events(&mut self, block: ContentBlock) -> (Vec<ProviderEvent>, Answered) {
        if self.departed {
            return Self::whole(block);
        }
        let Some(Shown { at, kind }) = self.shown.next() else {
            self.departed = true;
            return Self::whole(block);
        };
        // Only the block the stream broke off in goes on; an earlier one
        // showed whole.
        let last = self.shown.len() == 0;
        let continues = |shown: &str, whole: &str| {
            if last {
                whole.starts_with(shown)
            } else {
                whole == shown
            }
        };
        match (kind, block) {
            (ShownKind::Text(shown), ContentBlock::Text(block)) if continues(&shown, &block.text) => {
                let rest = &block.text[shown.len()..];
                let events = if rest.is_empty() {
                    Vec::new()
                } else {
                    vec![ProviderEvent::TextDelta(rest.to_owned())]
                };
                (events, Answered::Repeats(at))
            }
            (ShownKind::Thinking { text: shown, signed }, ContentBlock::Thinking(block))
                if continues(
                    &shown,
                    block
                        .thinking
                        .as_deref()
                        .or(block.text.as_deref())
                        .unwrap_or_default(),
                ) =>
            {
                let text = block.thinking.or(block.text).unwrap_or_default();
                let mut events = Vec::new();
                if text.len() > shown.len() {
                    events.push(ProviderEvent::ThinkingDelta(text[shown.len()..].to_owned()));
                }
                if let Some(signature) = block.signature.filter(|_| !signed) {
                    events.push(ProviderEvent::ThinkingSignature(signature));
                }
                (events, Answered::Repeats(at))
            }
            (_, block) => {
                self.unrepeated.push(at);
                self.departed = true;
                Self::whole(block)
            }
        }
    }

    fn whole(block: ContentBlock) -> (Vec<ProviderEvent>, Answered) {
        let events = block.events();
        let wrote = !events.is_empty();
        (events, Answered::New(wrote))
    }
}

impl ToolUseBlock {
    /// The call a tool use asks for, named without the `mcp__<server>__`
    /// prefix the CLI gives a tool it reaches over MCP.
    pub(crate) fn call(self) -> Result<ToolCall, InvalidToolUse> {
        let tool_use_id = match self.id {
            Some(ToolUseId::Text(id)) if !id.is_empty() => id,
            Some(ToolUseId::Number(id)) => id.to_string(),
            _ => return Err(InvalidToolUse),
        };
        let name = self
            .name
            .filter(|name| !name.is_empty())
            .ok_or(InvalidToolUse)?;
        Ok(ToolCall {
            tool_use_id,
            tool_name: tool_name(&name).to_owned(),
            input: self.input.unwrap_or_else(|| serde_json::json!({})),
        })
    }
}

impl StreamEvent {
    /// The events of a streamed piece: a thinking block's start, the text,
    /// reasoning and signatures as they come, and a tool use's start and its
    /// input pieces, for display only
    /// (`crates/provider-claude-code/src/mcp.rs`).
    ///
    /// The call itself is read from the whole message, the only place its input
    /// is whole. `writing` keeps the tool uses opened, by their block's index.
    pub(crate) fn events(self, writing: &mut HashMap<u64, String>) -> Vec<ProviderEvent> {
        match self {
            Self::BlockStart(start) => match start.content_block.and_then(|block| block.0) {
                Some(ContentBlock::Thinking(_)) => vec![ProviderEvent::ThinkingStart],
                Some(ContentBlock::Text(block)) if !block.text.is_empty() => {
                    vec![ProviderEvent::TextDelta(block.text)]
                }
                Some(ContentBlock::ToolUse(tool_use)) => {
                    let (Some(index), Ok(call)) = (start.index, tool_use.call()) else {
                        return Vec::new();
                    };
                    writing.insert(index, call.tool_use_id.clone());
                    vec![ProviderEvent::ToolCallStart {
                        tool_use_id: call.tool_use_id,
                        tool_name: call.tool_name,
                    }]
                }
                _ => Vec::new(),
            },
            Self::BlockDelta(BlockDelta {
                index,
                delta: Some(Tagged(Some(Delta::InputJson(piece)))),
            }) => match index.and_then(|index| writing.get(&index)) {
                Some(tool_use_id) if !piece.partial_json.is_empty() => {
                    vec![ProviderEvent::ToolCallInput {
                        tool_use_id: tool_use_id.clone(),
                        partial_json: piece.partial_json,
                    }]
                }
                _ => Vec::new(),
            },
            Self::BlockDelta(piece) => match piece.delta.and_then(|delta| delta.0) {
                Some(Delta::Text(delta)) if !delta.text.is_empty() => {
                    vec![ProviderEvent::TextDelta(delta.text)]
                }
                Some(Delta::Thinking(delta)) if !delta.thinking.is_empty() => {
                    vec![ProviderEvent::ThinkingDelta(delta.thinking)]
                }
                Some(Delta::Signature(delta)) => {
                    vec![ProviderEvent::ThinkingSignature(delta.signature)]
                }
                _ => Vec::new(),
            },
            Self::MessageStart(_) | Self::MessageDelta(_) | Self::MessageStop(_) => Vec::new(),
        }
    }
}

/// How a turn ended.
pub(crate) enum TurnEnd {
    /// The model answered.
    Answered,
    /// The CLI's error, as it worded it, with the vendor's HTTP status when
    /// the vendor refused the call.
    Failed {
        message: String,
        code: Option<ErrorCode>,
        status: Option<u16>,
    },
}

impl ResultLine {
    pub(crate) fn end(self) -> TurnEnd {
        if self.is_error == Some(true) {
            let mut parts: Vec<String> = Vec::new();
            let texts = self.result.into_inner().into_iter().chain(
                self.errors
                    .into_inner()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(Reported::into_inner),
            );
            for text in texts {
                let text = text.trim();
                if !text.is_empty() {
                    parts.push(text.to_owned());
                }
            }
            let message = if parts.is_empty() {
                "Claude Code returned an error".to_owned()
            } else {
                parts.join("\n")
            };
            let status = self.api_error_status;
            // The CLI's own sign of a used-up account comes first, whatever the
            // status. Otherwise the status decides, as for any HTTP failure:
            // the CLI words every call the vendor refused `API Error: <status>
            // …`, which its words alone read as a server error.
            let code = if self.api_error.as_deref() == Some(USAGE_LIMIT) {
                Some(ErrorCode::UsageLimit)
            } else {
                match status {
                    Some(status) => ErrorCode::from_http(status, &message),
                    None => ErrorCode::classify(None, &message),
                }
            };
            return TurnEnd::Failed {
                message,
                code,
                status,
            };
        }
        TurnEnd::Answered
    }
}

impl ErrorLine {
    /// The CLI's failure: its words decide the code, else the code it
    /// names.
    pub(crate) fn failure(self) -> (String, Option<ErrorCode>) {
        let message = self
            .message
            .into_inner()
            .unwrap_or_else(|| "Claude Code error".to_owned());
        let code = ErrorCode::classify(self.code.as_deref(), &message);
        (message, code)
    }
}

/// A tool's name without the `mcp__<server>__` prefix the CLI gives a tool
/// it reaches over MCP.
pub(crate) fn tool_name(name: &str) -> &str {
    let Some(rest) = name.strip_prefix("mcp__") else {
        return name;
    };
    match rest.split_once("__") {
        Some((server, tool)) if !server.is_empty() && !server.contains('_') && !tool.is_empty() => {
            tool
        }
        _ => name,
    }
}
