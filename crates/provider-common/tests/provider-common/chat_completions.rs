//! The Chat Completions stream mapped onto a run's events
//! (`crates/provider-common/src/contract.rs`,
//! `crates/backend-providers/src/llm/vendors.rs`), over recorded frames.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    ProviderEvent, ToolCall, read_http_failure,
    testing::FixedClock,
    wire::{Vendor, chat_completions::map_sse},
};
use demi_shared_types::{Timestamp, TokenUsage};
use futures_util::stream;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn vendor() -> Vendor {
    Vendor {
        label: "Grok Build",
        reader: read_http_failure,
        clock: Arc::new(FixedClock(
            "2026-09-18T14:00:00.000Z".parse::<Timestamp>().unwrap(),
        )),
        model: "grok-code".into(),
    }
}

/// A body with one `data:` frame per payload; a string is sent as it is.
fn body(frames: &[Value]) -> impl futures_util::Stream<Item = reqwest::Result<Bytes>> + 'static {
    let text: String = frames
        .iter()
        .map(|frame| match frame {
            Value::String(text) => format!("data: {text}\n\n"),
            other => format!("data: {other}\n\n"),
        })
        .collect();
    stream::iter([Ok(Bytes::from(text))])
}

async fn events(frames: &[Value]) -> Vec<ProviderEvent> {
    demi_provider_common::testing::all_events(map_sse(body(frames), vendor(), CancellationToken::new())).await
}

fn call(id: &str, name: &str, input: Value) -> ProviderEvent {
    ProviderEvent::ToolCall(ToolCall {
        tool_use_id: id.into(),
        tool_name: name.into(),
        input,
    })
}

/// The events without the starts and input pieces of calls being written.
fn whole(events: Vec<ProviderEvent>) -> Vec<ProviderEvent> {
    events
        .into_iter()
        .filter(|event| {
            !matches!(
                event,
                ProviderEvent::ToolCallStart { .. } | ProviderEvent::ToolCallInput { .. }
            )
        })
        .collect()
}

#[tokio::test]
async fn a_stream_maps_split_text_tool_call_arguments_and_usage() {
    let events = events(&[
        // The null spellings vendors stream mean nothing in this chunk.
        json!({ "id": "chatcmpl-1", "usage": null, "choices": [{ "index": 0, "delta": { "content": null }, "finish_reason": null }] }),
        json!({ "choices": [{ "delta": { "content": "hi ", "tool_calls": [{ "index": 0, "id": "call-1", "function": { "name": "read_file", "arguments": "{\"path\"" } }] } }] }),
        json!({ "choices": [{ "delta": { "content": "there", "tool_calls": [{ "index": 0, "function": { "arguments": ":\"a.ts\"}" } }] } }] }),
        json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }] }),
        json!({ "choices": [], "usage": { "prompt_tokens": 12, "completion_tokens": 5, "prompt_tokens_details": { "cached_tokens": 2 } } }),
        json!("[DONE]"),
        json!({ "choices": [{ "delta": { "content": "never read" } }] }),
    ])
    .await;
    let usage = TokenUsage {
        input_tokens: 10,
        output_tokens: 5,
        cache_read_tokens: 2,
        cache_write_tokens: 0,
    };
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("hi ".into()),
            // The call as the model writes it, for display, then whole.
            ProviderEvent::ToolCallStart {
                tool_use_id: "call-1".into(),
                tool_name: "read_file".into(),
            },
            ProviderEvent::ToolCallInput {
                tool_use_id: "call-1".into(),
                partial_json: "{\"path\"".into(),
            },
            ProviderEvent::TextDelta("there".into()),
            ProviderEvent::ToolCallInput {
                tool_use_id: "call-1".into(),
                partial_json: ":\"a.ts\"}".into(),
            },
            call("call-1", "read_file", json!({ "path": "a.ts" })),
            ProviderEvent::Response(Some(usage)),
        ]
    );
}

#[tokio::test]
async fn tool_calls_assemble_by_index_and_flush_when_the_reply_is_complete() {
    let assembled = whole(events(&[
        // Arguments that are not JSON stay the string the vendor sent.
        json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 1, "id": "call-2", "function": { "name": "bad", "arguments": "{" } }] } }] }),
        json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "function": { "name": "no_id" } }] } }] }),
        // A call that never names its tool is dropped.
        json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 2, "id": "call-3", "function": { "arguments": "{}" } }] } }] }),
        // Gateways leave out the closing `[DONE]`.
        json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }] }),
    ])
    .await);
    assert_eq!(
        assembled,
        [
            call("tool_call_0", "no_id", json!({})),
            call("call-2", "bad", json!("{")),
            ProviderEvent::Response(None),
        ]
    );
    // A vendor that omits `index` sends one call at a time, in order.
    let sequential = whole(events(&[
        json!({ "choices": [{ "delta": { "tool_calls": [{ "id": "a", "function": { "name": "first", "arguments": "{}" } }] } }] }),
        json!({ "choices": [{ "delta": { "tool_calls": [{ "id": "b", "function": { "name": "second", "arguments": "{}" } }] } }] }),
        json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] }),
        json!("[DONE]"),
    ])
    .await);
    assert_eq!(
        sequential[..2],
        [
            call("a", "first", json!({})),
            call("b", "second", json!({}))
        ]
    );
}
