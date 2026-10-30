//! What a Grok Build request sends: the Responses body as the chat proxy takes
//! it, and the headers the Grok CLI identifies itself and its session with
//! (`crates/shared-types/src/model.rs`).

use demi_provider_common::{
    InferenceRequest, Searches, json_body,
    openai_request::{
        AssistantReplay, InputItem, Reasoning, ReasoningReplay, ResponsesDialect, ResponsesTool,
        SummaryOff, ToolMedia, responses_input, responses_reasoning,
    },
};
use demi_shared_types::is_blank;
use http::{
    HeaderMap, HeaderValue,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
};
use serde::Serialize;

use crate::{SIGNATURE_TAG, auth::Credentials, releases::GrokReleases};

/// The JSON body of `request`. The proxy reads images, in a message and in a
/// tool's output, but no PDFs or video, and takes no output limit or service
/// tier.
/// xAI's own searches go with every request while the entry turns them on,
/// beside Demi's tools, and run at xAI (`crates/provider-common/src/wire/responses.rs`).
pub(crate) fn encode(request: &InferenceRequest, searches: Searches) -> Vec<u8> {
    let dialect = ResponsesDialect {
        signature_tag: SIGNATURE_TAG,
        assistant: AssistantReplay::Minimal,
        reasoning: ReasoningReplay::Replayable,
        tool_media: ToolMedia::Inline,
        searches: Some(searches),
    };
    let hosted = HostedTool::of(searches);
    let has_tools = !request.tools.is_empty() || !hosted.is_empty();
    let body = Body {
        model: &request.model_id,
        input: responses_input(&request.items, &dialect),
        stream: true,
        store: false,
        include: ["reasoning.encrypted_content"],
        instructions: (!is_blank(&request.system_prompt)).then_some(request.system_prompt.as_str()),
        tools: request
            .tools
            .iter()
            .map(|tool| Tool::Function(ResponsesTool::new(tool, false)))
            .chain(hosted.into_iter().map(Tool::Hosted))
            .collect(),
        tool_choice: has_tools.then_some("auto"),
        reasoning: responses_reasoning(request.thinking.as_ref(), SummaryOff::Omitted),
    };
    json_body(&body)
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    input: Vec<InputItem<'a>>,
    stream: bool,
    /// Stateless: every request carries its whole transcript.
    store: bool,
    include: [&'static str; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Tool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Reasoning<'a>>,
}

/// A tool of a request: one of Demi's, or a search xAI runs.
#[derive(Serialize)]
#[serde(untagged)]
enum Tool<'a> {
    Function(ResponsesTool<'a>),
    Hosted(HostedTool),
}

/// A search xAI runs on its side, looking at the images and watching the
/// videos it finds when the entry lets it.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HostedTool {
    WebSearch {
        enable_image_understanding: bool,
    },
    XSearch {
        enable_image_understanding: bool,
        enable_video_understanding: bool,
    },
}

impl HostedTool {
    /// The searches `searches` turns on.
    fn of(searches: Searches) -> Vec<Self> {
        let web = searches.web.then_some(Self::WebSearch {
            enable_image_understanding: searches.images,
        });
        let x = searches.x.then_some(Self::XSearch {
            enable_image_understanding: searches.images,
            enable_video_understanding: searches.videos,
        });
        web.into_iter().chain(x).collect()
    }
}

/// The headers every request of the account carries: its session and the
/// identity of the Grok CLI, whose chat proxy Demi talks to, in the newest
/// release `releases` knows.
pub(crate) fn identity_headers(credentials: &Credentials, releases: &GrokReleases) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, credentials.access_token.bearer());
    headers.insert("x-xai-token-auth", HeaderValue::from_static("xai-grok-cli"));
    headers.insert(
        "x-authenticateresponse",
        HeaderValue::from_static("authenticate-response"),
    );
    headers.insert("x-grok-client-version", releases.current());
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
pub(crate) fn inference_headers(
    credentials: &Credentials,
    releases: &GrokReleases,
    request: &RequestIds,
) -> HeaderMap {
    let mut headers = identity_headers(credentials, releases);
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
            session_id: request.cache_session().to_owned(),
            request_id: request.request_id.clone(),
            turn_id: request.turn_id.clone(),
        }
    }
}
