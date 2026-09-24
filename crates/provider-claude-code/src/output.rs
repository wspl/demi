//! What the CLI writes on its standard output (`claude-code.md` § Reading),
//! as Demi reads it: one type for each line Demi reads, decoded by its `type`
//! tag, and the events a line becomes. A line, content block, stream event
//! or delta of a type Demi does not read is skipped, so a CLI release that
//! adds one does not break a run; one Demi reads that is malformed fails it.

use demi_core::TokenUsage;
use demi_provider::wire::{Reported, ReportedString, Tagged};
use demi_provider::{ErrorCode, ProviderEvent, ToolCall, tagged_wire};
use serde::Deserialize;

tagged_wire! {
    /// One line of the CLI's output.
    pub(crate) enum Line {
        "assistant" => Assistant(AssistantLine),
        "stream_event" => StreamEvent(StreamEventLine),
        "control_request" => ControlRequest(ControlRequestLine),
        "control_response" => ControlResponse(ControlResponseLine),
        "result" => Result(ResultLine),
        "error" => Error(ErrorLine),
    }
}

/// A whole assistant message. With partial messages the CLI streams the
/// message first; its tool uses are read from here, since here their input
/// is whole.
#[derive(Deserialize)]
pub(crate) struct AssistantLine {
    #[serde(default)]
    message: Option<AssistantMessage>,
}

#[derive(Deserialize)]
struct AssistantMessage {
    #[serde(default)]
    content: Option<Vec<Tagged<ContentBlock>>>,
}

impl AssistantLine {
    pub(crate) fn content(self) -> impl Iterator<Item = ContentBlock> {
        self.message
            .and_then(|message| message.content)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|block| block.0)
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
    /// with it the model's batch of tool calls.
    pub(crate) enum StreamEvent {
        "content_block_start" => BlockStart(BlockStart),
        "content_block_delta" => BlockDelta(BlockDelta),
        "message_stop" => MessageStop(MessageStop),
    }
}

#[derive(Deserialize)]
pub(crate) struct BlockStart {
    #[serde(default)]
    content_block: Option<Tagged<ContentBlock>>,
}

#[derive(Deserialize)]
pub(crate) struct BlockDelta {
    #[serde(default)]
    delta: Option<Tagged<Delta>>,
}

#[derive(Deserialize)]
pub(crate) struct MessageStop {}

tagged_wire! {
    pub(crate) enum Delta {
        "text_delta" => Text(TextDelta),
        "thinking_delta" => Thinking(ThinkingDelta),
        "signature_delta" => Signature(SignatureDelta),
    }
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
    #[serde(default)]
    usage: Option<ResultUsage>,
}

/// The token counts of a turn: `usage` sums every API call of the turn, and
/// `iterations` lists the calls one by one.
#[derive(Deserialize)]
struct ResultUsage {
    #[serde(flatten)]
    total: UsageCounts,
    #[serde(default)]
    iterations: Option<Vec<UsageCounts>>,
}

/// Token counts, in the API's spelling or in Demi's own.
#[derive(Deserialize)]
struct UsageCounts {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default, rename = "inputTokens")]
    input_tokens_camel: Option<u64>,
    #[serde(default, rename = "outputTokens")]
    output_tokens_camel: Option<u64>,
    #[serde(default, rename = "cacheReadTokens")]
    cache_read_tokens_camel: Option<u64>,
    #[serde(default, rename = "cacheWriteTokens")]
    cache_write_tokens_camel: Option<u64>,
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
    /// The events of a streamed piece: a thinking block's start, and the
    /// text, reasoning and signatures as they come. A tool use streams its
    /// input in pieces, so it is read from the whole message instead.
    pub(crate) fn events(self) -> Vec<ProviderEvent> {
        match self {
            Self::BlockStart(start) => match start.content_block.and_then(|block| block.0) {
                Some(ContentBlock::Thinking(_)) => vec![ProviderEvent::ThinkingStart],
                Some(ContentBlock::Text(block)) if !block.text.is_empty() => {
                    vec![ProviderEvent::TextDelta(block.text)]
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
            Self::MessageStop(_) => Vec::new(),
        }
    }
}

/// How a turn ended.
pub(crate) enum TurnEnd {
    /// The usage of the turn's last API call.
    Answered(TokenUsage),
    /// The CLI's error, as it worded it.
    Failed {
        message: String,
        code: Option<ErrorCode>,
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
            let code = ErrorCode::classify(None, &message);
            return TurnEnd::Failed { message, code };
        }
        TurnEnd::Answered(self.usage.map(ResultUsage::last_call).unwrap_or_default())
    }
}

impl ResultUsage {
    /// The usage of the turn's last API call, which the response carries:
    /// the last iteration when the CLI lists them, else the turn's total.
    fn last_call(self) -> TokenUsage {
        let last = self.iterations.and_then(|mut iterations| iterations.pop());
        last.unwrap_or(self.total).usage()
    }
}

impl UsageCounts {
    fn usage(self) -> TokenUsage {
        TokenUsage {
            input_tokens: self.input_tokens.or(self.input_tokens_camel).unwrap_or(0),
            output_tokens: self.output_tokens.or(self.output_tokens_camel).unwrap_or(0),
            cache_read_tokens: self
                .cache_read_input_tokens
                .or(self.cache_read_tokens_camel)
                .unwrap_or(0),
            cache_write_tokens: self
                .cache_creation_input_tokens
                .or(self.cache_write_tokens_camel)
                .unwrap_or(0),
        }
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
