//! The request bodies of the two wires (`models.md` § Request parameters):
//! the Responses API's, and Chat Completions' for OpenAI-compatible
//! endpoints, each with the vendor policy the entry names.

use std::borrow::Cow;

use demi_core::is_blank;
use demi_provider::{
    InferenceRequest, UnloadedMedia, json_body,
    openai_request::{
        AssistantReplay, ChatDialect, ChatMedia, ChatMessage, ChatTool, InputItem, Reasoning,
        ReasoningReplay, ResponsesDialect, ResponsesTool, SummaryOff, ToolMedia, chat_messages,
        prompt_cache_key, reasoning_effort, responses_input, responses_reasoning,
    },
};
use serde::Serialize;

use crate::{SIGNATURE_TAG, VendorPolicy};

/// The JSON body of a Responses request.
pub(crate) fn responses(
    request: &InferenceRequest,
    policy: VendorPolicy,
) -> Result<Vec<u8>, UnloadedMedia> {
    let dialect = ResponsesDialect {
        signature_tag: SIGNATURE_TAG,
        assistant: if policy.replay_assistant_status {
            AssistantReplay::Completed
        } else {
            AssistantReplay::Minimal
        },
        reasoning: ReasoningReplay::Replayable,
        tool_media: ToolMedia::FollowUp,
    };
    let has_tools = !request.tools.is_empty();
    let body = ResponsesBody {
        model: &request.model_id,
        input: responses_input(&request.items, &dialect)?,
        stream: true,
        store: false,
        include: ["reasoning.encrypted_content"],
        prompt_cache_key: prompt_cache_key(&request.session_id),
        max_output_tokens: request.output_limit.map(|limit| limit.get()),
        instructions: (!is_blank(&request.system_prompt)).then_some(request.system_prompt.as_str()),
        tools: request
            .tools
            .iter()
            .map(|tool| ResponsesTool::new(tool, false))
            .collect(),
        tool_choice: has_tools.then_some("auto"),
        parallel_tool_calls: has_tools.then_some(true),
        reasoning: responses_reasoning(request.thinking.as_ref(), SummaryOff::Omitted),
        service_tier: request.service_tier_id.as_deref(),
    };
    Ok(json_body(&body))
}

#[derive(Serialize)]
struct ResponsesBody<'a> {
    model: &'a str,
    input: Vec<InputItem<'a>>,
    stream: bool,
    /// Stateless: every request carries its whole transcript.
    store: bool,
    include: [&'static str; 1],
    prompt_cache_key: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ResponsesTool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Reasoning<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<&'a str>,
}

/// The JSON body of a Chat Completions request.
pub(crate) fn chat_completions(
    request: &InferenceRequest,
    policy: VendorPolicy,
) -> Result<Vec<u8>, UnloadedMedia> {
    let dialect = ChatDialect {
        reasoning_content: policy.pass_back_reasoning_content,
        media: ChatMedia::Native,
    };
    let body = ChatBody {
        model: &request.model_id,
        messages: chat_messages(&request.system_prompt, &request.items, dialect)?,
        stream: true,
        tools: request.tools.iter().map(ChatTool::from).collect(),
        tool_choice: (!request.tools.is_empty()).then_some("auto"),
        max_completion_tokens: request.output_limit.map(|limit| limit.get()),
        reasoning_effort: reasoning_effort(request.thinking.as_ref()),
        service_tier: request.service_tier_id.as_deref(),
        stream_options: StreamOptions {
            include_usage: true,
        },
    };
    Ok(json_body(&body))
}

#[derive(Serialize)]
struct ChatBody<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ChatTool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<&'a str>,
    /// The last chunk carries the request's usage.
    stream_options: StreamOptions,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}
