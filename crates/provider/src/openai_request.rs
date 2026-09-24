//! The request side of the OpenAI-shaped formats, shared by the providers
//! that build them: the Responses input as the OpenAI API and the Codex
//! backend each want a transcript replayed, the Chat Completions messages of
//! OpenAI-compatible endpoints and the Grok Build proxy, and the fields both
//! formats spell alike. Each vendor's differences are a dialect, stated where
//! the vendor's provider builds its body.

use std::borrow::Cow;

use base64::{Engine, engine::general_purpose::STANDARD};
use demi_core::{
    DocumentSource, MediaSource, ThinkingConfig, ThinkingSummary, ToolMediaSource,
    ToolResultContentBlock, UserContentBlock, attachment_tag,
};
use serde::Serialize;

use crate::{
    InferenceItem, ToolDefinition, UnloadedMedia,
    wire::responses::{ReasoningItem, split_tool_use_id},
};

/// The effort a Chat Completions request asks for as `reasoning_effort`.
/// These endpoints level thinking by effort only, so a token budget or
/// thinking turned off names none, and the field is left out.
pub fn reasoning_effort(thinking: Option<&ThinkingConfig>) -> Option<&str> {
    match thinking? {
        ThinkingConfig::Adaptive { effort } | ThinkingConfig::Effort { effort, .. } => Some(effort),
        ThinkingConfig::Budget { .. } | ThinkingConfig::Disabled {} => None,
    }
}

/// A session id as a Responses `prompt_cache_key`, which the API limits to
/// 64 characters (UTF-16 code units): a longer id becomes `session_` and its
/// [`short_hash`].
pub fn prompt_cache_key(session_id: &str) -> Cow<'_, str> {
    if session_id.encode_utf16().count() <= 64 {
        Cow::Borrowed(session_id)
    } else {
        Cow::Owned(format!("session_{}", short_hash(session_id)))
    }
}

/// FNV-1a over the UTF-16 code units of `text`, as lowercase hexadecimal
/// without leading zeros: the short hash of the ids Demi derives for a
/// vendor, such as a long session's cache key.
pub fn short_hash(text: &str) -> String {
    let mut hash: u32 = 2_166_136_261;
    for unit in text.encode_utf16() {
        hash ^= u32::from(unit);
        hash = hash.wrapping_mul(16_777_619);
    }
    format!("{hash:x}")
}

/// The `reasoning` of a Responses request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reasoning<'a> {
    pub effort: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ThinkingSummary>,
}

/// What a vendor's Responses endpoint makes of a summary turned off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryOff {
    /// The field is left out; strict gateways refuse `reasoning.summary`
    /// whatever its value.
    Omitted,
    /// The summary is `auto`, as the Codex backend takes it.
    Auto,
}

/// The `reasoning` a Responses request asks for: none for a token budget or
/// thinking turned off; adaptive thinking at an effort with an `auto`
/// summary; the effort `none` alone; otherwise the effort with the summary
/// asked for, `auto` when unset.
pub fn responses_reasoning(thinking: Option<&ThinkingConfig>, off: SummaryOff) -> Option<Reasoning<'_>> {
    match thinking? {
        ThinkingConfig::Budget { .. } | ThinkingConfig::Disabled {} => None,
        ThinkingConfig::Adaptive { effort } => Some(Reasoning {
            effort,
            summary: Some(ThinkingSummary::Auto),
        }),
        ThinkingConfig::Effort { effort, summary } => {
            if effort == "none" {
                return Some(Reasoning { effort, summary: None });
            }
            let summary = match (summary, off) {
                (Some(ThinkingSummary::Off), SummaryOff::Omitted) => None,
                (Some(ThinkingSummary::Off), SummaryOff::Auto) | (None, _) => Some(ThinkingSummary::Auto),
                (Some(summary), _) => Some(*summary),
            };
            Some(Reasoning { effort, summary })
        }
    }
}

/// A tool definition in a Responses request.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResponsesTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Map<String, serde_json::Value>,
    /// `Some(None)` writes `strict: null`, as the Codex backend receives it.
    #[serde(skip_serializing_if = "Option::is_none")]
    strict: Option<Option<bool>>,
}

impl<'a> ResponsesTool<'a> {
    /// The tool, with `strict: null` when `strict_null`.
    pub fn new(tool: &'a ToolDefinition, strict_null: bool) -> Self {
        Self {
            kind: "function",
            name: &tool.name,
            description: &tool.description,
            parameters: &tool.input_schema,
            strict: strict_null.then_some(None),
        }
    }
}

/// How a vendor's Responses endpoint wants a transcript replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponsesDialect {
    /// The prefix the provider puts on the reasoning items it receives; a
    /// thinking block signed by another vendor is not replayed.
    pub signature_tag: &'static str,
    pub assistant: AssistantReplay,
    pub reasoning: ReasoningReplay,
    pub tool_media: ToolMedia,
}

/// How a replayed assistant message is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantReplay {
    /// Type, role and content only: relay gateways refuse anything else.
    Minimal,
    /// With `status: completed`, which gateways that validate the full item
    /// schema require.
    Completed,
    /// With `status: completed` and an id derived from the item's position,
    /// model and text, as the Codex backend receives it.
    Identified,
}

/// How a replayed reasoning item is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningReplay {
    /// Its type, id, summary, content and encrypted content only: strict
    /// gateways refuse the fields a response adds, such as `status`.
    Replayable,
    /// Whole, with every field the vendor sent.
    Whole,
}

/// Where the images and videos a tool returned go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolMedia {
    /// The tool's output is text, with a placeholder for each image or
    /// video, and a user message after it carries the media: gateways drop
    /// or refuse media inside a tool's output.
    FollowUp,
    /// Images ride inside the tool's output; videos are left out.
    Inline,
}

/// One item of a Responses request's `input`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum InputItem<'a> {
    User(UserInput<'a>),
    Assistant(AssistantInput<'a>),
    Reasoning(ReasoningItem),
    FunctionCall(FunctionCallInput<'a>),
    FunctionCallOutput(FunctionCallOutputInput<'a>),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UserInput<'a> {
    role: &'static str,
    content: Vec<InputPart<'a>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssistantInput<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'static str>,
    content: [OutputText<'a>; 1],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutputText<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'a str,
    annotations: [(); 0],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FunctionCallInput<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    call_id: &'a str,
    name: &'a str,
    arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FunctionCallOutputInput<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: &'a str,
    output: ToolOutput<'a>,
}

/// A tool's output: text, or parts when images ride inside it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ToolOutput<'a> {
    Text(String),
    Parts(Vec<InputPart<'a>>),
}

/// One part of a user message or a tool's output.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputPart<'a> {
    InputText { text: Cow<'a, str> },
    InputImage { image_url: Cow<'a, str>, detail: &'static str },
    InputFile { filename: &'a str, file_data: String },
}

/// The transcript as a Responses request's `input`, in `dialect`.
pub fn responses_input<'a>(
    items: &'a [InferenceItem],
    dialect: &ResponsesDialect,
) -> Result<Vec<InputItem<'a>>, UnloadedMedia> {
    let mut input = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        match item {
            InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
                input.push(InputItem::User(UserInput {
                    role: "user",
                    content: responses_user_parts(content)?,
                }));
            }
            InferenceItem::AssistantText { model_id, text } => {
                let (id, status) = match dialect.assistant {
                    AssistantReplay::Minimal => (None, None),
                    AssistantReplay::Completed => (None, Some("completed")),
                    AssistantReplay::Identified => {
                        let hash = short_hash(&format!("{index}:{model_id}:{text}"));
                        (Some(format!("msg_{hash}")), Some("completed"))
                    }
                };
                input.push(InputItem::Assistant(AssistantInput {
                    kind: "message",
                    role: "assistant",
                    id,
                    status,
                    content: [OutputText {
                        kind: "output_text",
                        text,
                        annotations: [],
                    }],
                }));
            }
            InferenceItem::AssistantThinking { signature, .. } => {
                let reasoning = signature
                    .as_deref()
                    .and_then(|signature| own_reasoning(signature, dialect.signature_tag));
                if let Some(item) = reasoning {
                    input.push(InputItem::Reasoning(match dialect.reasoning {
                        ReasoningReplay::Whole => item,
                        ReasoningReplay::Replayable => ReasoningItem {
                            extra: serde_json::Map::new(),
                            ..item
                        },
                    }));
                }
            }
            InferenceItem::AssistantRedactedThinking { .. } => {}
            InferenceItem::ToolUse {
                tool_use_id,
                tool_name,
                input: arguments,
                ..
            } => {
                let (call_id, item_id) = split_tool_use_id(tool_use_id);
                input.push(InputItem::FunctionCall(FunctionCallInput {
                    kind: "function_call",
                    id: item_id,
                    call_id,
                    name: tool_name,
                    arguments: tool_arguments(arguments),
                }));
            }
            InferenceItem::ToolResult {
                tool_use_id, output, ..
            } => {
                let (call_id, _) = split_tool_use_id(tool_use_id);
                push_tool_result(&mut input, call_id, output, dialect.tool_media)?;
            }
        }
    }
    Ok(input)
}

fn push_tool_result<'a>(
    input: &mut Vec<InputItem<'a>>,
    call_id: &'a str,
    output: &'a [ToolResultContentBlock],
    media: ToolMedia,
) -> Result<(), UnloadedMedia> {
    match media {
        ToolMedia::FollowUp => {
            input.push(InputItem::FunctionCallOutput(FunctionCallOutputInput {
                kind: "function_call_output",
                call_id,
                output: ToolOutput::Text(tool_output_text(output)),
            }));
            let mut parts = Vec::new();
            for block in output {
                if let Some(source) = tool_media_source(block) {
                    parts.push(InputPart::InputImage {
                        image_url: Cow::Owned(tool_media_url(source)?),
                        detail: "auto",
                    });
                }
            }
            if !parts.is_empty() {
                let note = format!("[media returned by tool call {call_id}]");
                parts.insert(0, InputPart::InputText { text: Cow::Owned(note) });
                input.push(InputItem::User(UserInput {
                    role: "user",
                    content: parts,
                }));
            }
        }
        ToolMedia::Inline => {
            let text = output
                .iter()
                .filter_map(|block| match block {
                    ToolResultContentBlock::Text { text } => Some(text.as_str()),
                    ToolResultContentBlock::Image { .. } | ToolResultContentBlock::Video { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            let mut images = Vec::new();
            for block in output {
                if let ToolResultContentBlock::Image { source } = block {
                    images.push(InputPart::InputImage {
                        image_url: Cow::Owned(tool_media_url(source)?),
                        detail: "auto",
                    });
                }
            }
            let output = if images.is_empty() {
                ToolOutput::Text(text)
            } else {
                let mut parts = Vec::with_capacity(images.len() + 1);
                if !text.is_empty() {
                    parts.push(InputPart::InputText { text: Cow::Owned(text) });
                }
                parts.extend(images);
                ToolOutput::Parts(parts)
            };
            input.push(InputItem::FunctionCallOutput(FunctionCallOutputInput {
                kind: "function_call_output",
                call_id,
                output,
            }));
        }
    }
    Ok(())
}

/// The reasoning item a thinking block's signature carries, when this
/// provider's vendor signed it: a signature without the tag, or one that is
/// not a reasoning item, belongs to another provider's transcript.
fn own_reasoning(signature: &str, tag: &str) -> Option<ReasoningItem> {
    let json = signature.strip_prefix(tag)?;
    serde_json::from_str(json).ok()
}

/// A user message's content as Responses parts: text, references and
/// attachment tags as text, PDFs as files, images and videos as images.
fn responses_user_parts(content: &[UserContentBlock]) -> Result<Vec<InputPart<'_>>, UnloadedMedia> {
    content
        .iter()
        .map(|block| {
            Ok(match block {
                UserContentBlock::Text { text } => InputPart::InputText {
                    text: Cow::Borrowed(text),
                },
                UserContentBlock::Reference { reference } => InputPart::InputText {
                    text: Cow::Borrowed(reference),
                },
                UserContentBlock::Attachment(attachment) => InputPart::InputText {
                    text: Cow::Owned(attachment_tag(attachment)),
                },
                UserContentBlock::Document { source } => {
                    let (file_name, file_data) = document_data(source)?;
                    InputPart::InputFile {
                        filename: file_name,
                        file_data,
                    }
                }
                UserContentBlock::Image { source } | UserContentBlock::Video { source } => {
                    InputPart::InputImage {
                        image_url: media_url(source)?,
                        detail: "auto",
                    }
                }
            })
        })
        .collect()
}

/// One message of a Chat Completions request.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum ChatMessage<'a> {
    System {
        content: &'a str,
    },
    User {
        content: ChatContent<'a>,
    },
    Assistant {
        /// `null` when the message is only tool calls.
        content: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ChatToolCall<'a>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
    },
    Tool {
        tool_call_id: &'a str,
        content: String,
    },
}

/// A user message's content: plain text when it is only text, else parts.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ChatContent<'a> {
    Text(String),
    Parts(Vec<ChatPart<'a>>),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatPart<'a> {
    Text { text: Cow<'a, str> },
    ImageUrl { image_url: ImageUrl<'a> },
    File { file: FileData<'a> },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImageUrl<'a> {
    url: Cow<'a, str>,
    detail: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileData<'a> {
    filename: &'a str,
    file_data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatToolCall<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: ChatFunctionCall<'a>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatFunctionCall<'a> {
    name: &'a str,
    arguments: String,
}

/// A tool definition in a Chat Completions request.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: ChatFunction<'a>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Map<String, serde_json::Value>,
}

impl<'a> From<&'a ToolDefinition> for ChatTool<'a> {
    fn from(tool: &'a ToolDefinition) -> Self {
        Self {
            kind: "function",
            function: ChatFunction {
                name: &tool.name,
                description: &tool.description,
                parameters: &tool.input_schema,
            },
        }
    }
}

/// How a vendor's Chat Completions endpoint wants a transcript replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatDialect {
    /// Earlier thinking is sent back as `reasoning_content` on the assistant
    /// message it preceded, and as an empty one on a tool-call message without
    /// thinking, as DeepSeek's thinking mode requires; OpenAI refuses the
    /// field.
    pub reasoning_content: bool,
    pub media: ChatMedia,
}

/// What the endpoint reads besides text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatMedia {
    /// Images and videos as image URLs, PDFs as files, and media a tool
    /// returned in a user message after the tool's output.
    Native,
    /// Images only: a video becomes a placeholder that names it, a PDF is
    /// left to its attachment tag, and a tool's media to its output's
    /// placeholders.
    Images,
}

/// The transcript as Chat Completions messages, in `dialect`: a system
/// message when the prompt is not blank, then consecutive assistant text and
/// tool calls as one assistant message, each tool result as a tool message.
pub fn chat_messages<'a>(
    system_prompt: &'a str,
    items: &'a [InferenceItem],
    dialect: ChatDialect,
) -> Result<Vec<ChatMessage<'a>>, UnloadedMedia> {
    let mut messages = Vec::new();
    if !demi_core::is_blank(system_prompt) {
        messages.push(ChatMessage::System {
            content: system_prompt,
        });
    }
    let mut turn = AssistantTurn::default();
    for item in items {
        match item {
            InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
                turn.flush(&mut messages, dialect);
                messages.push(ChatMessage::User {
                    content: chat_user_content(content, dialect.media)?,
                });
            }
            InferenceItem::AssistantText { text, .. } => {
                turn.open().content.push_str(text);
            }
            InferenceItem::ToolUse {
                tool_use_id,
                tool_name,
                input,
                ..
            } => {
                turn.open().tool_calls.push(ChatToolCall {
                    id: tool_use_id,
                    kind: "function",
                    function: ChatFunctionCall {
                        name: tool_name,
                        arguments: tool_arguments(input),
                    },
                });
            }
            InferenceItem::ToolResult {
                tool_use_id, output, ..
            } => {
                turn.flush(&mut messages, dialect);
                messages.push(ChatMessage::Tool {
                    tool_call_id: tool_use_id,
                    content: tool_output_text(output),
                });
                if dialect.media == ChatMedia::Native {
                    push_tool_media(&mut messages, tool_use_id, output)?;
                }
            }
            InferenceItem::AssistantThinking { text, .. } => {
                if dialect.reasoning_content {
                    turn.think(text);
                }
            }
            InferenceItem::AssistantRedactedThinking { .. } => {}
        }
    }
    turn.flush(&mut messages, dialect);
    Ok(messages)
}

/// The assistant message being assembled, and thinking that came before it.
#[derive(Default)]
struct AssistantTurn<'a> {
    open: Option<OpenAssistant<'a>>,
    /// Thinking not yet followed by assistant text or a tool call.
    pending_reasoning: String,
}

struct OpenAssistant<'a> {
    content: String,
    tool_calls: Vec<ChatToolCall<'a>>,
    reasoning: String,
}

impl<'a> AssistantTurn<'a> {
    /// The assistant message being assembled, opened with the thinking
    /// before it.
    fn open(&mut self) -> &mut OpenAssistant<'a> {
        let pending = &mut self.pending_reasoning;
        self.open.get_or_insert_with(|| OpenAssistant {
            content: String::new(),
            tool_calls: Vec::new(),
            reasoning: std::mem::take(pending),
        })
    }

    fn think(&mut self, text: &str) {
        match &mut self.open {
            Some(open) => open.reasoning.push_str(text),
            None => self.pending_reasoning.push_str(text),
        }
    }

    /// Ends the assistant message; thinking that no message followed goes
    /// with it, since it belonged to nothing the vendor will see.
    fn flush(&mut self, messages: &mut Vec<ChatMessage<'a>>, dialect: ChatDialect) {
        self.pending_reasoning.clear();
        let Some(open) = self.open.take() else {
            return;
        };
        let reasoning_content = (dialect.reasoning_content
            && (!open.reasoning.is_empty() || !open.tool_calls.is_empty()))
        .then_some(open.reasoning);
        messages.push(ChatMessage::Assistant {
            content: (!open.content.is_empty()).then_some(open.content),
            tool_calls: open.tool_calls,
            reasoning_content,
        });
    }
}

/// A user message's content as Chat Completions parts, as plain text when
/// every part is text.
fn chat_user_content(content: &[UserContentBlock], media: ChatMedia) -> Result<ChatContent<'_>, UnloadedMedia> {
    let mut parts = Vec::new();
    for block in content {
        match block {
            UserContentBlock::Text { text } => parts.push(ChatPart::Text {
                text: Cow::Borrowed(text),
            }),
            UserContentBlock::Reference { reference } => parts.push(ChatPart::Text {
                text: Cow::Borrowed(reference),
            }),
            UserContentBlock::Attachment(attachment) => parts.push(ChatPart::Text {
                text: Cow::Owned(attachment_tag(attachment)),
            }),
            UserContentBlock::Document { source } => {
                if media == ChatMedia::Native {
                    let (file_name, file_data) = document_data(source)?;
                    parts.push(ChatPart::File {
                        file: FileData {
                            filename: file_name,
                            file_data,
                        },
                    });
                }
            }
            UserContentBlock::Video { source } if media == ChatMedia::Images => {
                let named = match source {
                    MediaSource::Url { url } => url,
                    MediaSource::Binary { media_type, .. } | MediaSource::Ref { media_type, .. } => media_type,
                };
                parts.push(ChatPart::Text {
                    text: Cow::Owned(format!("[video:{named}]")),
                });
            }
            UserContentBlock::Image { source } | UserContentBlock::Video { source } => {
                parts.push(ChatPart::ImageUrl {
                    image_url: ImageUrl {
                        url: media_url(source)?,
                        detail: "auto",
                    },
                });
            }
        }
    }
    let texts: Option<Vec<&str>> = parts
        .iter()
        .map(|part| match part {
            ChatPart::Text { text } => Some(text.as_ref()),
            ChatPart::ImageUrl { .. } | ChatPart::File { .. } => None,
        })
        .collect();
    Ok(match texts {
        Some(texts) => ChatContent::Text(texts.join("\n")),
        None => ChatContent::Parts(parts),
    })
}

/// The images and videos a tool returned, in a user message after the tool's
/// output: a tool message is text only.
fn push_tool_media<'a>(
    messages: &mut Vec<ChatMessage<'a>>,
    tool_use_id: &str,
    output: &'a [ToolResultContentBlock],
) -> Result<(), UnloadedMedia> {
    let mut parts = Vec::new();
    for block in output {
        if let Some(source) = tool_media_source(block) {
            parts.push(ChatPart::ImageUrl {
                image_url: ImageUrl {
                    url: Cow::Owned(tool_media_url(source)?),
                    detail: "auto",
                },
            });
        }
    }
    if !parts.is_empty() {
        let note = format!("[media returned by tool call {tool_use_id}]");
        parts.insert(0, ChatPart::Text { text: Cow::Owned(note) });
        messages.push(ChatMessage::User {
            content: ChatContent::Parts(parts),
        });
    }
    Ok(())
}

/// A tool's output as text, for formats without media in a tool's output:
/// each image or video becomes a placeholder that names its type, such as
/// `[image:image/png]`.
pub fn tool_output_text(output: &[ToolResultContentBlock]) -> String {
    output
        .iter()
        .map(|block| match block {
            ToolResultContentBlock::Text { text } => text.clone(),
            ToolResultContentBlock::Image { source } => format!("[image:{}]", tool_media_type(source)),
            ToolResultContentBlock::Video { source } => format!("[video:{}]", tool_media_type(source)),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A tool call's input as the arguments text a vendor receives: a string as
/// it is, any other value as JSON, and no input as an empty object.
pub fn tool_arguments(input: &serde_json::Value) -> String {
    match input {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => "{}".to_owned(),
        value => value.to_string(),
    }
}

fn tool_media_source(block: &ToolResultContentBlock) -> Option<&ToolMediaSource> {
    match block {
        ToolResultContentBlock::Image { source } | ToolResultContentBlock::Video { source } => Some(source),
        ToolResultContentBlock::Text { .. } => None,
    }
}

fn tool_media_type(source: &ToolMediaSource) -> &str {
    match source {
        ToolMediaSource::Binary { media_type, .. } | ToolMediaSource::Ref { media_type, .. } => media_type,
    }
}

fn tool_media_url(source: &ToolMediaSource) -> Result<String, UnloadedMedia> {
    match source {
        ToolMediaSource::Binary { data, media_type } => Ok(data_url(media_type, data)),
        ToolMediaSource::Ref { r#ref, .. } => Err(UnloadedMedia(r#ref.to_string())),
    }
}

fn media_url(source: &MediaSource) -> Result<Cow<'_, str>, UnloadedMedia> {
    match source {
        MediaSource::Binary { data, media_type } => Ok(Cow::Owned(data_url(media_type, data))),
        MediaSource::Url { url } => Ok(Cow::Borrowed(url)),
        MediaSource::Ref { r#ref, .. } => Err(UnloadedMedia(r#ref.to_string())),
    }
}

/// A PDF's file name and its bytes as a data URL.
fn document_data(source: &DocumentSource) -> Result<(&str, String), UnloadedMedia> {
    match source {
        DocumentSource::Binary {
            data,
            media_type,
            file_name,
        } => Ok((file_name, data_url(media_type, data))),
        DocumentSource::Ref { r#ref, .. } => Err(UnloadedMedia(r#ref.to_string())),
    }
}

/// Bytes as a `data:` URL of `media_type`, base64 encoded.
fn data_url(media_type: &str, bytes: &[u8]) -> String {
    format!("data:{media_type};base64,{}", STANDARD.encode(bytes))
}
