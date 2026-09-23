//! What a Grok Build run sends and what it makes of the stream
//! (`providers.md` § Endpoints, `models.md` § Request parameters).

use std::sync::Arc;

use demi_core::{
    B64Bytes, DocumentSource, MediaSource, ThinkingConfig, TokenUsage, ToolMediaSource,
    ToolResultContentBlock, UserContentBlock,
};
use demi_provider::{
    ErrorCode, InferenceItem, InferenceRequest, ProviderEvent, ToolCall, ToolDefinition,
    testing::{MockResponse, MockVendor, inference_request},
};
use futures_util::StreamExt;
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
async fn a_run_posts_to_the_chat_proxy_with_the_grok_clis_identity() {
    let mut request = inference_request();
    request.model_id = "grok-4.5".into();
    let (events, vendor) = exchange(
        request,
        chat(&[json!({ "choices": [{ "delta": { "content": "hi" } }] })]),
    )
    .await;
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("hi".into()),
            ProviderEvent::Response(TokenUsage::default())
        ]
    );
    let sent = &vendor.requests()[0];
    assert_eq!((sent.method.as_str(), sent.uri.path()), ("POST", CHAT));
    let expected = [
        ("authorization", "Bearer session-token"),
        ("x-xai-token-auth", "xai-grok-cli"),
        ("x-authenticateresponse", "authenticate-response"),
        ("x-grok-client-identifier", "grok-shell"),
        ("x-grok-client-mode", "interactive"),
        ("x-grok-client-version", "1.0.5"),
        ("x-userid", "user-1"),
        ("x-grok-user-id", "user-1"),
        ("x-email", "user@example.com"),
        ("x-grok-model-override", "grok-4.5"),
        ("x-grok-session-id", "session-1"),
        ("x-grok-conv-id", "session-1"),
        ("x-grok-req-id", "request-1"),
        ("x-grok-turn-idx", "turn-1"),
        ("accept", "text/event-stream"),
    ];
    for (name, value) in expected {
        assert_eq!(sent.header(name), Some(value), "{name}");
    }
    assert!(sent.header("x-grok-client-surface").is_none());
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
            content: vec![UserContentBlock::Text {
                text: "hello".into(),
            }],
        },
        InferenceItem::AssistantThinking {
            model_id: "grok-4.5".into(),
            text: "hidden".into(),
            signature: None,
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
                ToolResultContentBlock::Text {
                    text: "contents".into(),
                },
                ToolResultContentBlock::Image {
                    source: ToolMediaSource::Binary {
                        data: B64Bytes::from(&b"PNG"[..]),
                        media_type: "image/png".into(),
                    },
                },
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
            UserContentBlock::Text {
                text: "look".into(),
            },
            UserContentBlock::Video {
                source: MediaSource::Binary {
                    data: B64Bytes::from(vec![1, 2]),
                    media_type: "video/mp4".into(),
                },
            },
            UserContentBlock::Document {
                source: DocumentSource::Binary {
                    data: B64Bytes::from(&b"%PDF"[..]),
                    media_type: "application/pdf".into(),
                    file_name: "a.pdf".into(),
                },
            },
            UserContentBlock::Image {
                source: MediaSource::Url {
                    url: "https://example.com/shot.png".into(),
                },
            },
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

#[tokio::test]
async fn the_stream_maps_reasoning_tool_calls_and_usage_by_the_shared_mapper() {
    let (events, _vendor) = exchange(
        inference_request(),
        chat(&[
            json!({ "choices": [{ "delta": { "reasoning_content": "think" } }] }),
            json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": "c1", "function": { "name": "shell_exec", "arguments": "{\"cmd\"" } }] } }] }),
            json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "function": { "arguments": ":\"ls\"}" } }] }, "finish_reason": "tool_calls" }] }),
            json!({ "choices": [], "usage": { "prompt_tokens": 5, "completion_tokens": 2 } }),
        ]),
    )
    .await;
    assert_eq!(
        events,
        [
            ProviderEvent::ThinkingStart,
            ProviderEvent::ThinkingDelta("think".into()),
            ProviderEvent::ToolCall(ToolCall {
                tool_use_id: "c1".into(),
                tool_name: "shell_exec".into(),
                input: json!({ "cmd": "ls" })
            }),
            ProviderEvent::Response(TokenUsage {
                input_tokens: 5,
                output_tokens: 2,
                cache_read_tokens: 0,
                cache_write_tokens: 0
            }),
        ]
    );
}

#[tokio::test]
async fn a_refused_request_fails_with_the_vendor_record() {
    let (events, _vendor) = exchange(
        inference_request(),
        MockResponse::status(429)
            .header("retry-after", "7")
            .chunk("slow down"),
    )
    .await;
    let [ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(
        failure.message,
        "Grok Build API request failed with HTTP 429: slow down"
    );
    assert_eq!(
        (failure.code.clone(), failure.retry_after),
        (
            Some(ErrorCode::RateLimit),
            Some(std::time::Duration::from_secs(7))
        )
    );
}

#[tokio::test]
async fn cancelling_mid_stream_ends_the_run_without_an_event_and_drops_the_connection() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(
        CHAT,
        MockResponse::event_stream("data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n")
            .stay_open(),
    );
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(crate::ACCOUNT));
    let mut runtime = runtime_of(&provider);
    let request = inference_request();
    let cancel = request.cancel.clone();
    let mut events = runtime.run(request);
    assert_eq!(
        events.next().await,
        Some(ProviderEvent::TextDelta("hel".into()))
    );
    cancel.cancel();
    assert_eq!(events.next().await, None);
    vendor.disconnected().await;
}
