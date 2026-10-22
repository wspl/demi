//! What a Codex request sends: the body as the backend wants a transcript
//! replayed, and the headers that name the account and the session
//! (`crates/shared-types/src/model.rs`, `crates/provider-codex/src/transport.rs`).

use std::sync::Arc;

use demi_provider_codex::TransportMode;
use demi_provider_common::{
    InferenceItem, InferenceRequest, MediaBytes, ResultPart, ToolDefinition, UserPart,
    openai_request::short_hash,
    testing::{MockVendor, RecordedRequest, inference_request},
};
use demi_shared_types::{B64Bytes, ThinkingConfig};
use serde_json::{Value, json};

use crate::{NOW, RESPONSES, completed, fresh_token, pool_with, provider, run, runtime_of, secret};

fn text(text: &str) -> Vec<UserPart> {
    vec![UserPart::Text(text.into())]
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
            kept_past_summary: false,
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
            output: vec![ResultPart::Text("/tmp".into())],
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
            kept_past_summary: false,
        },
        InferenceItem::AssistantThinking {
            model_id: "gpt-5.4".into(),
            text: "openai".into(),
            signature: Some(format!("openai:{item}")),
            kept_past_summary: false,
        },
        InferenceItem::ToolResult {
            tool_use_id: "call_1|fc_1".into(),
            output: vec![
                ResultPart::Text("see image".into()),
                ResultPart::Image(MediaBytes {
                    data: B64Bytes::from(vec![1, 2, 3]),
                    media_type: "image/png".into(),
                }),
                ResultPart::Video(MediaBytes {
                    data: B64Bytes::from(vec![4]),
                    media_type: "video/mp4".into(),
                }),
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
