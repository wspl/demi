//! What a Codex request sends: the Responses body as the Codex backend wants
//! a transcript replayed, and the headers that name the account and the
//! session (`models.md` § Request parameters).

use std::borrow::Cow;

use bytes::Bytes;
use demi_provider::{
    InferenceRequest, UnloadedMedia, json_body,
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

/// A request's body, encoded once: over server-sent events as it is, and
/// over a WebSocket as one `response.create` message with the same fields.
#[derive(Debug, Clone)]
pub(crate) struct Encoded {
    pub(crate) http: Bytes,
    /// Present when the transport may use a WebSocket.
    pub(crate) websocket: Option<Utf8Bytes>,
}

/// The body of `request`, and its WebSocket message when `websocket`.
pub(crate) fn encode(
    request: &InferenceRequest,
    websocket: bool,
) -> Result<Encoded, UnloadedMedia> {
    let dialect = ResponsesDialect {
        signature_tag: SIGNATURE_TAG,
        assistant: AssistantReplay::Identified,
        reasoning: ReasoningReplay::Whole,
        tool_media: ToolMedia::Inline,
    };
    let body = Body {
        model: &request.model_id,
        instructions: &request.system_prompt,
        input: responses_input(&request.items, &dialect)?,
        tools: request
            .tools
            .iter()
            .map(|tool| ResponsesTool::new(tool, true))
            .collect(),
        tool_choice: "auto",
        parallel_tool_calls: true,
        store: false,
        stream: true,
        include: ["reasoning.encrypted_content"],
        prompt_cache_key: prompt_cache_key(&request.session_id),
        text: Text { verbosity: "low" },
        reasoning: responses_reasoning(request.thinking.as_ref(), SummaryOff::Auto),
        service_tier: request.service_tier_id.as_deref(),
    };
    let http = json_body(&body);
    let websocket = websocket.then(|| websocket_message(&http));
    Ok(Encoded {
        http: Bytes::from(http),
        websocket,
    })
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
    tools: Vec<ResponsesTool<'a>>,
    tool_choice: &'static str,
    parallel_tool_calls: bool,
    /// Stateless: every request carries its whole transcript.
    store: bool,
    stream: bool,
    include: [&'static str; 1],
    prompt_cache_key: Cow<'a, str>,
    text: Text,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Reasoning<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<&'a str>,
}

#[derive(Serialize)]
struct Text {
    verbosity: &'static str,
}

/// The headers that authenticate a request of the account.
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

/// The headers of an inference request: the account's, the Responses beta,
/// and the session and request the backend reads the conversation from. The
/// backend derives the cache key from `session-id` and refuses one over 64
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
