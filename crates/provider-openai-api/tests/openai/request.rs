//! The requests a run sends over each wire (`models.md` § Request
//! parameters, `providers.md` § Endpoints, § Vendors from models.dev).

use std::{num::NonZeroU32, sync::Arc};

use demi_core::{
    B64Bytes, DocumentSource, MediaSource, ThinkingConfig, ThinkingSummary, TokenUsage,
    ToolMediaSource, ToolResultContentBlock, UserContentBlock,
};
use demi_provider::{
    InferenceItem, InferenceRequest, Provider, ProviderEvent, RuntimeEnv, ToolDefinition,
    testing::{MockVendor, inference_request},
};
use demi_provider_openai_api::{VendorPolicy, WireApi};
use serde_json::{Value, json};

use crate::{body_of, done, provider_at, run};

fn text(text: &str) -> Vec<UserContentBlock> {
    vec![UserContentBlock::Text { text: text.into() }]
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

fn tool_result(id: &str, output: Vec<ToolResultContentBlock>) -> InferenceItem {
    InferenceItem::ToolResult {
        tool_use_id: id.into(),
        output,
        is_error: false,
    }
}

fn contents(text: &str) -> Vec<ToolResultContentBlock> {
    vec![ToolResultContentBlock::Text { text: text.into() }]
}

fn thinking(text: &str, signature: Option<&str>) -> InferenceItem {
    InferenceItem::AssistantThinking {
        model_id: "gpt-test".into(),
        text: text.into(),
        signature: signature.map(str::to_owned),
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
};

#[tokio::test]
async fn a_run_posts_to_its_wires_endpoint_with_the_key() {
    let vendor = MockVendor::start().await;
    let cases = [
        (WireApi::Responses, "/v1/", "/v1/responses"),
        (WireApi::Responses, "/gateway/v1/responses", "/gateway/v1/responses"),
        (WireApi::ChatCompletions, "/openai/v1", "/openai/v1/chat/completions"),
        (WireApi::ChatCompletions, "/v1/chat/completions/", "/v1/chat/completions"),
    ];
    for (wire, base, _) in cases {
        vendor.respond(done());
        let mut runtime = provider_at(&vendor, base, wire, NO_POLICY)
            .runtime(RuntimeEnv { http: reqwest::Client::new() })
            .unwrap();
        let events = run(runtime.as_mut(), inference_request()).await;
        assert_eq!(events, [ProviderEvent::Response(TokenUsage::default())]);
    }
    for ((_, _, path), request) in cases.iter().zip(vendor.requests()) {
        assert_eq!((request.method.as_str(), request.uri.path()), ("POST", *path));
        assert_eq!(request.header("authorization"), Some("Bearer sk-test"));
        assert_eq!(request.header("content-type"), Some("application/json"));
        assert_eq!(request.header("accept"), Some("text/event-stream"));
    }
}

#[tokio::test]
async fn each_request_sends_its_own_output_limit_and_none_without_one() {
    let with_limit = |limit: Option<u32>| {
        let mut request = inference_request();
        request.output_limit = limit.and_then(NonZeroU32::new);
        request
    };
    let responses = body_of(WireApi::Responses, NO_POLICY, with_limit(Some(8_000))).await;
    assert_eq!(responses["max_output_tokens"], json!(8_000));
    let chat = body_of(WireApi::ChatCompletions, NO_POLICY, with_limit(Some(32_000))).await;
    assert_eq!(chat["max_completion_tokens"], json!(32_000));
    let responses = body_of(WireApi::Responses, NO_POLICY, with_limit(None)).await;
    assert!(responses.get("max_output_tokens").is_none());
    let chat = body_of(WireApi::ChatCompletions, NO_POLICY, with_limit(None)).await;
    assert!(chat.get("max_completion_tokens").is_none());
}

#[tokio::test]
async fn a_responses_body_carries_the_transcript_tools_tier_and_reasoning() {
    let mut request = request_with(vec![
        InferenceItem::UserMessage { content: text("hello") },
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
async fn a_blank_system_prompt_no_tools_and_no_tier_are_left_out() {
    let mut request = inference_request();
    request.system_prompt = " \n".into();
    let body = body_of(WireApi::Responses, NO_POLICY, request).await;
    for field in ["instructions", "tools", "tool_choice", "parallel_tool_calls", "reasoning", "service_tier", "stream_options"] {
        assert!(body.get(field).is_none(), "{field}");
    }
    let chat = body_of(WireApi::ChatCompletions, NO_POLICY, inference_request()).await;
    assert_eq!(chat["messages"], json!([{ "role": "user", "content": "hello" }]));
    for field in ["tools", "tool_choice", "reasoning_effort", "service_tier"] {
        assert!(chat.get(field).is_none(), "{field}");
    }
}

#[tokio::test]
async fn reasoning_follows_the_thinking_setting_and_a_summary_turned_off_leaves_the_field_out() {
    let cases = [
        (effort("high", Some(ThinkingSummary::Off)), json!({ "effort": "high" })),
        (effort("high", None), json!({ "effort": "high", "summary": "auto" })),
        (effort("low", Some(ThinkingSummary::Concise)), json!({ "effort": "low", "summary": "concise" })),
        (effort("none", None), json!({ "effort": "none" })),
        (Some(ThinkingConfig::Adaptive { effort: "medium".into() }), json!({ "effort": "medium", "summary": "auto" })),
    ];
    for (thinking, reasoning) in cases {
        let mut request = inference_request();
        request.thinking = thinking.clone();
        let body = body_of(WireApi::Responses, NO_POLICY, request).await;
        assert_eq!(body["reasoning"], reasoning, "{thinking:?}");
    }
    for thinking in [Some(ThinkingConfig::Budget { budget_tokens: 2048 }), Some(ThinkingConfig::Disabled {})] {
        let mut request = inference_request();
        request.thinking = thinking;
        assert!(body_of(WireApi::Responses, NO_POLICY, request.clone()).await.get("reasoning").is_none());
        assert!(body_of(WireApi::ChatCompletions, NO_POLICY, request).await.get("reasoning_effort").is_none());
    }
}

#[tokio::test]
async fn assistant_status_is_replayed_only_when_the_vendor_policy_asks() {
    let items = vec![InferenceItem::UserMessage { content: text("hello") }, assistant("prior reply")];
    let lean = body_of(WireApi::Responses, NO_POLICY, request_with(items.clone())).await;
    assert_eq!(lean["input"][1], json!({ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "prior reply", "annotations": [] }] }));
    let policy = VendorPolicy {
        replay_assistant_status: true,
        ..NO_POLICY
    };
    let strict = body_of(WireApi::Responses, policy, request_with(items)).await;
    assert_eq!(strict["input"][1]["status"], json!("completed"));
}

#[tokio::test]
async fn only_this_providers_reasoning_items_are_replayed_with_their_replayable_fields() {
    let item = json!({ "type": "reasoning", "id": "rs_1", "summary": [{ "type": "summary_text", "text": "sum" }], "encrypted_content": "enc", "status": "completed" });
    let body = body_of(
        WireApi::Responses,
        NO_POLICY,
        request_with(vec![
            InferenceItem::UserMessage { content: text("hello") },
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
async fn user_media_rides_as_images_and_files() {
    let png = B64Bytes::from(&b"PNG"[..]);
    let content = vec![
        UserContentBlock::Text { text: "look".into() },
        UserContentBlock::Image { source: MediaSource::Binary { data: png, media_type: "image/png".into() } },
        UserContentBlock::Image { source: MediaSource::Url { url: "https://example.com/a.png".into() } },
        UserContentBlock::Document {
            source: DocumentSource::Binary { data: B64Bytes::from(&b"%PDF"[..]), media_type: "application/pdf".into(), file_name: "a.pdf".into() },
        },
        UserContentBlock::Reference { reference: "see notes.md".into() },
    ];
    let request = || request_with(vec![InferenceItem::UserMessage { content: content.clone() }]);
    let responses = body_of(WireApi::Responses, NO_POLICY, request()).await;
    assert_eq!(
        responses["input"][0]["content"],
        json!([
            { "type": "input_text", "text": "look" },
            { "type": "input_image", "image_url": "data:image/png;base64,UE5H", "detail": "auto" },
            { "type": "input_image", "image_url": "https://example.com/a.png", "detail": "auto" },
            { "type": "input_file", "filename": "a.pdf", "file_data": "data:application/pdf;base64,JVBERg==" },
            { "type": "input_text", "text": "see notes.md" },
        ])
    );
    let chat = body_of(WireApi::ChatCompletions, NO_POLICY, request()).await;
    assert_eq!(
        chat["messages"][0]["content"],
        json!([
            { "type": "text", "text": "look" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,UE5H", "detail": "auto" } },
            { "type": "image_url", "image_url": { "url": "https://example.com/a.png", "detail": "auto" } },
            { "type": "file", "file": { "filename": "a.pdf", "file_data": "data:application/pdf;base64,JVBERg==" } },
            { "type": "text", "text": "see notes.md" },
        ])
    );
}

#[tokio::test]
async fn media_a_tool_returned_follows_its_output_in_a_user_message() {
    let output = vec![
        ToolResultContentBlock::Text { text: "<binary stdout: 75 bytes>".into() },
        ToolResultContentBlock::Image { source: ToolMediaSource::Binary { data: B64Bytes::from(&b"AAAA"[..]), media_type: "image/png".into() } },
        ToolResultContentBlock::Video { source: ToolMediaSource::Binary { data: B64Bytes::from(&b"MP4"[..]), media_type: "video/mp4".into() } },
    ];
    let responses = body_of(
        WireApi::Responses,
        NO_POLICY,
        request_with(vec![tool_use("call-1|fc-1", "shot.png"), tool_result("call-1|fc-1", output.clone())]),
    )
    .await;
    assert_eq!(
        responses["input"].as_array().unwrap()[1..],
        [
            json!({ "type": "function_call_output", "call_id": "call-1", "output": "<binary stdout: 75 bytes>\n[image:image/png]\n[video:video/mp4]" }),
            json!({ "role": "user", "content": [
                { "type": "input_text", "text": "[media returned by tool call call-1]" },
                { "type": "input_image", "image_url": "data:image/png;base64,QUFBQQ==", "detail": "auto" },
                { "type": "input_image", "image_url": "data:video/mp4;base64,TVA0", "detail": "auto" },
            ] }),
        ]
    );
    // A tool message is text only, so the media rides after it here too.
    let chat = body_of(WireApi::ChatCompletions, NO_POLICY, request_with(vec![tool_use("call-1", "shot.png"), tool_result("call-1", output)])).await;
    assert_eq!(chat["messages"][1], json!({ "role": "tool", "tool_call_id": "call-1", "content": "<binary stdout: 75 bytes>\n[image:image/png]\n[video:video/mp4]" }));
    assert_eq!(
        chat["messages"][2],
        json!({ "role": "user", "content": [
            { "type": "text", "text": "[media returned by tool call call-1]" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,QUFBQQ==", "detail": "auto" } },
            { "type": "image_url", "image_url": { "url": "data:video/mp4;base64,TVA0", "detail": "auto" } },
        ] })
    );
}

#[tokio::test]
async fn a_chat_completions_body_carries_the_transcript_tools_tier_and_effort() {
    let mut request = request_with(vec![
        InferenceItem::UserMessage { content: text("hello") },
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

/// The Chat Completions messages of `items` with thinking passed back.
async fn replayed(items: Vec<InferenceItem>, pass_back: bool) -> Value {
    let policy = VendorPolicy {
        pass_back_reasoning_content: pass_back,
        ..NO_POLICY
    };
    body_of(WireApi::ChatCompletions, policy, request_with(items)).await["messages"].clone()
}

#[tokio::test]
async fn thinking_is_replayed_as_reasoning_content_only_when_the_vendor_policy_asks() {
    let tool_round = vec![
        InferenceItem::UserMessage { content: text("hello") },
        thinking("inspect ", None),
        thinking("first", None),
        InferenceItem::AssistantRedactedThinking { model_id: "deepseek-v4-pro".into(), data: "opaque".into() },
        tool_use("call-1", "a.ts"),
        tool_result("call-1", contents("contents")),
    ];
    let call = json!([{ "id": "call-1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"a.ts\"}" } }]);
    let dropped = replayed(tool_round.clone(), false).await;
    assert_eq!(dropped[1], json!({ "role": "assistant", "content": null, "tool_calls": call }));
    let kept = replayed(tool_round, true).await;
    assert_eq!(kept[1], json!({ "role": "assistant", "content": null, "tool_calls": call, "reasoning_content": "inspect first" }));

    // A tool round without thinking still carries an empty one.
    let second_round = replayed(
        vec![
            InferenceItem::UserMessage { content: text("hello") },
            thinking("first thought", None),
            tool_use("call-1", "a.ts"),
            tool_result("call-1", contents("a")),
            tool_use("call-2", "b.ts"),
            tool_result("call-2", contents("b")),
        ],
        true,
    )
    .await;
    assert_eq!(second_round[1]["reasoning_content"], json!("first thought"));
    assert_eq!(second_round[3]["reasoning_content"], json!(""));

    // Thinking stays within its assistant message; thinking no message
    // followed goes nowhere.
    let segments = replayed(
        vec![
            InferenceItem::UserMessage { content: text("first") },
            thinking("thought one", None),
            assistant("answer one"),
            InferenceItem::UserMessage { content: text("second") },
            thinking("orphaned", None),
            InferenceItem::UserMessage { content: text("third") },
            assistant("answer three"),
        ],
        true,
    )
    .await;
    assert_eq!(
        segments,
        json!([
            { "role": "user", "content": "first" },
            { "role": "assistant", "content": "answer one", "reasoning_content": "thought one" },
            { "role": "user", "content": "second" },
            { "role": "user", "content": "third" },
            { "role": "assistant", "content": "answer three" },
        ])
    );
}
