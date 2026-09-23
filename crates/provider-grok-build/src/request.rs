//! What a Grok Build request sends: the Chat Completions body as the chat
//! proxy takes it, and the headers the Grok CLI identifies itself and its
//! session with (`models.md` § Request parameters).

use demi_provider::{
    InferenceRequest, UnloadedMedia, json_body,
    openai_request::{
        ChatDialect, ChatMedia, ChatMessage, ChatTool, chat_messages, reasoning_effort,
    },
};
use http::{
    HeaderMap, HeaderValue,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
};
use serde::Serialize;

use crate::auth::Credentials;

/// The Grok CLI release whose identity the provider's requests carry.
pub(crate) const CLIENT_VERSION: &str = "1.0.5";

/// The JSON body of `request`. The proxy reads images but no PDFs or
/// video, and takes no output limit or service tier.
pub(crate) fn encode(request: &InferenceRequest) -> Result<Vec<u8>, UnloadedMedia> {
    let dialect = ChatDialect {
        reasoning_content: false,
        media: ChatMedia::Images,
    };
    let body = Body {
        model: &request.model_id,
        messages: chat_messages(&request.system_prompt, &request.items, dialect)?,
        stream: true,
        stream_options: StreamOptions {
            include_usage: true,
        },
        tools: request.tools.iter().map(ChatTool::from).collect(),
        tool_choice: (!request.tools.is_empty()).then_some("auto"),
        reasoning_effort: reasoning_effort(request.thinking.as_ref()),
    };
    Ok(json_body(&body))
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
    /// The last chunk carries the request's usage.
    stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ChatTool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

/// The headers every request of the account carries: its session and the
/// identity of the Grok CLI, whose chat proxy Demi talks to.
pub(crate) fn identity_headers(credentials: &Credentials) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, credentials.access_token.bearer());
    headers.insert("x-xai-token-auth", HeaderValue::from_static("xai-grok-cli"));
    headers.insert(
        "x-authenticateresponse",
        HeaderValue::from_static("authenticate-response"),
    );
    headers.insert(
        "x-grok-client-version",
        HeaderValue::from_static(CLIENT_VERSION),
    );
    headers.insert(
        "x-grok-client-identifier",
        HeaderValue::from_static("grok-shell"),
    );
    headers.insert(
        "x-grok-client-mode",
        HeaderValue::from_static("interactive"),
    );
    // A user id or email that is not header text cannot be named in one; the
    // token still says who asks.
    if let Some(user) = credentials
        .user_id
        .as_deref()
        .and_then(|user| HeaderValue::from_str(user).ok())
    {
        headers.insert("x-userid", user.clone());
        headers.insert("x-grok-user-id", user);
    }
    if let Some(email) = credentials
        .email
        .as_deref()
        .and_then(|email| HeaderValue::from_str(email).ok())
    {
        headers.insert("x-email", email);
    }
    headers
}

/// The headers of an inference request: the identity's, the model, and the
/// session, request and turn the proxy groups the conversation by.
pub(crate) fn inference_headers(credentials: &Credentials, request: &RequestIds) -> HeaderMap {
    let mut headers = identity_headers(credentials);
    let ids = [
        ("x-grok-model-override", request.model_id.as_str()),
        ("x-grok-session-id", request.session_id.as_str()),
        ("x-grok-conv-id", request.session_id.as_str()),
        ("x-grok-req-id", request.request_id.as_str()),
        ("x-grok-turn-idx", request.turn_id.as_str()),
    ];
    for (name, value) in ids {
        // An id that is not header text is left out; the body names the
        // model, and the rest only group requests.
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(name, value);
        }
    }
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

/// The ids of a request that its headers carry.
pub(crate) struct RequestIds {
    pub(crate) model_id: String,
    pub(crate) session_id: String,
    pub(crate) request_id: String,
    pub(crate) turn_id: String,
}

impl RequestIds {
    pub(crate) fn of(request: &InferenceRequest) -> Self {
        Self {
            model_id: request.model_id.clone(),
            session_id: request.session_id.clone(),
            request_id: request.request_id.clone(),
            turn_id: request.turn_id.clone(),
        }
    }
}
