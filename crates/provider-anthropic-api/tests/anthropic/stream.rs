//! The events a run makes of recorded Messages API event streams.

use demi_provider_common::{
    ErrorCode, ProviderEvent, ToolCall,
    testing::{MockResponse, MockVendor, inference_request},
};
use demi_shared_types::{FailureSource, TokenUsage};
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
        // The delta's counts are the call's: one it leaves out, or states
        // as null, is message_start's; one it states, even 0, is its own.
        json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": { "input_tokens": null, "cache_creation_input_tokens": 0, "output_tokens": 5 } }),
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
            // The call as the model writes it, for display, then whole.
            ProviderEvent::ToolCallStart {
                tool_use_id: "toolu_1".into(),
                tool_name: "read_file".into(),
            },
            ProviderEvent::ToolCallInput {
                tool_use_id: "toolu_1".into(),
                partial_json: "{\"path\":".into(),
            },
            ProviderEvent::ToolCallInput {
                tool_use_id: "toolu_1".into(),
                partial_json: "\"a.ts\"}".into(),
            },
            ProviderEvent::ToolCall(ToolCall {
                tool_use_id: "toolu_1".into(),
                tool_name: "read_file".into(),
                input: json!({ "path": "a.ts" }),
            }),
            ProviderEvent::Response(Some(TokenUsage {
                input_tokens: 12,
                output_tokens: 5,
                cache_read_tokens: 2,
                cache_write_tokens: 0,
            })),
        ]
    );
}

#[tokio::test]
async fn a_tool_input_comes_from_its_deltas_else_its_start_else_is_an_empty_object() {
    let events = events_of(recorded(&[
        block_start(
            0,
            json!({ "type": "tool_use", "id": "t0", "name": "a", "input": { "q": 1 } }),
        ),
        block_stop(0),
        block_start(1, json!({ "type": "tool_use", "id": "t1", "name": "b" })),
        block_stop(1),
        block_start(
            2,
            json!({ "type": "tool_use", "id": "t2", "name": "c", "input": {} }),
        ),
        delta(
            2,
            json!({ "type": "input_json_delta", "partial_json": "{\"path\":" }),
        ),
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
async fn a_complete_stop_reason_ends_the_run_with_its_response_and_any_other_fails() {
    // The model finished, so the calls of the reply are the turn's calls.
    for reason in [Some("end_turn"), Some("tool_use"), Some("stop_sequence"), None] {
        let events = events_of(recorded(&[
            delta(0, json!({ "type": "text_delta", "text": "done" })),
            json!({ "type": "message_delta", "delta": { "stop_reason": reason }, "usage": { "output_tokens": 2 } }),
            message_stop(),
        ]))
        .await;
        let usage = TokenUsage {
            output_tokens: 2,
            ..TokenUsage::default()
        };
        assert_eq!(
            events,
            [
                ProviderEvent::TextDelta("done".into()),
                ProviderEvent::Response(Some(usage))
            ],
            "{reason:?}"
        );
    }
    // The vendor cut the reply short: a failure with its reason, after the
    // text that streamed.
    for (reason, message) in [
        ("max_tokens", "The reply reached the model's output limit (max_tokens)"),
        ("model_context_window_exceeded", "The reply reached the model's output limit (model_context_window_exceeded)"),
        ("refusal", "The provider's filter stopped the reply (refusal)"),
        ("pause_turn", "The provider ended the reply before it was complete (pause_turn)"),
    ] {
        let stopped = json!({ "type": "message_delta", "delta": { "stop_reason": reason }, "usage": { "output_tokens": 2 } });
        let events = events_of(recorded(&[
            delta(0, json!({ "type": "text_delta", "text": "half" })),
            stopped.clone(),
            message_stop(),
        ]))
        .await;
        let [ProviderEvent::TextDelta(text), ProviderEvent::Error(failure)] = &events[..] else {
            panic!("{reason}: {events:?}");
        };
        assert_eq!(text, "half");
        assert_eq!((failure.message.as_str(), &failure.code), (message, &Some(ErrorCode::Incomplete)));
        let diagnostics = failure.diagnostics.as_ref().unwrap();
        assert_eq!(
            (diagnostics.source, diagnostics.provider_code.as_deref(), diagnostics.upstream.clone()),
            (FailureSource::Stream, Some(reason), Some(stopped.to_string()))
        );
    }
}
