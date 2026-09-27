//! What Demi writes to the CLI's standard input (`claude-code.md` §
//! Requests over stream-json): the `initialize` control request that
//! declares the SDK MCP server, the transcript a new process starts from,
//! the user messages a kept process gains, and the answers to the CLI's
//! control requests. Each is one JSON line.

use std::borrow::Cow;

use demi_provider::openai_request::tool_output_text;
use demi_provider::{InferenceItem, Medium, UserPart, json_body};
use rmcp::model::ServerJsonRpcMessage;
use serde::Serialize;

/// The one SDK MCP server a process is told about.
pub(crate) const MCP_SERVER: &str = "main";

/// One line Demi writes.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Input<'a> {
    User {
        message: Message<'a>,
    },
    ControlRequest {
        request_id: &'a str,
        request: ControlRequest<'a>,
    },
    ControlResponse {
        response: ControlResponse<'a>,
    },
}

/// A request Demi sends the CLI: only `initialize`, which declares the SDK
/// MCP server.
#[derive(Serialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
pub(crate) enum ControlRequest<'a> {
    Initialize {
        #[serde(rename = "sdkMcpServers")]
        sdk_mcp_servers: [&'a str; 1],
        #[serde(rename = "systemPrompt")]
        system_prompt: &'a str,
    },
}

/// The one answer to a control request of the CLI's.
#[derive(Serialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
pub(crate) enum ControlResponse<'a> {
    /// The SDK MCP server's reply to the message the request carried.
    Success {
        request_id: &'a str,
        response: McpReply<'a>,
    },
    /// A request Demi does not serve.
    Error { request_id: &'a str, error: String },
}

#[derive(Serialize)]
pub(crate) struct McpReply<'a> {
    pub(crate) mcp_response: &'a ServerJsonRpcMessage,
}

/// A message of the conversation.
#[derive(Serialize)]
pub(crate) struct Message<'a> {
    role: Role,
    content: Vec<Block<'a>>,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Role {
    User,
    Assistant,
}

impl Role {
    /// How the transcript a new process starts from names the speaker of a
    /// part.
    fn speaker(self) -> &'static str {
        match self {
            Self::User => "User:",
            Self::Assistant => "Assistant:",
        }
    }
}

/// A content block, as the Messages API spells it.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block<'a> {
    Text { text: Cow<'a, str> },
    Image { source: ImageSource<'a> },
    Document { source: Base64<'a>, title: &'a str },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ImageSource<'a> {
    Base64 {
        media_type: &'a str,
        data: &'a demi_core::B64Bytes,
    },
    Url {
        url: &'a str,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename = "base64")]
struct Base64<'a> {
    media_type: &'a str,
    data: &'a demi_core::B64Bytes,
}

/// `input` as the line Demi writes.
pub(crate) fn line(input: &Input<'_>) -> Vec<u8> {
    let mut line = json_body(input);
    line.push(b'\n');
    line
}

/// The transcript as the one user message a new process starts from
/// (`claude-code.md` § Starting): the CLI keeps no order among the lines of
/// a history, so the transcript is written as text in order, each speaker's
/// part opening with `User:` or `Assistant:`. A user's part holds only the
/// user's real input, with images and documents as blocks in their places;
/// earlier tool calls and their results are the model's own words, since a
/// structured tool call replayed into a new SDK MCP session makes the vendor
/// refuse the request; earlier reasoning is left out, since its signatures do
/// not hold for a new process. A transcript of the user's input alone is
/// written as that input.
pub(crate) fn transcript(items: &[InferenceItem]) -> Vec<u8> {
    let mut parts = parts(items);
    let input_alone = matches!(parts.as_slice(), [only] if only.role == Role::User);
    let content = if input_alone {
        parts.pop().map(|part| part.content).unwrap_or_default()
    } else {
        spoken(parts)
    };
    let message = Message {
        role: Role::User,
        content,
    };
    line(&Input::User { message })
}

/// The transcript's parts: the items of one speaker in a row, each as the
/// content blocks it becomes, without the parts that become none.
fn parts(items: &[InferenceItem]) -> Vec<Message<'_>> {
    let mut parts: Vec<Message<'_>> = Vec::new();
    for item in items {
        match item {
            InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
                append(&mut parts, Role::User, user_content(content));
            }
            InferenceItem::AssistantText { text, .. } if !text.is_empty() => {
                append(&mut parts, Role::Assistant, vec![text_block(text)]);
            }
            InferenceItem::ToolUse {
                tool_name, input, ..
            } => {
                let text = format!(
                    "[Earlier in this conversation I called the tool {tool_name} with input: {input}."
                );
                append(&mut parts, Role::Assistant, vec![Block::Text { text: text.into() }]);
            }
            InferenceItem::ToolResult {
                tool_use_id,
                output,
                is_error,
            } => {
                let from = tool_name_of(items, tool_use_id)
                    .map(|name| format!(" from {name}"))
                    .unwrap_or_default();
                let body = tool_output_text(output);
                let text = if *is_error {
                    format!("It returned an error{from}: {body}]")
                } else {
                    format!("It returned{from}: {body}]")
                };
                append(&mut parts, Role::Assistant, vec![Block::Text { text: text.into() }]);
            }
            InferenceItem::AssistantText { .. }
            | InferenceItem::AssistantThinking { .. }
            | InferenceItem::AssistantRedactedThinking { .. } => {}
        }
    }
    parts
}

/// `parts` as the content of one message: their text in order, each part
/// opening with its speaker, a blank line between parts and between the
/// texts of a part; an image or a document stays a block in its place.
fn spoken(parts: Vec<Message<'_>>) -> Vec<Block<'_>> {
    let mut content = Vec::new();
    let mut text = String::new();
    for part in parts {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(part.role.speaker());
        // Whether the text ends with the speaker's name, which the part's
        // first text follows on its line.
        let mut named = true;
        for block in part.content {
            let Block::Text { text: piece } = block else {
                if !text.is_empty() {
                    content.push(Block::Text {
                        text: Cow::Owned(std::mem::take(&mut text)),
                    });
                }
                content.push(block);
                named = false;
                continue;
            };
            if named {
                text.push(' ');
            } else if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&piece);
            named = false;
        }
    }
    if !text.is_empty() {
        content.push(Block::Text {
            text: Cow::Owned(text),
        });
    }
    content
}

/// Adds `blocks` to the last message when it has `role`, else as a new
/// message.
fn append<'a>(messages: &mut Vec<Message<'a>>, role: Role, blocks: Vec<Block<'a>>) {
    if blocks.is_empty() {
        return;
    }
    match messages.last_mut() {
        Some(last) if last.role == role => last.content.extend(blocks),
        _ => messages.push(Message {
            role,
            content: blocks,
        }),
    }
}

/// The user messages of `items` after the first `sent`, which a kept
/// process has not received yet: new messages and steers.
pub(crate) fn new_user_messages(items: &[InferenceItem], sent: usize) -> Vec<u8> {
    let mut lines = Vec::new();
    for content in user_messages(items).skip(sent) {
        let message = Message {
            role: Role::User,
            content: user_content(content),
        };
        lines.extend(line(&Input::User { message }));
    }
    lines
}

/// The content of every user message and steer of `items`, in order.
pub(crate) fn user_messages(items: &[InferenceItem]) -> impl Iterator<Item = &Vec<UserPart>> {
    items.iter().filter_map(|item| match item {
        InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
            Some(content)
        }
        _ => None,
    })
}

/// The name of the tool `tool_use_id` called earlier in `items`.
fn tool_name_of<'a>(items: &'a [InferenceItem], tool_use_id: &str) -> Option<&'a str> {
    items.iter().find_map(|item| match item {
        InferenceItem::ToolUse {
            tool_use_id: id,
            tool_name,
            ..
        } if id == tool_use_id => Some(tool_name.as_str()),
        _ => None,
    })
}

fn text_block(text: &str) -> Block<'_> {
    Block::Text {
        text: Cow::Borrowed(text),
    }
}

/// A user message's content as the Messages API reads it. The API has no
/// video block, and the catalog marks video unsupported, so a video that
/// reaches here anyway is named in text.
fn user_content(content: &[UserPart]) -> Vec<Block<'_>> {
    content
        .iter()
        .map(|part| match part {
            UserPart::Text(text) => text_block(text),
            UserPart::Image(medium) => Block::Image {
                source: match medium {
                    Medium::Bytes(bytes) => ImageSource::Base64 {
                        media_type: &bytes.media_type,
                        data: &bytes.data,
                    },
                    Medium::Url(url) => ImageSource::Url { url },
                },
            },
            UserPart::Video(_) => text_block("[video]"),
            UserPart::Document { bytes, file_name } => Block::Document {
                source: Base64 {
                    media_type: &bytes.media_type,
                    data: &bytes.data,
                },
                title: file_name,
            },
        })
        .collect()
}
