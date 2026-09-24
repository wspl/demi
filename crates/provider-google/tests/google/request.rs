//! The `generateContent` request a run sends (`models.md` § Request
//! parameters, `providers.md` § Endpoints).

use std::{num::NonZeroU32, sync::Arc};

use demi_core::{
    B64Bytes, MediaSource, ThinkingConfig, TokenUsage, ToolMediaSource, ToolResultContentBlock,
    UserContentBlock,
};
use demi_provider::{
    InferenceItem, InferenceRequest, Provider, ProviderEvent, RuntimeEnv, ToolDefinition,
    testing::{MockVendor, inference_request},
};
use serde_json::{Value, json};

use crate::{body_of, chunks, provider_at, run};

fn text(text: &str) -> Vec<UserContentBlock> {
    vec![UserContentBlock::Text { text: text.into() }]
}

fn request_with(items: Vec<InferenceItem>) -> InferenceRequest {
    let mut request = inference_request();
    request.items = items.into();
    request
}

fn signed(signature: &str) -> InferenceItem {
    InferenceItem::AssistantThinking {
        model_id: "gemini".into(),
        text: String::new(),
        signature: Some(signature.into()),
    }
}

fn tool_use(id: &str, name: &str, input: Value) -> InferenceItem {
    InferenceItem::ToolUse {
        model_id: "gemini".into(),
        tool_use_id: id.into(),
        tool_name: name.into(),
        input,
    }
}

fn tool_result(id: &str, output: Vec<ToolResultContentBlock>) -> InferenceItem {
    InferenceItem::ToolResult {
        tool_use_id: id.into(),
        output,
        is_error: false,
    }
}

#[tokio::test]
async fn a_run_streams_from_the_models_endpoint_with_the_key() {
    let vendor = MockVendor::start().await;
    for base in ["/v1beta", "/v1beta/"] {
        vendor.respond(chunks(&[]));
        let mut runtime = provider_at(&vendor, base)
            .runtime(RuntimeEnv {
                http: reqwest::Client::new(),
            })
            .unwrap();
        let mut request = inference_request();
        request.model_id = "gemini-3.6-flash".into();
        let events = run(runtime.as_mut(), request).await;
        assert_eq!(events, [ProviderEvent::Response(TokenUsage::default())]);
    }
    for sent in vendor.requests() {
        assert_eq!(
            sent.uri.to_string(),
            "/v1beta/models/gemini-3.6-flash:streamGenerateContent?alt=sse"
        );
        assert_eq!(sent.header("x-goog-api-key"), Some("google-key"));
        assert_eq!(sent.header("accept"), Some("text/event-stream"));
    }
}

#[tokio::test]
async fn each_request_sends_its_own_output_limit_or_the_agent_sized_default() {
    let mut request = inference_request();
    request.output_limit = NonZeroU32::new(8_000);
    assert_eq!(
        body_of(request).await["generationConfig"]["maxOutputTokens"],
        json!(8_000)
    );
    assert_eq!(
        body_of(inference_request()).await["generationConfig"]["maxOutputTokens"],
        json!(32_000)
    );
}

#[tokio::test]
async fn the_system_prompt_tools_and_thinking_budget_land_in_the_body() {
    let tool = ToolDefinition {
        name: "shell_exec".into(),
        description: "run".into(),
        input_schema: json!({ "type": "object" }).as_object().unwrap().clone(),
    };
    let mut request = request_with(vec![InferenceItem::UserMessage {
        content: text("hi"),
    }]);
    request.system_prompt = "you are a shell".into();
    request.tools = Arc::new([tool]);
    request.thinking = Some(ThinkingConfig::Effort {
        effort: "high".into(),
        summary: None,
    });
    let body = body_of(request.clone()).await;
    assert_eq!(
        body["systemInstruction"],
        json!({ "parts": [{ "text": "you are a shell" }] })
    );
    assert_eq!(
        body["tools"],
        json!([{ "functionDeclarations": [{ "name": "shell_exec", "description": "run", "parameters": { "type": "object" } }] }])
    );
    assert_eq!(
        body["contents"],
        json!([{ "role": "user", "parts": [{ "text": "hi" }] }])
    );
    assert_eq!(
        body["generationConfig"]["thinkingConfig"],
        json!({ "includeThoughts": true, "thinkingBudget": 32_768 })
    );

    let cases = [
        (
            Some(ThinkingConfig::Budget {
                budget_tokens: 2_048,
            }),
            json!({ "includeThoughts": true, "thinkingBudget": 2_048 }),
        ),
        (
            Some(ThinkingConfig::Adaptive {
                effort: "unheard-of".into(),
            }),
            json!({ "includeThoughts": true, "thinkingBudget": 16_384 }),
        ),
        (
            Some(ThinkingConfig::Disabled {}),
            json!({ "includeThoughts": false, "thinkingBudget": 0 }),
        ),
        (None, json!({ "includeThoughts": true })),
    ];
    for (thinking, expected) in cases {
        let mut request = request.clone();
        request.thinking = thinking.clone();
        assert_eq!(
            body_of(request).await["generationConfig"]["thinkingConfig"],
            expected,
            "{thinking:?}"
        );
    }
    let blank = body_of(inference_request()).await;
    assert!(blank.get("systemInstruction").is_none() && blank.get("tools").is_none());
}

#[tokio::test]
async fn tool_schemas_are_reduced_to_the_keywords_gemini_accepts() {
    let schema = json!({
        "type": "object",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "additionalProperties": false,
        "required": ["script"],
        "properties": {
            "script": { "type": "string", "minLength": 1, "description": "the script" },
            "tags": { "type": "array", "items": { "type": "string", "pattern": "^[a-z]+$" } },
            "mode": { "anyOf": [{ "type": "string", "const": "fast" }, { "type": "null" }] },
        },
    });
    let mut request = inference_request();
    request.tools = Arc::new([ToolDefinition {
        name: "shell_exec".into(),
        description: "run".into(),
        input_schema: schema.as_object().unwrap().clone(),
    }]);
    let body = body_of(request).await;
    assert_eq!(
        body["tools"][0]["functionDeclarations"][0]["parameters"],
        json!({
            "type": "object",
            "required": ["script"],
            "properties": {
                "script": { "type": "string", "description": "the script" },
                "tags": { "type": "array", "items": { "type": "string" } },
                "mode": { "anyOf": [{ "type": "string" }, { "type": "null" }] },
            },
        })
    );
}

#[tokio::test]
async fn a_tool_call_replays_with_the_signature_of_the_thinking_item_in_front_of_it() {
    let body = body_of(request_with(vec![
        InferenceItem::UserMessage {
            content: text("list files"),
        },
        signed("google:sig-abc"),
        tool_use("call-1", "shell_exec", json!({ "command": "ls" })),
        tool_result(
            "call-1",
            vec![ToolResultContentBlock::Text {
                text: "a.md".into(),
            }],
        ),
    ]))
    .await;
    assert_eq!(
        body["contents"],
        json!([
            { "role": "user", "parts": [{ "text": "list files" }] },
            { "role": "model", "parts": [{ "functionCall": { "name": "shell_exec", "args": { "command": "ls" }, "id": "call-1" }, "thoughtSignature": "sig-abc" }] },
            { "role": "user", "parts": [{ "functionResponse": { "name": "shell_exec", "id": "call-1", "response": { "output": "a.md" } } }] },
        ])
    );
}

#[tokio::test]
async fn a_tool_call_without_a_signature_of_this_provider_replays_as_text() {
    // History from another provider: Gemini refuses a function call without
    // its own signature, so the exchange survives as text.
    for signature in [
        Some("{\"type\":\"reasoning\",\"encrypted_content\":\"…\"}"),
        Some("google:"),
        None,
    ] {
        let mut items = vec![InferenceItem::UserMessage {
            content: text("list files"),
        }];
        if let Some(signature) = signature {
            items.push(signed(signature));
        }
        items.push(tool_use("call-9", "shell_exec", json!({ "command": "ls" })));
        items.push(tool_result(
            "call-9",
            vec![
                ToolResultContentBlock::Text {
                    text: "a.md".into(),
                },
                ToolResultContentBlock::Image {
                    source: ToolMediaSource::Binary {
                        data: B64Bytes::from(&b"PNG"[..]),
                        media_type: "image/png".into(),
                    },
                },
            ],
        ));
        let body = body_of(request_with(items)).await;
        assert_eq!(
            body["contents"][1],
            json!({ "role": "model", "parts": [{ "text": "[called shell_exec with {\"command\":\"ls\"}]" }] }),
            "{signature:?}"
        );
        assert_eq!(
            body["contents"][2],
            json!({ "role": "user", "parts": [{ "text": "[shell_exec returned] a.md\n[image/png]" }] })
        );
        assert!(!body.to_string().contains("functionCall"));
    }
}

#[tokio::test]
async fn video_rides_inline_and_tool_media_follows_the_function_response() {
    let body = body_of(request_with(vec![
        InferenceItem::UserMessage {
            content: vec![
                UserContentBlock::Text {
                    text: "watch this".into(),
                },
                UserContentBlock::Video {
                    source: MediaSource::Binary {
                        data: B64Bytes::from(&b"VID"[..]),
                        media_type: "video/mp4".into(),
                    },
                },
                UserContentBlock::Image {
                    source: MediaSource::Url {
                        url: "https://example.com/a.png".into(),
                    },
                },
            ],
        },
        signed("google:sig-2"),
        tool_use("call-2", "look", Value::Null),
        tool_result(
            "call-2",
            vec![
                ToolResultContentBlock::Text {
                    text: "rendered".into(),
                },
                ToolResultContentBlock::Image {
                    source: ToolMediaSource::Binary {
                        data: B64Bytes::from(&b"PNG"[..]),
                        media_type: "image/png".into(),
                    },
                },
            ],
        ),
    ]))
    .await;
    assert_eq!(
        body["contents"][0]["parts"],
        json!([
            { "text": "watch this" },
            { "inlineData": { "mimeType": "video/mp4", "data": "VklE" } },
            { "fileData": { "fileUri": "https://example.com/a.png" } },
        ])
    );
    assert_eq!(
        body["contents"][1]["parts"][0]["functionCall"]["args"],
        json!({})
    );
    // A function response holds JSON only, so the picture travels beside it.
    assert_eq!(
        body["contents"][2]["parts"],
        json!([
            { "functionResponse": { "name": "look", "id": "call-2", "response": { "output": "rendered" } } },
            { "inlineData": { "mimeType": "image/png", "data": "UE5H" } },
        ])
    );
}
