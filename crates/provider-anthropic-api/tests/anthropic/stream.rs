//! The events a run makes of recorded Messages API event streams.

use demi_core::{FailureSource, TokenUsage};
use demi_provider::{
    ErrorCode, ProviderEvent, ToolCall,
    testing::{MockResponse, MockVendor, inference_request},
};
use serde_json::{Value, json};

use crate::{recorded, run, runtime};

async fn events_of(response: MockResponse) -> Vec<ProviderEvent> {
    let vendor = MockVendor::start().await;
    vendor.respond(response);
    run(runtime(&vendor).as_mut(), inference_request()).await
}

fn block_start(index: u32, block: Value) -> Value {
    json!({ "type": "content_block_start", "index": index, "content_block": block })
}

fn delta(index: u32, delta: Value) -> Value {
    json!({ "type": "content_block_delta", "index": index, "delta": delta })
}

fn block_stop(index: u32) -> Value {
    json!({ "type": "content_block_stop", "index": index })
}

fn message_stop() -> Value {
    json!({ "type": "message_stop" })
}

#[tokio::test]
async fn a_stream_maps_thinking_text_tool_use_and_usage() {
    let events = events_of(recorded(&[
        json!({ "type": "message_start", "message": {
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-4-8", "content": [],
            "usage": { "input_tokens": 12, "cache_read_input_tokens": 2, "cache_creation_input_tokens": 1, "output_tokens": 1 },
        } }),
        block_start(0, json!({ "type": "thinking", "thinking": "", "signature": "" })),
        json!({ "type": "ping" }),
        delta(0, json!({ "type": "thinking_delta", "thinking": "plan" })),
        delta(0, json!({ "type": "signature_delta", "signature": "sig" })),
        block_stop(0),
        block_start(1, json!({ "type": "text", "text": "" })),
        delta(1, json!({ "type": "text_delta", "text": "hello" })),
        block_stop(1),
        block_start(2, json!({ "type": "tool_use", "id": "toolu_1", "name": "read_file", "input": {} })),
        delta(2, json!({ "type": "input_json_delta", "partial_json": "{\"path\":" })),
        delta(2, json!({ "type": "input_json_delta", "partial_json": "\"a.ts\"}" })),
        block_stop(2),
        // A count reported as zero keeps the one reported before.
        json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": { "input_tokens": 0, "output_tokens": 5 } }),
        message_stop(),
    ]))
    .await;
    assert_eq!(
        events,
        [
            ProviderEvent::ThinkingStart,
            ProviderEvent::ThinkingDelta("plan".into()),
            ProviderEvent::ThinkingSignature("anthropic:sig".into()),
            ProviderEvent::TextDelta("hello".into()),
            ProviderEvent::ToolCall(ToolCall {
                tool_use_id: "toolu_1".into(),
                tool_name: "read_file".into(),
                input: json!({ "path": "a.ts" }),
            }),
            ProviderEvent::Response(TokenUsage {
                input_tokens: 12,
                output_tokens: 5,
                cache_read_tokens: 2,
                cache_write_tokens: 1,
            }),
        ]
    );
}

#[tokio::test]
async fn a_tool_input_comes_from_its_deltas_else_its_start_else_is_an_empty_object() {
    let events = events_of(recorded(&[
        block_start(0, json!({ "type": "tool_use", "id": "t0", "name": "a", "input": { "q": 1 } })),
        block_stop(0),
        block_start(1, json!({ "type": "tool_use", "id": "t1", "name": "b" })),
        block_stop(1),
        block_start(2, json!({ "type": "tool_use", "id": "t2", "name": "c", "input": {} })),
        delta(2, json!({ "type": "input_json_delta", "partial_json": "{\"path\":" })),
        block_stop(2),
        message_stop(),
    ]))
    .await;
    let inputs: Vec<Value> = events
        .into_iter()
        .filter_map(|event| match event {
            ProviderEvent::ToolCall(call) => Some(call.input),
            _ => None,
        })
        .collect();
    // Streamed text that is not JSON is kept as the string the vendor sent.
    assert_eq!(inputs, [json!({ "q": 1 }), json!({}), json!("{\"path\":")]);
}

#[tokio::test]
async fn redacted_thinking_is_kept_as_received() {
    let events = events_of(recorded(&[
        block_start(0, json!({ "type": "redacted_thinking", "data": "EmwKAhgB" })),
        block_stop(0),
        message_stop(),
    ]))
    .await;
    assert_eq!(events[0], ProviderEvent::RedactedThinking("anthropic:EmwKAhgB".into()));
}

#[tokio::test]
async fn a_stream_that_ends_without_message_stop_still_responds_with_its_usage() {
    let events = events_of(recorded(&[
        json!({ "type": "message_start", "message": { "usage": { "input_tokens": 7 } } }),
        delta(0, json!({ "type": "text_delta", "text": "partial" })),
    ]))
    .await;
    let usage = TokenUsage { input_tokens: 7, ..TokenUsage::default() };
    assert_eq!(events, [ProviderEvent::TextDelta("partial".into()), ProviderEvent::Response(usage)]);
}

#[tokio::test]
async fn events_and_blocks_the_provider_does_not_map_are_skipped() {
    let events = events_of(MockResponse::event_stream(concat!(
        ": keep-alive\n\n",
        "event: ping\ndata: {\"type\":\"ping\"}\n\n",
        "data:\n\n",
        "data: {\"type\":\"message_flavour\",\"flavour\":\"new\"}\n\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"server_tool_use\",\"id\":\"s1\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"citations_delta\"}}\n\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    )))
    .await;
    assert_eq!(events, [ProviderEvent::Response(TokenUsage::default())]);
}

#[tokio::test]
async fn a_malformed_known_event_is_a_protocol_failure_that_names_its_field() {
    let cases = [
        (delta_without_index(), "index"),
        (block_start(0, json!({ "type": "text", "text": 42 })), "text"),
        (block_start(0, json!({ "type": "tool_use", "id": "", "name": "a" })), "id"),
        (json!({ "type": "message_delta", "usage": { "output_tokens": 1.5 } }), "output_tokens"),
    ];
    for (frame, field) in cases {
        let events = events_of(recorded(&[frame.clone(), message_stop()])).await;
        let [ProviderEvent::Error(failure)] = events.as_slice() else {
            panic!("{frame}: {events:?}");
        };
        assert_eq!(failure.code, None, "{frame}");
        assert!(failure.message.contains(field), "{field}: {}", failure.message);
        let diagnostics = failure.diagnostics.as_ref().unwrap();
        assert_eq!(diagnostics.source, FailureSource::Stream);
        assert_eq!(diagnostics.upstream.as_deref(), Some(frame.to_string().as_str()));
    }
}

fn delta_without_index() -> Value {
    json!({ "type": "content_block_delta", "delta": { "type": "text_delta", "text": "hello" } })
}

#[tokio::test]
async fn an_error_event_ends_the_run_with_its_classified_failure() {
    let overloaded = json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } });
    let events = events_of(recorded(&[
        delta(0, json!({ "type": "text_delta", "text": "partial" })),
        overloaded.clone(),
        message_stop(),
    ]))
    .await;
    let [ProviderEvent::TextDelta(_), ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!((failure.message.as_str(), failure.code.clone()), ("Overloaded", Some(ErrorCode::Overloaded)));
    assert_eq!(failure.retry_after, None);
    let diagnostics = failure.diagnostics.as_ref().unwrap();
    assert_eq!(diagnostics.source, FailureSource::Stream);
    assert_eq!(diagnostics.provider_code.as_deref(), Some("overloaded_error"));
    assert_eq!(diagnostics.upstream.as_deref(), Some(overloaded.to_string().as_str()));

    let cases = [
        (json!({ "type": "error", "error": { "type": "rate_limit_error", "message": "Number of requests exceeded" } }), "Number of requests exceeded", Some(ErrorCode::RateLimit)),
        (json!({ "type": "error", "error": { "type": "invalid_request_error", "message": 42 } }), "Anthropic API stream error", Some(ErrorCode::Vendor("invalid_request_error".into()))),
        (json!({ "type": "error", "message": "stream closed by the service" }), "stream closed by the service", Some(ErrorCode::Vendor("error".into()))),
    ];
    for (frame, message, code) in cases {
        let events = events_of(recorded(&[frame.clone()])).await;
        let [ProviderEvent::Error(failure)] = events.as_slice() else {
            panic!("{frame}: {events:?}");
        };
        assert_eq!((failure.message.as_str(), failure.code.clone()), (message, code), "{frame}");
    }
}
