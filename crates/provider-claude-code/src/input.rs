//! What Demi writes to the CLI's standard input (`claude-code.md` §
//! Requests over stream-json): the `initialize` control request that
//! declares the SDK MCP server, the history a new process starts from, the
//! user messages a kept process gains, and the answers to the CLI's control
//! requests. Each is one JSON line.

use std::borrow::Cow;

use demi_core::{DocumentSource, MediaSource, UserContentBlock, attachment_tag};
use demi_provider::openai_request::tool_output_text;
use demi_provider::{InferenceItem, UnloadedMedia, json_body};
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
    Assistant {
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

/// The history a new process starts from, as its first messages. Earlier
/// tool calls and their results become assistant text, since replaying a
/// structured tool call into a new SDK MCP session makes the vendor refuse
/// the request; a user message holds only what the user sent; earlier
/// reasoning is left out, since its signatures do not hold for a new
/// process. Consecutive items of one role are one message.
pub(crate) fn history(items: &[InferenceItem]) -> Result<Vec<u8>, UnloadedMedia> {
    let mut messages: Vec<Message<'_>> = Vec::new();
    for item in items {
        match item {
            InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
                append(&mut messages, Role::User, user_content(content)?);
            }
            InferenceItem::AssistantText { text, .. } if !text.is_empty() => {
                append(&mut messages, Role::Assistant, vec![text_block(text)]);
            }
            InferenceItem::ToolUse {
                tool_name, input, ..
            } => {
                let text = format!(
                    "[Earlier in this conversation I called the tool {tool_name} with input: {input}."
                );
                append(
                    &mut messages,
                    Role::Assistant,
                    vec![Block::Text { text: text.into() }],
                );
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
                append(
                    &mut messages,
                    Role::Assistant,
                    vec![Block::Text { text: text.into() }],
                );
            }
            InferenceItem::AssistantText { .. }
            | InferenceItem::AssistantThinking { .. }
            | InferenceItem::AssistantRedactedThinking { .. } => {}
        }
    }
    let mut lines = Vec::new();
    for message in messages {
        let input = match message.role {
            Role::User => Input::User { message },
            Role::Assistant => Input::Assistant { message },
        };
        lines.extend(line(&input));
    }
    Ok(lines)
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
pub(crate) fn new_user_messages(
    items: &[InferenceItem],
    sent: usize,
) -> Result<Vec<u8>, UnloadedMedia> {
    let mut lines = Vec::new();
    for content in user_messages(items).skip(sent) {
        let message = Message {
            role: Role::User,
            content: user_content(content)?,
        };
        lines.extend(line(&Input::User { message }));
    }
    Ok(lines)
}

/// The content of every user message and steer of `items`, in order.
pub(crate) fn user_messages(
    items: &[InferenceItem],
) -> impl Iterator<Item = &Vec<UserContentBlock>> {
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
fn user_content(content: &[UserContentBlock]) -> Result<Vec<Block<'_>>, UnloadedMedia> {
    content
        .iter()
        .map(|block| {
            Ok(match block {
                UserContentBlock::Text { text } => text_block(text),
                UserContentBlock::Image { source } => Block::Image {
                    source: match source {
                        MediaSource::Binary { data, media_type } => {
                            ImageSource::Base64 { media_type, data }
                        }
                        MediaSource::Url { url } => ImageSource::Url { url },
                        MediaSource::Ref { r#ref, .. } => {
                            return Err(UnloadedMedia(r#ref.to_string()));
                        }
                    },
                },
                UserContentBlock::Video { .. } => text_block("[video]"),
                UserContentBlock::Document { source } => match source {
                    DocumentSource::Binary {
                        data,
                        media_type,
                        file_name,
                    } => Block::Document {
                        source: Base64 { media_type, data },
                        title: file_name,
                    },
                    DocumentSource::Ref { r#ref, .. } => {
                        return Err(UnloadedMedia(r#ref.to_string()));
                    }
                },
                UserContentBlock::Attachment(attachment) => Block::Text {
                    text: attachment_tag(attachment).into(),
                },
                UserContentBlock::Reference { reference } => text_block(reference),
            })
        })
        .collect()
}
