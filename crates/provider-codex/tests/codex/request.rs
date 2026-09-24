//! What a Codex request sends: the body as the backend wants a transcript
//! replayed, and the headers that name the account and the session
//! (`models.md` § Request parameters, `providers.md` § Endpoints).

use std::sync::Arc;

use demi_core::{
    B64Bytes, ThinkingConfig, ThinkingSummary, ToolMediaSource, ToolResultContentBlock,
    UserContentBlock,
};
use demi_provider::{
    InferenceItem, InferenceRequest, ToolDefinition,
    openai_request::{prompt_cache_key, short_hash},
    testing::{MockVendor, RecordedRequest, inference_request, jwt},
};
use demi_provider_codex::TransportMode;
use serde_json::{Value, json};

use crate::{NOW, RESPONSES, completed, fresh_token, pool_with, provider, run, runtime_of, secret};

fn text(text: &str) -> Vec<UserContentBlock> {
    vec![UserContentBlock::Text { text: text.into() }]
}

fn shell_tool() -> ToolDefinition {
    ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute shell".into(),
        input_schema: json!({ "type": "object" }).as_object().unwrap().clone(),
    }
}

fn request_with(items: Vec<InferenceItem>) -> InferenceRequest {
    let mut request = inference_request();
    request.model_id = "gpt-5.4".into();
    request.system_prompt = "system".into();
    request.items = items.into();
    request.tools = Arc::new([shell_tool()]);
    request.thinking = Some(ThinkingConfig::Effort {
        effort: "medium".into(),
        summary: None,
    });
    request
}

/// What the backend received for `request`, over server-sent events.
async fn sent(request: InferenceRequest) -> RecordedRequest {
    let vendor = MockVendor::start().await;
    vendor.respond_at(RESPONSES, completed());
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    run(runtime_of(&provider).as_mut(), request).await;
    vendor
        .requests()
        .into_iter()
        .find(|request| request.uri.path() == RESPONSES)
        .unwrap()
}

async fn body_of(request: InferenceRequest) -> Value {
    sent(request).await.json()
}

#[tokio::test]
async fn a_body_replays_the_transcript_with_codexs_items_tools_and_reasoning() {
    let reasoning = json!({ "type": "reasoning", "id": "rs_1", "encrypted_content": "enc", "summary": [{ "text": "summary" }], "status": "completed" });
    let body = body_of(request_with(vec![
        InferenceItem::UserMessage {
            content: text("hello"),
        },
        InferenceItem::UserSteer {
            content: text("steer this turn"),
        },
        InferenceItem::AssistantThinking {
            model_id: "gpt-5.4".into(),
            text: "private".into(),
            signature: Some(format!("codex:{reasoning}")),
        },
        InferenceItem::AssistantText {
            model_id: "gpt-5.4".into(),
            text: "visible".into(),
        },
        InferenceItem::ToolUse {
            model_id: "gpt-5.4".into(),
            tool_use_id: "call_1|fc_1".into(),
            tool_name: "shell_exec".into(),
            input: json!({ "script": "pwd" }),
        },
        InferenceItem::ToolResult {
            tool_use_id: "call_1|fc_1".into(),
            output: vec![ToolResultContentBlock::Text {
                text: "/tmp".into(),
            }],
            is_error: false,
        },
    ]))
    .await;
    let message_id = format!("msg_{}", short_hash("3:gpt-5.4:visible"));
    assert_eq!(
        body,
        json!({
            "model": "gpt-5.4",
            "instructions": "system",
            "input": [
                { "role": "user", "content": [{ "type": "input_text", "text": "hello" }] },
                { "role": "user", "content": [{ "type": "input_text", "text": "steer this turn" }] },
                // Codex takes its reasoning items back whole.
                reasoning,
                { "type": "message", "role": "assistant", "id": message_id, "status": "completed", "content": [{ "type": "output_text", "text": "visible", "annotations": [] }] },
                { "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "shell_exec", "arguments": "{\"script\":\"pwd\"}" },
                { "type": "function_call_output", "call_id": "call_1", "output": "/tmp" },
            ],
            "tools": [{ "type": "function", "name": "shell_exec", "description": "Execute shell", "parameters": { "type": "object" }, "strict": null }],
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "store": false,
            "stream": true,
            "include": ["reasoning.encrypted_content"],
            "prompt_cache_key": "session-1",
            "text": { "verbosity": "low" },
            "reasoning": { "effort": "medium", "summary": "auto" },
        })
    );
}

#[tokio::test]
async fn thinking_signed_by_another_vendor_is_skipped_and_tool_images_ride_inside_the_output() {
    let item = json!({ "type": "reasoning", "encrypted_content": "enc" });
    let mut request = request_with(vec![
        InferenceItem::AssistantThinking {
            model_id: "gpt-5.4".into(),
            text: "unsigned".into(),
            signature: None,
        },
        InferenceItem::AssistantThinking {
            model_id: "gpt-5.4".into(),
            text: "openai".into(),
            signature: Some(format!("openai:{item}")),
        },
        InferenceItem::ToolResult {
            tool_use_id: "call_1|fc_1".into(),
            output: vec![
                ToolResultContentBlock::Text {
                    text: "see image".into(),
                },
                ToolResultContentBlock::Image {
                    source: ToolMediaSource::Binary {
                        data: B64Bytes::from(vec![1, 2, 3]),
                        media_type: "image/png".into(),
                    },
                },
                ToolResultContentBlock::Video {
                    source: ToolMediaSource::Binary {
                        data: B64Bytes::from(vec![4]),
                        media_type: "video/mp4".into(),
                    },
                },
            ],
            is_error: false,
        },
    ]);
    request.system_prompt = String::new();
    request.tools = Arc::new([]);
    let body = body_of(request).await;
    assert_eq!(
        body["input"],
        json!([{
            "type": "function_call_output",
            "call_id": "call_1",
            "output": [
                { "type": "input_text", "text": "see image" },
                { "type": "input_image", "image_url": "data:image/png;base64,AQID", "detail": "auto" },
            ],
        }])
    );
    // The backend requires its instructions and tools even when empty.
    assert_eq!(
        (&body["instructions"], &body["tools"]),
        (&json!(""), &json!([]))
    );
}

#[tokio::test]
async fn the_tier_is_sent_only_when_selected_and_efforts_reach_codex_unchanged() {
    let mut standard = request_with(Vec::new());
    standard.thinking = None;
    let body = body_of(standard.clone()).await;
    assert!(body.get("service_tier").is_none() && body.get("reasoning").is_none());
    standard.service_tier_id = Some("priority".into());
    assert_eq!(body_of(standard).await["service_tier"], json!("priority"));
    for effort in ["low", "xhigh", "max", "ultra"] {
        let mut request = request_with(Vec::new());
        request.thinking = Some(ThinkingConfig::Effort {
            effort: effort.into(),
            summary: None,
        });
        assert_eq!(
            body_of(request).await["reasoning"],
            json!({ "effort": effort, "summary": "auto" })
        );
    }
    // A summary turned off still reaches Codex as `auto`.
    let mut off = request_with(Vec::new());
    off.thinking = Some(ThinkingConfig::Effort {
        effort: "high".into(),
        summary: Some(ThinkingSummary::Off),
    });
    assert_eq!(
        body_of(off).await["reasoning"],
        json!({ "effort": "high", "summary": "auto" })
    );
}

#[tokio::test]
async fn the_headers_name_the_account_the_session_and_the_request() {
    let request = sent(request_with(Vec::new())).await;
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {}", fresh_token()).as_str())
    );
    assert_eq!(request.header("chatgpt-account-id"), Some("acct-1"));
    assert_eq!(
        request.header("openai-beta"),
        Some("responses=experimental")
    );
    assert_eq!(
        (request.header("session-id"), request.header("thread-id")),
        (Some("session-1"), Some("session-1"))
    );
    assert_eq!(request.header("x-client-request-id"), Some("request-1"));
    assert_eq!(request.header("accept"), Some("text/event-stream"));
    assert!(
        request
            .header("user-agent")
            .unwrap()
            .starts_with("demi-codex-provider/")
    );
    assert!(request.header("x-openai-fedramp").is_none());

    // The backend derives the cache key from the session header and refuses
    // one over 64 characters, so a long id arrives clamped like the body's.
    let mut long = request_with(Vec::new());
    long.session_id = format!("chat-{}", "x".repeat(80));
    let clamped = sent(long.clone()).await;
    let key = prompt_cache_key(&long.session_id).into_owned();
    assert!(key.len() <= 64);
    assert_eq!(
        (clamped.header("session-id"), clamped.header("thread-id")),
        (Some(key.as_str()), Some(key.as_str()))
    );
    assert_eq!(clamped.json()["prompt_cache_key"], json!(key));
}

#[tokio::test]
async fn a_fedramp_account_says_so() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(RESPONSES, completed());
    let access = jwt(
        &json!({ "exp": crate::NOW_SECONDS + 3_600, "https://api.openai.com/auth": { "chatgpt_account_id": "acct-1", "chatgpt_account_is_fedramp": true } }),
    );
    let pool = pool_with(secret(&access, "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    run(runtime_of(&provider).as_mut(), inference_request()).await;
    assert_eq!(
        vendor.requests()[0].header("x-openai-fedramp"),
        Some("true")
    );
}
