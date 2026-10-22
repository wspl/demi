//! The requests a run sends over each wire (`crates/shared-types/src/model.rs`,
//! `crates/provider-common/src/endpoint.rs`,
//! `crates/backend-providers/src/llm/vendors.rs`).

use std::sync::Arc;

use demi_provider_common::VendorPolicy;
use demi_provider_common::{
    InferenceItem, InferenceRequest, MediaBytes, ResultPart, ToolDefinition, UserPart,
    testing::inference_request,
};
use demi_shared_types::{B64Bytes, ThinkingConfig, ThinkingSummary, WireApi};
use serde_json::json;

use crate::body_of;

fn text(text: &str) -> Vec<UserPart> {
    vec![UserPart::Text(text.into())]
}

fn read_file_tool() -> ToolDefinition {
    let schema = json!({ "type": "object", "properties": { "path": { "type": "string" } } });
    ToolDefinition {
        name: "read_file".into(),
        description: "Read a file".into(),
        input_schema: schema.as_object().unwrap().clone(),
    }
}

fn tool_use(id: &str, path: &str) -> InferenceItem {
    InferenceItem::ToolUse {
        model_id: "gpt-test".into(),
        tool_use_id: id.into(),
        tool_name: "read_file".into(),
        input: json!({ "path": path }),
    }
}

fn tool_result(id: &str, output: Vec<ResultPart>) -> InferenceItem {
    InferenceItem::ToolResult {
        tool_use_id: id.into(),
        output,
        is_error: false,
    }
}

fn contents(text: &str) -> Vec<ResultPart> {
    vec![ResultPart::Text(text.into())]
}

fn thinking(text: &str, signature: Option<&str>) -> InferenceItem {
    InferenceItem::AssistantThinking {
        model_id: "gpt-test".into(),
        text: text.into(),
        signature: signature.map(str::to_owned),
        kept_past_summary: false,
    }
}

fn assistant(text: &str) -> InferenceItem {
    InferenceItem::AssistantText {
        model_id: "gpt-test".into(),
        text: text.into(),
    }
}

fn request_with(items: Vec<InferenceItem>) -> InferenceRequest {
    let mut request = inference_request();
    request.model_id = "gpt-test".into();
    request.items = items.into();
    request
}

fn effort(effort: &str, summary: Option<ThinkingSummary>) -> Option<ThinkingConfig> {
    Some(ThinkingConfig::Effort {
        effort: effort.into(),
        summary,
    })
}

const NO_POLICY: VendorPolicy = VendorPolicy {
    pass_back_reasoning_content: false,
    replay_assistant_status: false,
    effort_as_budget: false,
};

#[tokio::test]
async fn a_responses_body_carries_the_transcript_tools_tier_and_reasoning() {
    let mut request = request_with(vec![
        InferenceItem::UserMessage {
            content: text("hello"),
        },
        assistant("Use tool"),
        tool_use("call-1|fc-1", "a.ts"),
        tool_result("call-1|fc-1", contents("contents")),
    ]);
    request.system_prompt = "system instructions".into();
    request.service_tier_id = Some("priority".into());
    request.thinking = effort("high", None);
    request.tools = Arc::new([read_file_tool()]);
    let body = body_of(WireApi::Responses, NO_POLICY, request).await;
    assert_eq!(
        body,
        json!({
            "model": "gpt-test",
            "input": [
                { "role": "user", "content": [{ "type": "input_text", "text": "hello" }] },
                // The minimal item: strict gateways refuse an id or status.
                { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "Use tool", "annotations": [] }] },
                { "type": "function_call", "id": "fc-1", "call_id": "call-1", "name": "read_file", "arguments": "{\"path\":\"a.ts\"}" },
                { "type": "function_call_output", "call_id": "call-1", "output": "contents" },
            ],
            "stream": true,
            "store": false,
            "include": ["reasoning.encrypted_content"],
            "prompt_cache_key": "session-1",
            "instructions": "system instructions",
            "tools": [{ "type": "function", "name": "read_file", "description": "Read a file", "parameters": { "type": "object", "properties": { "path": { "type": "string" } } } }],
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "reasoning": { "effort": "high", "summary": "auto" },
            "service_tier": "priority",
        })
    );
}

#[tokio::test]
async fn reasoning_follows_the_thinking_setting_and_a_summary_turned_off_leaves_the_field_out() {
    let cases = [
        (
            effort("high", Some(ThinkingSummary::Off)),
            json!({ "effort": "high" }),
        ),
        (
            effort("high", None),
            json!({ "effort": "high", "summary": "auto" }),
        ),
        (
            effort("low", Some(ThinkingSummary::Concise)),
            json!({ "effort": "low", "summary": "concise" }),
        ),
        (effort("none", None), json!({ "effort": "none" })),
        (
            Some(ThinkingConfig::Adaptive {
                effort: "medium".into(),
            }),
            json!({ "effort": "medium", "summary": "auto" }),
        ),
    ];
    for (thinking, reasoning) in cases {
        let mut request = inference_request();
        request.thinking = thinking.clone();
        let body = body_of(WireApi::Responses, NO_POLICY, request).await;
        assert_eq!(body["reasoning"], reasoning, "{thinking:?}");
    }
    for thinking in [
        Some(ThinkingConfig::Budget {
            budget_tokens: 2048,
        }),
        Some(ThinkingConfig::Disabled {}),
    ] {
        let mut request = inference_request();
        request.thinking = thinking;
        assert!(
            body_of(WireApi::Responses, NO_POLICY, request.clone())
                .await
                .get("reasoning")
                .is_none()
        );
        assert!(
            body_of(WireApi::ChatCompletions, NO_POLICY, request)
                .await
                .get("reasoning_effort")
                .is_none()
        );
    }
}

#[tokio::test]
async fn only_this_providers_reasoning_items_are_replayed_with_their_replayable_fields() {
    let item = json!({ "type": "reasoning", "id": "rs_1", "summary": [{ "type": "summary_text", "text": "sum" }], "encrypted_content": "enc", "status": "completed" });
    let body = body_of(
        WireApi::Responses,
        NO_POLICY,
        request_with(vec![
            InferenceItem::UserMessage {
                content: text("hello"),
            },
            thinking("mine", Some(&format!("openai:{item}"))),
            // Another vendor's signature, an untagged one and unsigned
            // thinking are not this vendor's to replay.
            thinking("codex", Some(&format!("codex:{item}"))),
            thinking("untagged", Some(&item.to_string())),
            thinking("unsigned", None),
            thinking("not an item", Some("openai:{\"type\":\"message\"}")),
        ]),
    )
    .await;
    assert_eq!(
        body["input"],
        json!([
            { "role": "user", "content": [{ "type": "input_text", "text": "hello" }] },
            { "type": "reasoning", "id": "rs_1", "summary": [{ "type": "summary_text", "text": "sum" }], "encrypted_content": "enc" },
        ])
    );
}

#[tokio::test]
async fn media_a_tool_returned_follows_its_output_in_a_user_message() {
    let output = vec![
        ResultPart::Text("<binary stdout: 75 bytes>".into()),
        ResultPart::Image(MediaBytes {
            data: B64Bytes::from(&b"AAAA"[..]),
            media_type: "image/png".into(),
        }),
        ResultPart::Video(MediaBytes {
            data: B64Bytes::from(&b"MP4"[..]),
            media_type: "video/mp4".into(),
        }),
        ResultPart::Document {
            bytes: MediaBytes {
                data: B64Bytes::from(&b"%PDF"[..]),
                media_type: "application/pdf".into(),
            },
            file_name: "document-3.pdf".into(),
        },
    ];
    let responses = body_of(
        WireApi::Responses,
        NO_POLICY,
        request_with(vec![
            tool_use("call-1|fc-1", "shot.png"),
            tool_result("call-1|fc-1", output.clone()),
        ]),
    )
    .await;
    assert_eq!(
        responses["input"].as_array().unwrap()[1..],
        [
            json!({ "type": "function_call_output", "call_id": "call-1", "output": "<binary stdout: 75 bytes>\n[image:image/png]\n[video:video/mp4]\n[document:application/pdf]" }),
            json!({ "role": "user", "content": [
                { "type": "input_text", "text": "[media returned by tool call call-1]" },
                { "type": "input_image", "image_url": "data:image/png;base64,QUFBQQ==", "detail": "auto" },
                { "type": "input_image", "image_url": "data:video/mp4;base64,TVA0", "detail": "auto" },
                { "type": "input_file", "filename": "document-3.pdf", "file_data": "data:application/pdf;base64,JVBERg==" },
            ] }),
        ]
    );
    // A tool message is text only, so the media rides after it here too.
    let chat = body_of(
        WireApi::ChatCompletions,
        NO_POLICY,
        request_with(vec![
            tool_use("call-1", "shot.png"),
            tool_result("call-1", output),
        ]),
    )
    .await;
    assert_eq!(
        chat["messages"][1],
        json!({ "role": "tool", "tool_call_id": "call-1", "content": "<binary stdout: 75 bytes>\n[image:image/png]\n[video:video/mp4]\n[document:application/pdf]" })
    );
    assert_eq!(
        chat["messages"][2],
        json!({ "role": "user", "content": [
            { "type": "text", "text": "[media returned by tool call call-1]" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,QUFBQQ==", "detail": "auto" } },
            { "type": "image_url", "image_url": { "url": "data:video/mp4;base64,TVA0", "detail": "auto" } },
            { "type": "file", "file": { "filename": "document-3.pdf", "file_data": "data:application/pdf;base64,JVBERg==" } },
        ] })
    );
}

#[tokio::test]
async fn a_chat_completions_body_carries_the_transcript_tools_tier_and_effort() {
    let mut request = request_with(vec![
        InferenceItem::UserMessage {
            content: text("hello"),
        },
        assistant("Use tool"),
        tool_use("call-1", "a.ts"),
        tool_result("call-1", contents("contents")),
    ]);
    request.system_prompt = "system instructions".into();
    request.service_tier_id = Some("priority".into());
    request.thinking = effort("high", None);
    request.tools = Arc::new([read_file_tool()]);
    let body = body_of(WireApi::ChatCompletions, NO_POLICY, request).await;
    assert_eq!(
        body,
        json!({
            "model": "gpt-test",
            "messages": [
                { "role": "system", "content": "system instructions" },
                { "role": "user", "content": "hello" },
                { "role": "assistant", "content": "Use tool", "tool_calls": [{ "id": "call-1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"a.ts\"}" } }] },
                { "role": "tool", "tool_call_id": "call-1", "content": "contents" },
            ],
            "stream": true,
            "tools": [{ "type": "function", "function": { "name": "read_file", "description": "Read a file", "parameters": { "type": "object", "properties": { "path": { "type": "string" } } } } }],
            "tool_choice": "auto",
            "reasoning_effort": "high",
            "service_tier": "priority",
            "stream_options": { "include_usage": true },
        })
    );
}
