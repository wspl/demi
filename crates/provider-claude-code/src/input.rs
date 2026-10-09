//! What Demi writes to the CLI's standard input (`claude-code.md` §
//! Requests over stream-json): the `initialize` control request that
//! declares the SDK MCP server, the user messages a kept process gains, and
//! the answers to the CLI's control requests. Each is one JSON line. A user
//! message is the same message in the session a new process resumes.

use std::borrow::Cow;

use demi_provider_common::{InferenceItem, Medium, UserPart, json_body};
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

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    User,
}

impl<'a> Message<'a> {
    /// The user message of `content`.
    pub(crate) fn user(content: &'a [UserPart]) -> Self {
        Self {
            role: Role::User,
            content: user_content(content),
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
        data: &'a demi_shared_types::B64Bytes,
    },
    Url {
        url: &'a str,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename = "base64")]
struct Base64<'a> {
    media_type: &'a str,
    data: &'a demi_shared_types::B64Bytes,
}

/// `input` as the line Demi writes.
pub(crate) fn line(input: &Input<'_>) -> Vec<u8> {
    let mut line = json_body(input);
    line.push(b'\n');
    line
}

/// The user messages of `items` after the first `sent`, which a kept
/// process has not received yet: new messages and steers.
pub(crate) fn new_user_messages(items: &[InferenceItem], sent: usize) -> Vec<u8> {
    let mut lines = Vec::new();
    for content in user_messages(items).skip(sent) {
        let message = Message::user(content);
        lines.extend(line(&Input::User { message }));
    }
    lines
}

/// The content of every user message and steer of `items`, in order.
pub(crate) fn user_messages(items: &[InferenceItem]) -> impl Iterator<Item = &Vec<UserPart>> {
    user_message_items(items).map(|(_, content)| content)
}

/// The same, each with its index in `items`.
pub(crate) fn user_message_items(
    items: &[InferenceItem],
) -> impl Iterator<Item = (usize, &Vec<UserPart>)> {
    items.iter().enumerate().filter_map(|(index, item)| match item {
        InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
            Some((index, content))
        }
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
