//! What a Grok Build run sends and what it makes of the stream
//! (`crates/provider-grok-build/src/releases.rs`, `crates/shared-types/src/model.rs`).

use std::sync::Arc;

use demi_provider_common::{
    ErrorCode, InferenceItem, InferenceRequest, MediaBytes, Medium, ProviderEvent, ResultPart, ToolDefinition, UserPart,
    testing::{MockResponse, MockVendor, inference_request},
};
use demi_shared_types::{B64Bytes, ThinkingConfig};
use serde_json::{Value, json};

use crate::{CHAT, chat, pool_with, provider, run, runtime_of, secret};

async fn exchange(
    request: InferenceRequest,
    response: MockResponse,
) -> (Vec<ProviderEvent>, MockVendor) {
    let vendor = MockVendor::start().await;
    vendor.respond_at(CHAT, response);
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(crate::ACCOUNT));
    let events = run(runtime_of(&provider).as_mut(), request).await;
    (events, vendor)
}

async fn body_of(request: InferenceRequest) -> Value {
    exchange(request, chat(&[])).await.1.requests()[0].json()
}

#[tokio::test]
async fn the_chat_body_carries_the_tools_the_replay_and_the_effort_but_no_limit_or_tier() {
    let mut request = inference_request();
    request.model_id = "grok-4.5".into();
    request.system_prompt = "system".into();
    request.output_limit = std::num::NonZeroU32::new(8_000);
    request.service_tier_id = Some("priority".into());
    request.thinking = Some(ThinkingConfig::Effort {
        effort: "high".into(),
        summary: None,
    });
    request.tools = Arc::new([ToolDefinition {
        name: "read_file".into(),
        description: "Read a file".into(),
        input_schema: json!({ "type": "object", "properties": { "path": { "type": "string" } } })
            .as_object()
            .unwrap()
            .clone(),
    }]);
    request.items = Arc::new([
        InferenceItem::UserMessage {
            content: vec![UserPart::Text("hello".into())],
        },
        InferenceItem::AssistantThinking {
            model_id: "grok-4.5".into(),
            text: "hidden".into(),
            signature: None,
            kept_past_summary: false,
        },
        InferenceItem::AssistantText {
            model_id: "grok-4.5".into(),
            text: "Use tool".into(),
        },
        InferenceItem::ToolUse {
            model_id: "grok-4.5".into(),
            tool_use_id: "call-1".into(),
            tool_name: "read_file".into(),
            input: json!({ "path": "a.ts" }),
        },
        InferenceItem::ToolResult {
            tool_use_id: "call-1".into(),
            output: vec![
                ResultPart::Text("contents".into()),
                ResultPart::Image(MediaBytes {
                    data: B64Bytes::from(&b"PNG"[..]),
                    media_type: "image/png".into(),
                }),
            ],
            is_error: false,
        },
    ]);
    assert_eq!(
        body_of(request).await,
        json!({
            "model": "grok-4.5",
            "messages": [
                { "role": "system", "content": "system" },
                { "role": "user", "content": "hello" },
                { "role": "assistant", "content": "Use tool", "tool_calls": [{ "id": "call-1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"a.ts\"}" } }] },
                // The proxy's tool messages are text; media is named.
                { "role": "tool", "tool_call_id": "call-1", "content": "contents\n[image:image/png]" },
            ],
            "stream": true,
            "stream_options": { "include_usage": true },
            "tools": [{ "type": "function", "function": { "name": "read_file", "description": "Read a file", "parameters": { "type": "object", "properties": { "path": { "type": "string" } } } } }],
            "tool_choice": "auto",
            "reasoning_effort": "high",
        })
    );
}

#[tokio::test]
async fn video_becomes_text_and_a_pdf_is_left_to_its_attachment_tag() {
    let mut request = inference_request();
    request.items = Arc::new([InferenceItem::UserMessage {
        content: vec![
            UserPart::Text("look".into()),
            UserPart::Video(Medium::Bytes(MediaBytes {
                data: B64Bytes::from(vec![1, 2]),
                media_type: "video/mp4".into(),
            })),
            UserPart::Document {
                bytes: MediaBytes {
                    data: B64Bytes::from(&b"%PDF"[..]),
                    media_type: "application/pdf".into(),
                },
                file_name: "a.pdf".into(),
            },
            UserPart::Image(Medium::Url("https://example.com/shot.png".into())),
        ],
    }]);
    assert_eq!(
        body_of(request).await["messages"][0],
        json!({ "role": "user", "content": [
            { "type": "text", "text": "look" },
            { "type": "text", "text": "[video:video/mp4]" },
            { "type": "image_url", "image_url": { "url": "https://example.com/shot.png", "detail": "auto" } },
        ] })
    );
}

/// Grok Build's signs of a used-up account, as the Grok CLI reads them, are
/// usage limits; any other 403 is a refused request and a 429 throttles.
/// One mock vendor per case, about 10 ms each.
#[tokio::test]
async fn a_used_up_account_is_a_usage_limit_and_a_429_throttles() {
    let cases = [
        (402, "Grok Build usage balance exhausted", ErrorCode::UsageLimit),
        (402, "", ErrorCode::UsageLimit),
        (403, "status 403: Run out of credits", ErrorCode::UsageLimit),
        (403, "Forbidden", ErrorCode::PermissionDenied),
        (429, "slow down", ErrorCode::RateLimit),
    ];
    for (status, body, code) in cases {
        let (events, _vendor) =
            exchange(inference_request(), MockResponse::status(status).chunk(body)).await;
        let [ProviderEvent::Error(failure)] = events.as_slice() else {
            panic!("{events:?}");
        };
        assert_eq!(failure.code, Some(code), "{status} {body}");
    }
}
