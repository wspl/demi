//! The Messages API request body (`models.md` § Request parameters): the
//! transcript as alternating user and assistant messages, the tools, the
//! output limit, thinking, the service tier, and the marks where the vendor
//! caches a session's requests (`providers.md` § Per vendor).

use std::borrow::Cow;

use demi_provider_common::{
    InferenceItem, InferenceRequest, MediaBytes, Medium, PromptCache, ResultPart, ToolDefinition,
    UserPart, json_body,
};
use demi_shared_types::{B64Bytes, ThinkingConfig, ThinkingSummary, is_blank};
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

/// How long the vendor keeps an entry after the request that wrote or last
/// read it: an hour, because a session's requests are often more than five
/// minutes apart, while a tool runs or the user reads.
const CACHE_LIFETIME: &str = "1h";

/// A mark on the block where the vendor writes a cache entry, and where a
/// later request reads it back.
const CACHE_MARK: CacheControl = CacheControl {
    kind: "ephemeral",
    ttl: CACHE_LIFETIME,
};

/// The JSON body of `request`.
pub(crate) fn encode(request: &InferenceRequest) -> Vec<u8> {
    json_body(&body(request))
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    max_tokens: u32,
    stream: bool,
    /// The system prompt as one text block, which can carry a mark.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    system: Vec<Content<'a>>,
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
    content: Vec<Content<'a>>,
}

/// A block's place in the messages: its message's index and its own.
type Position = (usize, usize);

/// A content block with the cache mark it may carry.
#[derive(Serialize)]
struct Content<'a> {
    #[serde(flatten)]
    block: Block<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

impl<'a> From<Block<'a>> for Content<'a> {
    fn from(block: Block<'a>) -> Self {
        Self {
            block,
            cache_control: None,
        }
    }
}

impl Content<'_> {
    /// Whether the API takes a mark on this block: it takes none on
    /// reasoning.
    fn can_mark(&self) -> bool {
        !matches!(
            self.block,
            Block::Thinking { .. } | Block::RedactedThinking { .. }
        )
    }
}

#[derive(Clone, Copy, Serialize)]
struct CacheControl {
    #[serde(rename = "type")]
    kind: &'static str,
    ttl: &'static str,
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

impl<'a> From<&'a MediaBytes> for Base64<'a> {
    fn from(bytes: &'a MediaBytes) -> Self {
        Self {
            kind: "base64",
            media_type: &bytes.media_type,
            data: &bytes.data,
        }
    }
}

#[derive(Serialize)]
struct Tool<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

impl<'a> From<&'a ToolDefinition> for Tool<'a> {
    fn from(tool: &'a ToolDefinition) -> Self {
        Self {
            name: &tool.name,
            description: &tool.description,
            input_schema: &tool.input_schema,
            cache_control: None,
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

fn body(request: &InferenceRequest) -> Body<'_> {
    let max_tokens = request
        .max_output_tokens()
        .map_or(DEFAULT_MAX_TOKENS, |limit| limit.get());
    let (thinking, output_config) = thinking(request.thinking.as_ref(), max_tokens);
    let mut system = Vec::new();
    if !is_blank(&request.system_prompt) {
        let prompt = Block::Text {
            text: Cow::Borrowed(&request.system_prompt),
        };
        system.push(Content::from(prompt));
    }
    let mut tools: Vec<Tool<'_>> = request.tools.iter().map(Tool::from).collect();
    let messages = match request.prompt_cache {
        PromptCache::Off => messages(&request.items, 0).0,
        PromptCache::Session { answered_items } => {
            let (mut messages, answered_end) = messages(&request.items, answered_items);
            mark_shared_prefix(&mut system, &mut tools);
            let request_end = last_position(&messages);
            for position in [answered_end, request_end].into_iter().flatten() {
                mark_at(&mut messages, position);
            }
            messages
        }
    };
    Body {
        model: &request.model_id,
        messages,
        max_tokens,
        stream: true,
        system,
        tools,
        thinking,
        output_config,
        service_tier: request.service_tier_id.as_deref(),
    }
}

/// Marks the end of what the nodes of one harness and profile share: the
/// system prompt, or the last tool when the prompt is blank. Its entry serves
/// a conversation's first request and each request after a summary.
fn mark_shared_prefix(system: &mut [Content<'_>], tools: &mut [Tool<'_>]) {
    if let Some(prompt) = system.last_mut() {
        prompt.cache_control = Some(CACHE_MARK);
    } else if let Some(tool) = tools.last_mut() {
        tool.cache_control = Some(CACHE_MARK);
    }
}

/// Marks the block at `position`, or the nearest block before it that can
/// carry a mark.
fn mark_at(messages: &mut [Message<'_>], position: Position) {
    let (last_message, last_block) = position;
    for index in (0..=last_message).rev() {
        let content = &mut messages[index].content;
        let end = if index == last_message {
            last_block + 1
        } else {
            content.len()
        };
        if let Some(block) = content[..end]
            .iter_mut()
            .rev()
            .find(|block| block.can_mark())
        {
            block.cache_control = Some(CACHE_MARK);
            return;
        }
    }
}

/// The position of the last block of `messages`; every message holds one.
fn last_position(messages: &[Message<'_>]) -> Option<Position> {
    let last = messages.last()?;
    Some((messages.len() - 1, last.content.len() - 1))
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
/// message and the tool results that answer them one user message. Also the
/// position of the last block of the first `answered` items, none while they
/// have no block.
fn messages(items: &[InferenceItem], answered: usize) -> (Vec<Message<'_>>, Option<Position>) {
    let mut messages: Vec<Message<'_>> = Vec::new();
    let mut answered_end = None;
    for (count, item) in (1..).zip(items) {
        if let Some((role, content)) = item_content(item) {
            append(&mut messages, role, content);
        }
        if count == answered {
            answered_end = last_position(&messages);
        }
    }
    (messages, answered_end)
}

/// What one item adds to the messages: its role and blocks, or nothing for
/// reasoning this provider cannot send back.
fn item_content(item: &InferenceItem) -> Option<(Role, Vec<Content<'_>>)> {
    let (role, blocks) = match item {
        InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
            (Role::User, user_content(content))
        }
        InferenceItem::AssistantText { text, .. } => (
            Role::Assistant,
            vec![Block::Text {
                text: Cow::Borrowed(text),
            }],
        ),
        InferenceItem::AssistantThinking {
            text,
            signature,
            kept_past_summary,
            ..
        } => {
            // Unsigned thinking and another vendor's signature cannot be
            // sent back; the vendor needs only its own. Reasoning kept past
            // a summary would fail the vendor's check of the history before
            // it, and leaving it out at the start of the history is allowed.
            if *kept_past_summary {
                return None;
            }
            let signature = signature.as_deref().and_then(own)?;
            (
                Role::Assistant,
                vec![Block::Thinking {
                    thinking: text,
                    signature,
                }],
            )
        }
        InferenceItem::AssistantRedactedThinking {
            data,
            kept_past_summary,
            ..
        } => {
            let data = own(data).filter(|_| !kept_past_summary)?;
            (Role::Assistant, vec![Block::RedactedThinking { data }])
        }
        InferenceItem::ToolUse {
            tool_use_id,
            tool_name,
            input,
            ..
        } => {
            let input = match input {
                serde_json::Value::Null => {
                    Cow::Owned(serde_json::Value::Object(Default::default()))
                }
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
                content: tool_result_content(output),
                is_error: *is_error,
            }],
        ),
    };
    let content = blocks.into_iter().map(Content::from).collect();
    Some((role, content))
}

fn append<'a>(messages: &mut Vec<Message<'a>>, role: Role, mut content: Vec<Content<'a>>) {
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

/// A message's content: text as text, images and PDFs inline, and an image
/// by URL as a text that names it. The API has no video block, and the
/// catalog marks video unsupported, so a video becomes a placeholder.
fn user_content(content: &[UserPart]) -> Vec<Block<'_>> {
    content
        .iter()
        .map(|part| match part {
            UserPart::Text(text) => Block::Text {
                text: Cow::Borrowed(text),
            },
            UserPart::Video(_) => Block::Text {
                text: Cow::Borrowed("[video]"),
            },
            UserPart::Image(Medium::Bytes(bytes)) => Block::Image {
                source: Base64::from(bytes),
            },
            UserPart::Image(Medium::Url(url)) => Block::Text {
                text: Cow::Owned(format!("[image:{url}]")),
            },
            UserPart::Document { bytes, file_name } => Block::Document {
                source: Base64::from(bytes),
                title: file_name,
            },
        })
        .collect()
}

/// A tool result's content: text and images; a video becomes a placeholder
/// that names its type.
fn tool_result_content(output: &[ResultPart]) -> Vec<ResultBlock<'_>> {
    output
        .iter()
        .map(|part| match part {
            ResultPart::Text(text) => ResultBlock::Text {
                text: Cow::Borrowed(text),
            },
            ResultPart::Video(bytes) => ResultBlock::Text {
                text: Cow::Owned(format!("[video:{}]", bytes.media_type)),
            },
            ResultPart::Image(bytes) => ResultBlock::Image {
                source: Base64::from(bytes),
            },
        })
        .collect()
}
