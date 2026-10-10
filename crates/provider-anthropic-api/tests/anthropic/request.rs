//! The request a run sends (`models.md` § Request parameters).

use std::{num::NonZeroU32, sync::Arc};

use demi_provider_common::{
    InferenceItem, MediaBytes, Medium, PromptCache, Provider, ProviderEvent, ResultPart,
    RuntimeEnv, ToolDefinition, UserPart, VendorPolicy,
    testing::{MockVendor, inference_request},
};
use demi_shared_types::{B64Bytes, ThinkingConfig, ThinkingSummary, TokenUsage};
use serde_json::{Value, json};

use crate::{provider_at, run, runtime, runtime_with, stop};

fn text(text: &str) -> Vec<UserPart> {
    vec![UserPart::Text(text.into())]
}

/// The body the vendor received for a request carrying `items` and
/// `thinking` with the given output limit.
async fn body(
    items: Vec<InferenceItem>,
    thinking: Option<ThinkingConfig>,
    output_limit: Option<u32>,
) -> Value {
    body_for(items, thinking, output_limit, VendorPolicy::default()).await
}

/// The body a provider for a vendor of `policy` sends.
async fn body_for(
    items: Vec<InferenceItem>,
    thinking: Option<ThinkingConfig>,
    output_limit: Option<u32>,
    policy: VendorPolicy,
) -> Value {
    let vendor = MockVendor::start().await;
    vendor.respond(stop());
    let mut request = inference_request();
    request.items = items.into();
    request.thinking = thinking;
    request.output_limit = output_limit.and_then(NonZeroU32::new);
    run(runtime_with(&vendor, policy).as_mut(), request).await;
    vendor.requests()[0].json()
}

#[tokio::test]
async fn a_run_posts_to_the_messages_endpoint_with_the_key_and_version() {
    let vendor = MockVendor::start().await;
    for base in ["/v1", "/v1/", "/v1/messages"] {
        vendor.respond(stop());
        let mut runtime = provider_at(&vendor, base)
            .runtime(RuntimeEnv {
                http: reqwest::Client::new(),
            })
            .unwrap();
        let events = run(runtime.as_mut(), inference_request()).await;
        assert_eq!(events, [ProviderEvent::Response(TokenUsage::default())]);
    }
    for request in vendor.requests() {
        assert_eq!(
            (request.method.as_str(), request.uri.path()),
            ("POST", "/v1/messages")
        );
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
    request.thinking = Some(ThinkingConfig::Budget {
        budget_tokens: 1024,
    });
    request.items = Arc::new([
        InferenceItem::UserMessage {
            content: text("hello"),
        },
        InferenceItem::AssistantText {
            model_id: "claude-test".into(),
            text: "Use tool".into(),
        },
        InferenceItem::ToolUse {
            model_id: "claude-test".into(),
            tool_use_id: "toolu-1".into(),
            tool_name: "read_file".into(),
            input: json!({ "path": "a.ts" }),
        },
        InferenceItem::ToolResult {
            tool_use_id: "toolu-1".into(),
            output: vec![ResultPart::Text("contents".into())],
            is_error: false,
        },
        InferenceItem::UserSteer {
            content: text("also check b.ts"),
        },
        InferenceItem::ToolUse {
            model_id: "claude-test".into(),
            tool_use_id: "toolu-2".into(),
            tool_name: "read_file".into(),
            input: Value::Null,
        },
        InferenceItem::ToolResult {
            tool_use_id: "toolu-2".into(),
            output: Vec::new(),
            is_error: true,
        },
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
async fn a_session_request_marks_its_shared_prefix_the_latest_answered_request_and_its_end_for_an_hour()
 {
    // The latest answered request carried the user's message. The model
    // then called a tool twelve times at once, and this request adds its
    // answer, each call with its result, and a steer: more than the 20 blocks
    // a mark looks back over for an earlier entry.
    let model = || "model-a".to_owned();
    let mut items = vec![
        InferenceItem::UserMessage {
            content: text("check every file"),
        },
        InferenceItem::AssistantThinking {
            model_id: model(),
            text: "plan".into(),
            signature: Some("anthropic:sig-1".into()),
            kept_past_summary: false,
        },
        InferenceItem::AssistantText {
            model_id: model(),
            text: "Reading them all".into(),
        },
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
            output: vec![ResultPart::Text("contents".into())],
            is_error: false,
        });
    }
    items.push(InferenceItem::UserSteer {
        content: text("also check b.ts"),
    });
    let hour = json!({ "type": "ephemeral", "ttl": "1h" });
    // Messages: the user's (0), the answer with the first call (1), then
    // each result and the next call in turn, and the last result with the
    // steer (24).
    let the_end = ("/messages/24/content/1".to_owned(), hour.clone());
    let the_question = ("/messages/0/content/0".to_owned(), hour.clone());
    let cases = [
        // The system prompt, the latest answered request's last block, and
        // the request's own last block.
        (
            "system",
            PromptCache::Session { answered_items: 1 },
            vec![
                the_question.clone(),
                the_end.clone(),
                ("/system/0".to_owned(), hour.clone()),
            ],
        ),
        // Without a system prompt, the tools end the shared prefix.
        (
            " ",
            PromptCache::Session { answered_items: 1 },
            vec![
                the_question.clone(),
                the_end.clone(),
                ("/tools/0".to_owned(), hour.clone()),
            ],
        ),
        // A thinking block takes no mark: the nearest block before it does.
        (
            "system",
            PromptCache::Session { answered_items: 2 },
            vec![
                the_question.clone(),
                the_end.clone(),
                ("/system/0".to_owned(), hour.clone()),
            ],
        ),
        // Before any answer, only the shared prefix and the end.
        (
            "system",
            PromptCache::Session { answered_items: 0 },
            vec![the_end.clone(), ("/system/0".to_owned(), hour.clone())],
        ),
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
        assert_eq!(
            body["messages"][24]["content"][1]["text"],
            "also check b.ts"
        );
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
    for absent in [
        "system",
        "tools",
        "thinking",
        "output_config",
        "service_tier",
    ] {
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
    let items = || {
        vec![InferenceItem::UserMessage {
            content: text("hi"),
        }]
    };
    let cases = [
        // A budget stays below max_tokens and at least the API's minimum.
        (
            Some(ThinkingConfig::Budget {
                budget_tokens: 999_999,
            }),
            Some(8_192),
            json!({ "type": "enabled", "budget_tokens": 7_168 }),
            Value::Null,
        ),
        (
            Some(ThinkingConfig::Budget { budget_tokens: 100 }),
            None,
            json!({ "type": "enabled", "budget_tokens": 1_024 }),
            Value::Null,
        ),
        // An effort is adaptive thinking at that effort, summarized unless
        // summaries are off.
        (
            Some(ThinkingConfig::Effort {
                effort: "high".into(),
                summary: None,
            }),
            None,
            json!({ "type": "adaptive", "display": "summarized" }),
            json!({ "effort": "high" }),
        ),
        (
            Some(ThinkingConfig::Effort {
                effort: "low".into(),
                summary: Some(ThinkingSummary::Off),
            }),
            None,
            json!({ "type": "adaptive", "display": "omitted" }),
            json!({ "effort": "low" }),
        ),
        (
            Some(ThinkingConfig::Adaptive {
                effort: "max".into(),
            }),
            None,
            json!({ "type": "adaptive", "display": "summarized" }),
            json!({ "effort": "max" }),
        ),
        (
            Some(ThinkingConfig::Disabled {}),
            None,
            Value::Null,
            Value::Null,
        ),
        (None, None, Value::Null, Value::Null),
    ];
    for (thinking, limit, expected, output_config) in cases {
        let body = body(items(), thinking.clone(), limit).await;
        assert_eq!(
            body.get("thinking").cloned().unwrap_or(Value::Null),
            expected,
            "{thinking:?}"
        );
        assert_eq!(
            body.get("output_config").cloned().unwrap_or(Value::Null),
            output_config,
            "{thinking:?}"
        );
    }

    // A vendor that takes only budgets, as every Anthropic-compatible
    // endpoint but Anthropic's own: an effort is its budget, kept within the
    // same bounds, and an effort the ladder does not name thinks like medium.
    let budgets = VendorPolicy {
        effort_as_budget: true,
        ..VendorPolicy::default()
    };
    let cases = [
        (
            ThinkingConfig::Effort {
                effort: "high".into(),
                summary: None,
            },
            Some(128_000),
            32_768,
        ),
        (
            ThinkingConfig::Adaptive {
                effort: "max".into(),
            },
            Some(8_192),
            7_168,
        ),
        (
            ThinkingConfig::Effort {
                effort: "minimal".into(),
                summary: None,
            },
            Some(128_000),
            16_384,
        ),
    ];
    for (thinking, limit, budget_tokens) in cases {
        let body = body_for(items(), Some(thinking.clone()), limit, budgets).await;
        assert_eq!(
            body["thinking"],
            json!({ "type": "enabled", "budget_tokens": budget_tokens }),
            "{thinking:?}"
        );
        assert_eq!(body.get("output_config"), None, "{thinking:?}");
    }
}

#[tokio::test]
async fn thinking_is_sent_back_only_when_this_provider_received_it_and_no_summary_replaced_its_history()
 {
    let model = || "claude-opus-4-8".to_owned();
    let body = body(
        vec![
            // Reasoning compaction kept after a summary would fail the
            // vendor's check of the history before it.
            InferenceItem::UserMessage {
                content: text("Previous conversation summary:\nthe user said hello"),
            },
            InferenceItem::AssistantThinking {
                model_id: model(),
                text: "kept".into(),
                signature: Some("anthropic:sig-0".into()),
                kept_past_summary: true,
            },
            InferenceItem::AssistantRedactedThinking {
                model_id: model(),
                data: "anthropic:kept-opaque".into(),
                kept_past_summary: true,
            },
            InferenceItem::AssistantText {
                model_id: model(),
                text: "hello".into(),
            },
            InferenceItem::UserMessage {
                content: text("hi"),
            },
            InferenceItem::AssistantThinking {
                model_id: model(),
                text: "plan".into(),
                signature: Some("anthropic:sig-1".into()),
                kept_past_summary: false,
            },
            InferenceItem::AssistantThinking {
                model_id: model(),
                text: "theirs".into(),
                signature: Some("google:sig-2".into()),
                kept_past_summary: false,
            },
            InferenceItem::AssistantThinking {
                model_id: model(),
                text: "unsigned".into(),
                signature: None,
                kept_past_summary: false,
            },
            InferenceItem::AssistantRedactedThinking {
                model_id: model(),
                data: "anthropic:opaque".into(),
                kept_past_summary: false,
            },
            InferenceItem::AssistantRedactedThinking {
                model_id: model(),
                data: "opaque-elsewhere".into(),
                kept_past_summary: false,
            },
            InferenceItem::ToolUse {
                model_id: model(),
                tool_use_id: "toolu-1".into(),
                tool_name: "ls".into(),
                input: json!({}),
            },
        ],
        None,
        None,
    )
    .await;
    assert_eq!(
        body["messages"][1],
        json!({ "role": "assistant", "content": [{ "type": "text", "text": "hello" }] })
    );
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
    let bytes = |data: &[u8], media_type: &str| MediaBytes {
        data: B64Bytes::from(data.to_vec()),
        media_type: media_type.into(),
    };
    let png = bytes(&[0x89, b'P', b'N', b'G'], "image/png");
    let body = body(
        vec![
            InferenceItem::UserMessage {
                content: vec![
                    UserPart::Image(Medium::Bytes(png.clone())),
                    UserPart::Image(Medium::Url("https://example.com/a.png".into())),
                    UserPart::Document {
                        bytes: bytes(b"%PDF", "application/pdf"),
                        file_name: "spec.pdf".into(),
                    },
                    UserPart::Video(Medium::Bytes(bytes(&[0x89, b'P', b'N', b'G'], "video/mp4"))),
                ],
            },
            InferenceItem::ToolUse {
                model_id: "m".into(),
                tool_use_id: "toolu-1".into(),
                tool_name: "shot".into(),
                input: json!({}),
            },
            InferenceItem::ToolResult {
                tool_use_id: "toolu-1".into(),
                output: vec![
                    ResultPart::Image(png),
                    ResultPart::Video(bytes(b"\x1a\x45\xdf\xa3", "video/webm")),
                    ResultPart::Document {
                        bytes: bytes(b"%PDF", "application/pdf"),
                        file_name: "document-3.pdf".into(),
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
        ])
    );
    assert_eq!(
        body["messages"][2]["content"][0]["content"],
        json!([
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "iVBORw==" } },
            { "type": "text", "text": "[video:video/webm]" },
            { "type": "document", "source": { "type": "base64", "media_type": "application/pdf", "data": "JVBERg==" }, "title": "document-3.pdf" },
        ])
    );
}
