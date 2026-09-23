//! The Messages API request body (`models.md` § Request parameters): the
//! transcript as alternating user and assistant messages, the tools, the
//! output limit, thinking and the service tier.

use std::borrow::Cow;

use demi_core::{
    B64Bytes, DocumentSource, MediaSource, ThinkingConfig, ThinkingSummary, ToolMediaSource,
    ToolResultContentBlock, UserContentBlock, attachment_tag, is_blank,
};
use demi_provider::{InferenceItem, InferenceRequest, ToolDefinition};
use serde::Serialize;

/// The prefix this provider puts on the signatures and redacted data it
/// receives, so that on replay it sends back only its vendor's: another
/// vendor's signature would fail the vendor's check.
pub(crate) const SIGNATURE_TAG: &str = "anthropic:";

/// `max_tokens` when the model names no output limit: agent turns stream long
/// tool-heavy answers, which small defaults cut off.
const DEFAULT_MAX_TOKENS: u32 = 32_000;

/// The smallest thinking budget the API accepts, which is also what a budget
/// keeps below `max_tokens`.
const MIN_THINKING_BUDGET: u32 = 1_024;

/// Media the session did not load back before the request: it names the
/// blob.
#[derive(Debug)]
pub(crate) struct UnloadedMedia(pub(crate) String);

/// The JSON body of `request`.
pub(crate) fn encode(request: &InferenceRequest) -> Result<Vec<u8>, UnloadedMedia> {
    let body = body(request)?;
    // The body is strings, numbers and JSON values, which always serialize.
    Ok(serde_json::to_vec(&body).expect("a request body serializes"))
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    max_tokens: u32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Tool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Thinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<OutputConfig<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<&'a str>,
}

#[derive(Serialize)]
struct Message<'a> {
    role: Role,
    content: Vec<Block<'a>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    User,
    Assistant,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block<'a> {
    Text {
        text: Cow<'a, str>,
    },
    Image {
        source: Base64<'a>,
    },
    Document {
        source: Base64<'a>,
        title: &'a str,
    },
    ToolUse {
        id: &'a str,
        name: &'a str,
        input: Cow<'a, serde_json::Value>,
    },
    ToolResult {
        tool_use_id: &'a str,
        content: Vec<ResultBlock<'a>>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
    Thinking {
        thinking: &'a str,
        signature: &'a str,
    },
    RedactedThinking {
        data: &'a str,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResultBlock<'a> {
    Text { text: Cow<'a, str> },
    Image { source: Base64<'a> },
}

/// Inline bytes; `data` serializes as base64.
#[derive(Serialize)]
struct Base64<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    media_type: &'a str,
    data: &'a B64Bytes,
}

impl<'a> Base64<'a> {
    fn new(media_type: &'a str, data: &'a B64Bytes) -> Self {
        Self {
            kind: "base64",
            media_type,
            data,
        }
    }
}

#[derive(Serialize)]
struct Tool<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a serde_json::Map<String, serde_json::Value>,
}

impl<'a> From<&'a ToolDefinition> for Tool<'a> {
    fn from(tool: &'a ToolDefinition) -> Self {
        Self {
            name: &tool.name,
            description: &tool.description,
            input_schema: &tool.input_schema,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Thinking {
    /// A fixed token budget.
    Enabled { budget_tokens: u32 },
    /// Thinking whose depth the effort level sets.
    Adaptive { display: Display },
}

/// Whether a thinking block streams a readable summary or empty text.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Display {
    Summarized,
    Omitted,
}

#[derive(Serialize)]
struct OutputConfig<'a> {
    effort: &'a str,
}

fn body(request: &InferenceRequest) -> Result<Body<'_>, UnloadedMedia> {
    let max_tokens = request
        .output_limit
        .map_or(DEFAULT_MAX_TOKENS, |limit| limit.get());
    let (thinking, output_config) = thinking(request.thinking.as_ref(), max_tokens);
    Ok(Body {
        model: &request.model_id,
        messages: messages(&request.items)?,
        max_tokens,
        stream: true,
        system: (!is_blank(&request.system_prompt)).then_some(request.system_prompt.as_str()),
        tools: request.tools.iter().map(Tool::from).collect(),
        thinking,
        output_config,
        service_tier: request.service_tier_id.as_deref(),
    })
}

/// The thinking setting as the Messages API takes it: a budget as a budget,
/// kept at least the API's minimum and below `max_tokens`; an effort or an
/// adaptive setting as adaptive thinking at that effort, which streams a
/// summary unless the setting turns summaries off; off or no setting as no
/// thinking field.
fn thinking(
    config: Option<&ThinkingConfig>,
    max_tokens: u32,
) -> (Option<Thinking>, Option<OutputConfig<'_>>) {
    match config {
        None | Some(ThinkingConfig::Disabled {}) => (None, None),
        Some(ThinkingConfig::Budget { budget_tokens }) => {
            let ceiling = MIN_THINKING_BUDGET.max(max_tokens.saturating_sub(MIN_THINKING_BUDGET));
            let budget_tokens = (*budget_tokens).min(ceiling).max(MIN_THINKING_BUDGET);
            (Some(Thinking::Enabled { budget_tokens }), None)
        }
        Some(ThinkingConfig::Adaptive { effort }) => (
            Some(Thinking::Adaptive {
                display: Display::Summarized,
            }),
            Some(OutputConfig { effort }),
        ),
        Some(ThinkingConfig::Effort { effort, summary }) => {
            let display = match summary {
                Some(ThinkingSummary::Off) => Display::Omitted,
                _ => Display::Summarized,
            };
            (
                Some(Thinking::Adaptive { display }),
                Some(OutputConfig { effort }),
            )
        }
    }
}

/// The transcript as messages: consecutive items of one role share a
/// message, so a turn's text, thinking and tool uses form one assistant
/// message and the tool results that answer them one user message.
fn messages(items: &[InferenceItem]) -> Result<Vec<Message<'_>>, UnloadedMedia> {
    let mut messages: Vec<Message<'_>> = Vec::new();
    for item in items {
        let (role, content) = match item {
            InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
                (Role::User, user_content(content)?)
            }
            InferenceItem::AssistantText { text, .. } => (
                Role::Assistant,
                vec![Block::Text {
                    text: Cow::Borrowed(text),
                }],
            ),
            InferenceItem::AssistantThinking {
                text, signature, ..
            } => {
                // Unsigned thinking and another vendor's signature cannot be
                // sent back; the vendor needs only its own.
                let Some(signature) = signature.as_deref().and_then(own) else {
                    continue;
                };
                (
                    Role::Assistant,
                    vec![Block::Thinking {
                        thinking: text,
                        signature,
                    }],
                )
            }
            InferenceItem::AssistantRedactedThinking { data, .. } => {
                let Some(data) = own(data) else {
                    continue;
                };
                (Role::Assistant, vec![Block::RedactedThinking { data }])
            }
            InferenceItem::ToolUse {
                tool_use_id,
                tool_name,
                input,
                ..
            } => {
                let input = match input {
                    serde_json::Value::Null => Cow::Owned(serde_json::Value::Object(Default::default())),
                    input => Cow::Borrowed(input),
                };
                (
                    Role::Assistant,
                    vec![Block::ToolUse {
                        id: tool_use_id,
                        name: tool_name,
                        input,
                    }],
                )
            }
            InferenceItem::ToolResult {
                tool_use_id,
                output,
                is_error,
            } => (
                Role::User,
                vec![Block::ToolResult {
                    tool_use_id,
                    content: tool_result_content(output)?,
                    is_error: *is_error,
                }],
            ),
        };
        append(&mut messages, role, content);
    }
    Ok(messages)
}

fn append<'a>(messages: &mut Vec<Message<'a>>, role: Role, mut content: Vec<Block<'a>>) {
    if content.is_empty() {
        return;
    }
    match messages.last_mut() {
        Some(last) if last.role == role => last.content.append(&mut content),
        _ => messages.push(Message { role, content }),
    }
}

/// A signature or redacted data this provider received, without its tag.
fn own(tagged: &str) -> Option<&str> {
    tagged.strip_prefix(SIGNATURE_TAG)
}

/// A message's content: text, references and attachment tags as text, images
/// and PDFs inline. The API has no video block, and the catalog marks video
/// unsupported, so a video becomes a placeholder.
fn user_content(content: &[UserContentBlock]) -> Result<Vec<Block<'_>>, UnloadedMedia> {
    content
        .iter()
        .map(|block| {
            Ok(match block {
                UserContentBlock::Text { text } => Block::Text {
                    text: Cow::Borrowed(text),
                },
                UserContentBlock::Reference { reference } => Block::Text {
                    text: Cow::Borrowed(reference),
                },
                UserContentBlock::Attachment(attachment) => Block::Text {
                    text: Cow::Owned(attachment_tag(attachment)),
                },
                UserContentBlock::Video { .. } => Block::Text {
                    text: Cow::Borrowed("[video]"),
                },
                UserContentBlock::Image { source } => match source {
                    MediaSource::Binary { data, media_type } => Block::Image {
                        source: Base64::new(media_type, data),
                    },
                    MediaSource::Url { url } => Block::Text {
                        text: Cow::Owned(format!("[image:{url}]")),
                    },
                    MediaSource::Ref { r#ref, .. } => return Err(UnloadedMedia(r#ref.to_string())),
                },
                UserContentBlock::Document { source } => match source {
                    DocumentSource::Binary {
                        data,
                        media_type,
                        file_name,
                    } => Block::Document {
                        source: Base64::new(media_type, data),
                        title: file_name,
                    },
                    DocumentSource::Ref { r#ref, .. } => return Err(UnloadedMedia(r#ref.to_string())),
                },
            })
        })
        .collect()
}

/// A tool result's content: text and images; a video becomes a placeholder
/// that names its type.
fn tool_result_content(
    output: &[ToolResultContentBlock],
) -> Result<Vec<ResultBlock<'_>>, UnloadedMedia> {
    output
        .iter()
        .map(|block| {
            Ok(match block {
                ToolResultContentBlock::Text { text } => ResultBlock::Text {
                    text: Cow::Borrowed(text),
                },
                ToolResultContentBlock::Video { source } => {
                    let media_type = match source {
                        ToolMediaSource::Binary { media_type, .. }
                        | ToolMediaSource::Ref { media_type, .. } => media_type,
                    };
                    ResultBlock::Text {
                        text: Cow::Owned(format!("[video:{media_type}]")),
                    }
                }
                ToolResultContentBlock::Image { source } => match source {
                    ToolMediaSource::Binary { data, media_type } => ResultBlock::Image {
                        source: Base64::new(media_type, data),
                    },
                    ToolMediaSource::Ref { r#ref, .. } => return Err(UnloadedMedia(r#ref.to_string())),
                },
            })
        })
        .collect()
}
