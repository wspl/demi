//! The development backend's model (`backend.md` § One-command development
//! backend): an Anthropic-compatible Messages endpoint on the loopback
//! interface that answers every request with the text of its last user
//! message, as the vendor's event stream. The backend's Anthropic API
//! provider calls it as it calls the vendor, so no request reaches a model.

use std::net::Ipv4Addr;

use axum::Json;
use axum::Router;
use axum::response::sse::{Event, Sse};
use axum::routing::post;
use futures_util::Stream;
use serde::Deserialize;
use serde_json::json;
use tokio_util::task::AbortOnDropHandle;

/// The endpoint, serving while it lives.
pub struct Echo {
    /// The base URL an entry names: the provider appends `/messages`.
    pub url: String,
    /// The port it listens on.
    pub port: u16,
    _server: AbortOnDropHandle<()>,
}

/// The variable that turns the echo model on: `1`, or unset for off.
const ECHO: &str = "DEMI_DEV_ECHO";

/// Whether `var` turns the echo model on, or what is wrong with its value.
pub fn enabled(var: impl Fn(&str) -> Option<String>) -> Result<bool, String> {
    match var(ECHO).as_deref() {
        None | Some("") => Ok(false),
        Some("1") => Ok(true),
        Some(other) => Err(format!("{ECHO} must be 1 or unset, not {other:?}")),
    }
}

/// Starts the endpoint on `port` of the loopback interface, or on a free
/// one for 0.
pub async fn start(port: u16) -> std::io::Result<Echo> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let address = listener.local_addr()?;
    let url = format!("http://{address}/v1");
    let app = Router::new().route("/v1/messages", post(messages));
    let server = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            eprintln!("xtask dev: the echo model stopped: {error}");
        }
    });
    Ok(Echo {
        url,
        port: address.port(),
        _server: AbortOnDropHandle::new(server),
    })
}

/// The parts of a Messages API request the endpoint reads; serde skips the
/// rest, which the backend's provider writes.
#[derive(Deserialize)]
struct MessagesRequest {
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct Message {
    role: Role,
    content: Content,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Role {
    User,
    Assistant,
}

/// A message's content: a string, or blocks.
#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Blocks(Vec<Block>),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    Text {
        text: String,
    },
    /// An image, a document, a tool result or another block without text.
    #[serde(other)]
    Other,
}

impl MessagesRequest {
    /// The text blocks of the last user message, one per line.
    fn last_user_text(&self) -> String {
        let last = self
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::User);
        let Some(message) = last else {
            return String::new();
        };
        match &message.content {
            Content::Text(text) => text.clone(),
            Content::Blocks(blocks) => {
                let texts: Vec<&str> = blocks
                    .iter()
                    .filter_map(|block| match block {
                        Block::Text { text } => Some(text.as_str()),
                        Block::Other => None,
                    })
                    .collect();
                texts.join("\n")
            }
        }
    }
}

/// Answers `request` with `Echo: <its last user message>`, one text delta
/// per word, so the page shows the answer stream in.
async fn messages(
    Json(request): Json<MessagesRequest>,
) -> Sse<impl Stream<Item = Result<Event, axum::Error>>> {
    let reply = format!("Echo: {}", request.last_user_text());
    let words = reply.split_inclusive(' ').count();
    let mut events = vec![
        (
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_echo",
                    "type": "message",
                    "role": "assistant",
                    "model": "echo",
                    "content": [],
                    "usage": { "input_tokens": 1, "output_tokens": 1 },
                },
            }),
        ),
        (
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": { "type": "text", "text": "" },
            }),
        ),
    ];
    for word in reply.split_inclusive(' ') {
        events.push((
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": word },
            }),
        ));
    }
    events.push((
        "content_block_stop",
        json!({ "type": "content_block_stop", "index": 0 }),
    ));
    events.push((
        "message_delta",
        json!({
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn", "stop_sequence": null },
            "usage": { "output_tokens": words },
        }),
    ));
    events.push(("message_stop", json!({ "type": "message_stop" })));
    let events = events
        .into_iter()
        .map(|(name, data)| Event::default().event(name).json_data(data));
    Sse::new(futures_util::stream::iter(events))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_echo_model_is_off_unless_the_variable_is_1() {
        let read = |value: Option<&str>| enabled(|_| value.map(str::to_owned));
        assert_eq!(read(None), Ok(false));
        assert_eq!(read(Some("")), Ok(false));
        assert_eq!(read(Some("1")), Ok(true));
        assert!(read(Some("true")).is_err());
    }
}
