//! The request a run sends (`models.md` § Request parameters).

use std::{num::NonZeroU32, sync::Arc};

use demi_core::{
    Attachment, B64Bytes, DocumentSource, MediaSource, ThinkingConfig, ThinkingSummary,
    ToolMediaSource, ToolResultContentBlock, TokenUsage, UserContentBlock,
};
use demi_provider::{
    InferenceItem, PromptCache, ProviderEvent, Provider, RuntimeEnv, ToolDefinition,
    testing::{MockVendor, inference_request},
};
use serde_json::{Value, json};

use crate::{provider_at, run, runtime, stop};

fn text(text: &str) -> Vec<UserContentBlock> {
    vec![UserContentBlock::Text { text: text.into() }]
}

/// The body the vendor received for a request carrying `items` and
/// `thinking` with the given output limit.
async fn body(items: Vec<InferenceItem>, thinking: Option<ThinkingConfig>, output_limit: Option<u32>) -> Value {
    let vendor = MockVendor::start().await;
    vendor.respond(stop());
    let mut request = inference_request();
    request.items = items.into();
    request.thinking = thinking;
    request.output_limit = output_limit.and_then(NonZeroU32::new);
    run(runtime(&vendor).as_mut(), request).await;
    vendor.requests()[0].json()
}

#[tokio::test]
async fn a_run_posts_to_the_messages_endpoint_with_the_key_and_version() {
    let vendor = MockVendor::start().await;
    for base in ["/v1", "/v1/", "/v1/messages"] {
        vendor.respond(stop());
        let mut runtime = provider_at(&vendor, base)
            .runtime(RuntimeEnv { http: reqwest::Client::new() })
            .unwrap();
        let events = run(runtime.as_mut(), inference_request()).await;
        assert_eq!(events, [ProviderEvent::Response(TokenUsage::default())]);
    }
    for request in vendor.requests() {
        assert_eq!((request.method.as_str(), request.uri.path()), ("POST", "/v1/messages"));
        assert_eq!(request.header("x-api-key"), Some("sk-ant-test"));
        assert_eq!(request.header("anthropic-version"), Some("2023-06-01"));
        assert_eq!(request.header("content-type"), Some("application/json"));
        assert_eq!(request.header("accept"), Some("text/event-stream"));
    }
}

#[tokio::test]
async fn the_body_groups_turns_and_carries_the_tools_system_prompt_and_tier() {
    let vendor = MockVendor::start().await;
    vendor.respond(stop());
    let mut request = inference_request();
    request.model_id = "claude-test".into();
    request.system_prompt = "system instructions".into();
    request.service_tier_id = Some("standard_only".into());
    request.output_limit = NonZeroU32::new(8192);
    request.thinking = Some(ThinkingConfig::Budget { budget_tokens: 1024 });
    request.items = Arc::new([
        InferenceItem::UserMessage { content: text("hello") },
        InferenceItem::AssistantText { model_id: "claude-test".into(), text: "Use tool".into() },
        InferenceItem::ToolUse {
            model_id: "claude-test".into(),
            tool_use_id: "toolu-1".into(),
            tool_name: "read_file".into(),
            input: json!({ "path": "a.ts" }),
        },
        InferenceItem::ToolResult {
            tool_use_id: "toolu-1".into(),
            output: vec![ToolResultContentBlock::Text { text: "contents".into() }],
            is_error: false,
        },
        InferenceItem::UserSteer { content: text("also check b.ts") },
        InferenceItem::ToolUse {
            model_id: "claude-test".into(),
            tool_use_id: "toolu-2".into(),
            tool_name: "read_file".into(),
            input: Value::Null,
        },
        InferenceItem::ToolResult { tool_use_id: "toolu-2".into(), output: Vec::new(), is_error: true },
    ]);
    let schema = json!({ "type": "object", "properties": { "path": { "type": "string" } } });
    request.tools = Arc::new([ToolDefinition {
        name: "read_file".into(),
        description: "Read a file".into(),
        input_schema: schema.as_object().unwrap().clone(),
    }]);
    run(runtime(&vendor).as_mut(), request).await;

    assert_eq!(
        vendor.requests()[0].json(),
        json!({
            "model": "claude-test",
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "hello" }] },
                { "role": "assistant", "content": [
                    { "type": "text", "text": "Use tool" },
                    { "type": "tool_use", "id": "toolu-1", "name": "read_file", "input": { "path": "a.ts" } },
                ] },
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu-1", "content": [{ "type": "text", "text": "contents" }] },
                    { "type": "text", "text": "also check b.ts" },
                ] },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "toolu-2", "name": "read_file", "input": {} },
                ] },
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu-2", "content": [], "is_error": true },
                ] },
            ],
            "max_tokens": 8192,
            "stream": true,
            "system": [{ "type": "text", "text": "system instructions" }],
            "tools": [{ "name": "read_file", "description": "Read a file", "input_schema": schema }],
            "thinking": { "type": "enabled", "budget_tokens": 1024 },
            "service_tier": "standard_only",
        })
    );
}

/// Every cache mark of `body`: where it sits, as a JSON pointer, and what it
/// says, in the order of the pointers.
fn marks(body: &Value) -> Vec<(String, Value)> {
    fn visit(value: &Value, path: &str, found: &mut Vec<(String, Value)>) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    if key == "cache_control" {
                        found.push((path.to_owned(), value.clone()));
                    } else {
                        visit(value, &format!("{path}/{key}"), found);
                    }
                }
            }
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    visit(value, &format!("{path}/{index}"), found);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    visit(body, "", &mut found);
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

#[tokio::test]
async fn a_session_request_marks_its_shared_prefix_the_latest_answered_request_and_its_end_for_an_hour() {
    // The latest answered request carried the user's message. The model
    // then called a tool twelve times at once, and this request adds its
    // answer, each call with its result, and a steer: more than the 20 blocks
    // a mark looks back over for an earlier entry.
    let model = || "model-a".to_owned();
    let mut items = vec![
        InferenceItem::UserMessage { content: text("check every file") },
        InferenceItem::AssistantThinking { model_id: model(), text: "plan".into(), signature: Some("anthropic:sig-1".into()), kept_past_summary: false },
        InferenceItem::AssistantText { model_id: model(), text: "Reading them all".into() },
    ];
    for call in 0..12 {
        let id = format!("toolu-{call}");
        items.push(InferenceItem::ToolUse {
            model_id: model(),
            tool_use_id: id.clone(),
            tool_name: "read_file".into(),
            input: json!({ "path": format!("{call}.ts") }),
        });
        items.push(InferenceItem::ToolResult {
            tool_use_id: id,
            output: vec![ToolResultContentBlock::Text { text: "contents".into() }],
            is_error: false,
        });
    }
    items.push(InferenceItem::UserSteer { content: text("also check b.ts") });
    let hour = json!({ "type": "ephemeral", "ttl": "1h" });
    // Messages: the user's (0), the answer with the first call (1), then
    // each result and the next call in turn, and the last result with the
    // steer (24).
    let the_end = ("/messages/24/content/1".to_owned(), hour.clone());
    let the_question = ("/messages/0/content/0".to_owned(), hour.clone());
    let cases = [
        // The system prompt, the latest answered request's last block, and
        // the request's own last block.
        ("system", PromptCache::Session { answered_items: 1 }, vec![the_question.clone(), the_end.clone(), ("/system/0".to_owned(), hour.clone())]),
        // Without a system prompt, the tools end the shared prefix.
        (" ", PromptCache::Session { answered_items: 1 }, vec![the_question.clone(), the_end.clone(), ("/tools/0".to_owned(), hour.clone())]),
        // A thinking block takes no mark: the nearest block before it does.
        ("system", PromptCache::Session { answered_items: 2 }, vec![the_question.clone(), the_end.clone(), ("/system/0".to_owned(), hour.clone())]),
        // Before any answer, only the shared prefix and the end.
        ("system", PromptCache::Session { answered_items: 0 }, vec![the_end.clone(), ("/system/0".to_owned(), hour.clone())]),
        // A request no later request extends, such as a title request.
        ("system", PromptCache::Off, Vec::new()),
    ];
    for (system_prompt, prompt_cache, expected) in cases {
        let vendor = MockVendor::start().await;
        vendor.respond(stop());
        let mut request = inference_request();
        request.model_id = model();
        request.system_prompt = system_prompt.into();
        request.tools = Arc::new([ToolDefinition {
            name: "read_file".into(),
            description: "Read a file".into(),
            input_schema: serde_json::Map::new(),
        }]);
        request.items = items.clone().into();
        request.prompt_cache = prompt_cache;
        run(runtime(&vendor).as_mut(), request).await;
        let body = vendor.requests()[0].json();
        assert_eq!(body["messages"][24]["content"][1]["text"], "also check b.ts");
        assert_eq!(marks(&body), expected, "{prompt_cache:?} {system_prompt:?}");
    }
}

#[tokio::test]
async fn a_blank_system_prompt_no_tools_and_no_tier_are_left_out() {
    let vendor = MockVendor::start().await;
    vendor.respond(stop());
    let mut request = inference_request();
    request.system_prompt = " \n\t".into();
    run(runtime(&vendor).as_mut(), request).await;
    let body = vendor.requests()[0].json();
    for absent in ["system", "tools", "thinking", "output_config", "service_tier"] {
        assert!(body.get(absent).is_none(), "{absent}: {body}");
    }
    assert_eq!(body["max_tokens"], json!(32_000));
}

#[tokio::test]
async fn a_reused_runtime_uses_the_output_limit_of_each_request() {
    let vendor = MockVendor::start().await;
    let mut runtime = runtime(&vendor);
    // A request's cap lowers the model's limit, and stands in for none.
    let cases = [
        ("a", Some(8_000), None),
        ("b", Some(32_000), None),
        ("c", None, None),
        ("d", Some(8_000), Some(1_024)),
        ("e", None, Some(1_024)),
    ];
    for (model, limit, cap) in cases {
        vendor.respond(stop());
        let mut request = inference_request();
        request.model_id = model.into();
        request.output_limit = limit.and_then(NonZeroU32::new);
        request.output_cap = cap.and_then(NonZeroU32::new);
        run(runtime.as_mut(), request).await;
    }
    let sent: Vec<(Value, Value)> = vendor
        .requests()
        .iter()
        .map(|request| {
            let body = request.json();
            (body["model"].clone(), body["max_tokens"].clone())
        })
        .collect();
    assert_eq!(
        sent,
        [
            (json!("a"), json!(8_000)),
            (json!("b"), json!(32_000)),
            (json!("c"), json!(32_000)),
            (json!("d"), json!(1_024)),
            (json!("e"), json!(1_024))
        ]
    );
}

#[tokio::test]
async fn thinking_maps_onto_a_budget_or_adaptive_thinking_at_an_effort() {
    let items = || vec![InferenceItem::UserMessage { content: text("hi") }];
    let cases = [
        // A budget stays below max_tokens and at least the API's minimum.
        (Some(ThinkingConfig::Budget { budget_tokens: 999_999 }), Some(8_192), json!({ "type": "enabled", "budget_tokens": 7_168 }), Value::Null),
        (Some(ThinkingConfig::Budget { budget_tokens: 100 }), None, json!({ "type": "enabled", "budget_tokens": 1_024 }), Value::Null),
        // An effort is adaptive thinking at that effort, summarized unless
        // summaries are off.
        (
            Some(ThinkingConfig::Effort { effort: "high".into(), summary: None }),
            None,
            json!({ "type": "adaptive", "display": "summarized" }),
            json!({ "effort": "high" }),
        ),
        (
            Some(ThinkingConfig::Effort { effort: "low".into(), summary: Some(ThinkingSummary::Off) }),
            None,
            json!({ "type": "adaptive", "display": "omitted" }),
            json!({ "effort": "low" }),
        ),
        (
            Some(ThinkingConfig::Adaptive { effort: "max".into() }),
            None,
            json!({ "type": "adaptive", "display": "summarized" }),
            json!({ "effort": "max" }),
        ),
        (Some(ThinkingConfig::Disabled {}), None, Value::Null, Value::Null),
        (None, None, Value::Null, Value::Null),
    ];
    for (thinking, limit, expected, output_config) in cases {
        let body = body(items(), thinking.clone(), limit).await;
        assert_eq!(body.get("thinking").cloned().unwrap_or(Value::Null), expected, "{thinking:?}");
        assert_eq!(body.get("output_config").cloned().unwrap_or(Value::Null), output_config, "{thinking:?}");
    }
}

#[tokio::test]
async fn thinking_is_sent_back_only_when_this_provider_received_it_and_no_summary_replaced_its_history() {
    let model = || "claude-opus-4-8".to_owned();
    let body = body(
        vec![
            // Reasoning compaction kept after a summary would fail the
            // vendor's check of the history before it.
            InferenceItem::UserMessage { content: text("Previous conversation summary:\nthe user said hello") },
            InferenceItem::AssistantThinking { model_id: model(), text: "kept".into(), signature: Some("anthropic:sig-0".into()), kept_past_summary: true },
            InferenceItem::AssistantRedactedThinking { model_id: model(), data: "anthropic:kept-opaque".into(), kept_past_summary: true },
            InferenceItem::AssistantText { model_id: model(), text: "hello".into() },
            InferenceItem::UserMessage { content: text("hi") },
            InferenceItem::AssistantThinking { model_id: model(), text: "plan".into(), signature: Some("anthropic:sig-1".into()), kept_past_summary: false },
            InferenceItem::AssistantThinking { model_id: model(), text: "theirs".into(), signature: Some("google:sig-2".into()), kept_past_summary: false },
            InferenceItem::AssistantThinking { model_id: model(), text: "unsigned".into(), signature: None, kept_past_summary: false },
            InferenceItem::AssistantRedactedThinking { model_id: model(), data: "anthropic:opaque".into(), kept_past_summary: false },
            InferenceItem::AssistantRedactedThinking { model_id: model(), data: "opaque-elsewhere".into(), kept_past_summary: false },
            InferenceItem::ToolUse { model_id: model(), tool_use_id: "toolu-1".into(), tool_name: "ls".into(), input: json!({}) },
        ],
        None,
        None,
    )
    .await;
    assert_eq!(body["messages"][1], json!({ "role": "assistant", "content": [{ "type": "text", "text": "hello" }] }));
    assert_eq!(
        body["messages"][3],
        json!({ "role": "assistant", "content": [
            { "type": "thinking", "thinking": "plan", "signature": "sig-1" },
            { "type": "redacted_thinking", "data": "opaque" },
            { "type": "tool_use", "id": "toolu-1", "name": "ls", "input": {} },
        ] })
    );
}

#[tokio::test]
async fn media_travels_inline_and_what_the_api_cannot_read_becomes_text() {
    let png = B64Bytes::from(vec![0x89, b'P', b'N', b'G']);
    let pdf = B64Bytes::from(b"%PDF".to_vec());
    let attachment = Attachment {
        name: "notes.md".into(),
        path: "/home/demi/.demi/attachments/c1/notes.md".into(),
        media_type: "text/markdown".into(),
        size_bytes: 82,
        sha256: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".parse().unwrap(),
        snippet: None,
    };
    let body = body(
        vec![
            InferenceItem::UserMessage {
                content: vec![
                    UserContentBlock::Image { source: MediaSource::Binary { data: png.clone(), media_type: "image/png".into() } },
                    UserContentBlock::Image { source: MediaSource::Url { url: "https://example.com/a.png".into() } },
                    UserContentBlock::Document {
                        source: DocumentSource::Binary { data: pdf, media_type: "application/pdf".into(), file_name: "spec.pdf".into() },
                    },
                    UserContentBlock::Video { source: MediaSource::Binary { data: png.clone(), media_type: "video/mp4".into() } },
                    UserContentBlock::Attachment(attachment),
                    UserContentBlock::Reference { reference: "src/main.rs".into() },
                ],
            },
            InferenceItem::ToolUse { model_id: "m".into(), tool_use_id: "toolu-1".into(), tool_name: "shot".into(), input: json!({}) },
            InferenceItem::ToolResult {
                tool_use_id: "toolu-1".into(),
                output: vec![
                    ToolResultContentBlock::Image { source: ToolMediaSource::Binary { data: png, media_type: "image/png".into() } },
                    ToolResultContentBlock::Video {
                        source: ToolMediaSource::Ref {
                            r#ref: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".parse().unwrap(),
                            media_type: "video/webm".into(),
                        },
                    },
                ],
                is_error: false,
            },
        ],
        None,
        None,
    )
    .await;
    assert_eq!(
        body["messages"][0]["content"],
        json!([
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "iVBORw==" } },
            { "type": "text", "text": "[image:https://example.com/a.png]" },
            { "type": "document", "source": { "type": "base64", "media_type": "application/pdf", "data": "JVBERg==" }, "title": "spec.pdf" },
            { "type": "text", "text": "[video]" },
            { "type": "text", "text": "<attachment name=\"notes.md\" type=\"text/markdown\" size=\"82\" path=\"/home/demi/.demi/attachments/c1/notes.md\"/>" },
            { "type": "text", "text": "src/main.rs" },
        ])
    );
    assert_eq!(
        body["messages"][2]["content"][0]["content"],
        json!([
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "iVBORw==" } },
            { "type": "text", "text": "[video:video/webm]" },
        ])
    );
}

#[tokio::test]
async fn media_that_was_not_loaded_fails_the_run_before_any_request() {
    let vendor = MockVendor::start().await;
    let mut request = inference_request();
    request.items = Arc::new([InferenceItem::UserMessage {
        content: vec![UserContentBlock::Image {
            source: MediaSource::Ref {
                r#ref: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".parse().unwrap(),
                media_type: "image/png".into(),
            },
        }],
    }]);
    let events = run(runtime(&vendor).as_mut(), request).await;
    let [ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(failure.code, None);
    assert!(failure.message.contains("9f86d081"), "{}", failure.message);
    assert!(vendor.requests().is_empty());
}
