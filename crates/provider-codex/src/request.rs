//! What a Codex request sends: the Responses body as the Codex backend wants
//! a transcript replayed, and the headers that name the account and the
//! session (`crates/shared-types/src/model.rs`).

use std::borrow::Cow;

use bytes::Bytes;
use demi_provider_common::{
    InferenceRequest, Searches, json_body,
    openai_request::{
        AssistantReplay, InputItem, Reasoning, ReasoningReplay, ResponsesDialect, ResponsesTool,
        SummaryOff, ToolMedia, prompt_cache_key, responses_input, responses_reasoning,
    },
};
use http::{
    HeaderMap, HeaderValue,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT},
};
use serde::Serialize;
use tokio_tungstenite::tungstenite::Utf8Bytes;

use crate::{SIGNATURE_TAG, auth::Credentials};

/// A request's body, encoded once: over server-sent events as it is, and over a
/// WebSocket as one `response.create` message with the same fields, in the same
/// bytes.
///
/// So a request extends the previous one for the vendor's cache whichever way
/// each went.
#[derive(Debug, Clone)]
pub(crate) struct Encoded {
    pub(crate) http: Bytes,
    /// Present when the transport may use a WebSocket.
    pub(crate) websocket: Option<Utf8Bytes>,
}

/// The body of `request`, and its WebSocket message when `websocket`.
/// OpenAI's web search goes with every request while the entry turns it on,
/// beside Demi's tools, live, as the Codex CLI's `--search` sends it, and with
/// the sources of each search (`crates/provider-common/src/wire/responses.rs`).
pub(crate) fn encode(request: &InferenceRequest, websocket: bool, searches: Searches) -> Encoded {
    let dialect = ResponsesDialect {
        signature_tag: SIGNATURE_TAG,
        assistant: AssistantReplay::Identified,
        reasoning: ReasoningReplay::Whole,
        tool_media: ToolMedia::Inline,
        searches: Some(searches),
    };
    let body = Body {
        model: &request.model_id,
        instructions: &request.system_prompt,
        input: responses_input(&request.items, &dialect),
        tools: request
            .tools
            .iter()
            .map(|tool| Tool::Function(ResponsesTool::new(tool, true)))
            .chain(searches.web.then_some(Tool::Hosted(HostedTool::WebSearch {
                external_web_access: true,
            })))
            .collect(),
        tool_choice: "auto",
        parallel_tool_calls: true,
        store: false,
        stream: true,
        include: if searches.web { &INCLUDE_SEARCH } else { &INCLUDE },
        prompt_cache_key: prompt_cache_key(request.cache_session()),
        text: Text { verbosity: "low" },
        reasoning: responses_reasoning(request.thinking.as_ref(), SummaryOff::Auto),
        service_tier: request.service_tier_id.as_deref(),
    };
    let http = json_body(&body);
    let websocket = websocket.then(|| websocket_message(&http));
    Encoded {
        http: Bytes::from(http),
        websocket,
    }
}

/// The body as the WebSocket's `response.create` message: the type, then
/// the body's own fields.
fn websocket_message(body: &[u8]) -> Utf8Bytes {
    // The body is a JSON object with at least its model, so it opens with a
    // brace followed by its first field, and JSON text is UTF-8.
    let fields = std::str::from_utf8(&body[1..]).expect("a JSON body is UTF-8");
    let mut message = String::with_capacity(body.len() + 26);
    message.push_str("{\"type\":\"response.create\",");
    message.push_str(fields);
    Utf8Bytes::from(message)
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    /// Sent even when empty: the backend requires the field.
    instructions: &'a str,
    input: Vec<InputItem<'a>>,
    tools: Vec<Tool<'a>>,
    tool_choice: &'static str,
    parallel_tool_calls: bool,
    /// Stateless: every request carries its whole transcript.
    store: bool,
    stream: bool,
    include: &'static [&'static str],
    prompt_cache_key: Cow<'a, str>,
    text: Text,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Reasoning<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<&'a str>,
}

/// What a response includes: its reasoning's ciphertext, and each web search's
/// sources while the entry searches.
const INCLUDE: [&str; 1] = ["reasoning.encrypted_content"];
const INCLUDE_SEARCH: [&str; 2] = ["reasoning.encrypted_content", "web_search_call.action.sources"];

/// A tool of a request: one of Demi's, or a search OpenAI runs.
#[derive(Serialize)]
#[serde(untagged)]
enum Tool<'a> {
    Function(ResponsesTool<'a>),
    Hosted(HostedTool),
}

/// A search OpenAI runs on its side.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HostedTool {
    WebSearch { external_web_access: bool },
}

#[derive(Serialize)]
struct Text {
    verbosity: &'static str,
}

pub(crate) fn account_headers(credentials: &Credentials, user_agent: &HeaderValue) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, credentials.access_token.bearer());
    headers.insert("chatgpt-account-id", credentials.account_id.header_value());
    if credentials.fedramp {
        headers.insert("x-openai-fedramp", HeaderValue::from_static("true"));
    }
    headers.insert(USER_AGENT, user_agent.clone());
    headers
}

/// The headers of an inference request: the account's, the Responses beta, and
/// the session and request the backend reads the conversation from.
///
/// The backend derives the cache key from `session-id` and refuses one over 64
/// characters, so the session id is clamped as the body's key is.
pub(crate) fn inference_headers(
    credentials: &Credentials,
    user_agent: &HeaderValue,
    session_id: &str,
    request_id: &str,
) -> HeaderMap {
    let mut headers = account_headers(credentials, user_agent);
    headers.insert(
        "openai-beta",
        HeaderValue::from_static("responses=experimental"),
    );
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    // An id that is not header text cannot name the session in a header; the
    // body's cache key still does.
    if let Ok(session) = HeaderValue::from_str(&prompt_cache_key(session_id)) {
        headers.insert("session-id", session.clone());
        headers.insert("thread-id", session);
    }
    if let Ok(request) = HeaderValue::from_str(request_id) {
        headers.insert("x-client-request-id", request);
    }
    headers
}
