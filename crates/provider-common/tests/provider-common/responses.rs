//! The Responses stream mapped onto a run's events
//! (`crates/provider-common/src/contract.rs`,
//! `crates/provider-common/src/wire/mod.rs`), over recorded frames.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    ProviderEvent, ToolCall, read_http_failure,
    testing::FixedClock,
    wire::{
        Vendor,
        responses::{map_events, sse_events},
    },
};
use demi_shared_types::{
    Timestamp, TokenUsage,
};
use futures_util::stream;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const NOW: &str = "2026-09-18T14:00:00.000Z";

fn vendor(reader: demi_provider_common::FailureReader) -> Vendor {
    Vendor {
        label: "Codex",
        reader,
        clock: Arc::new(FixedClock(NOW.parse::<Timestamp>().unwrap())),
        model: "gpt-5-codex".into(),
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

async fn events_with(
    frames: &[Value],
    reader: demi_provider_common::FailureReader,
) -> Vec<ProviderEvent> {
    let cancel = CancellationToken::new();
    let events = sse_events(body(frames), "Codex", cancel.clone());
    demi_provider_common::testing::all_events(map_events(events, vendor(reader), "codex:".into(), cancel)).await
}

async fn events(frames: &[Value]) -> Vec<ProviderEvent> {
    events_with(frames, read_http_failure).await
}

#[tokio::test]
async fn a_stream_maps_thinking_text_tool_calls_and_usage() {
    let reasoning = json!({
        "type": "reasoning", "id": "rs_1", "encrypted_content": "enc",
        "summary": [{ "type": "summary_text", "text": "thought" }], "status": "completed",
    });
    let events = events(&[
        json!({ "type": "response.created", "response": { "id": "resp-1" } }),
        json!({ "type": "response.output_item.added", "item": { "type": "reasoning", "id": "rs_1" } }),
        json!({ "type": "response.reasoning_summary_text.delta", "delta": "think", "sequence_number": 3 }),
        json!({ "type": "response.output_item.done", "item": reasoning }),
        json!({ "type": "response.output_text.delta", "delta": "hello " }),
        json!({ "type": "response.output_item.added", "item": { "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "shell_exec", "arguments": "" } }),
        json!({ "type": "response.function_call_arguments.delta", "item_id": "fc_1", "delta": "{\"script\":" }),
        json!({ "type": "response.function_call_arguments.done", "item_id": "fc_1", "arguments": "{\"script\":\"pwd\"}" }),
        json!({ "type": "response.output_item.done", "item": { "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "shell_exec" } }),
        json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 100, "output_tokens": 7, "input_tokens_details": { "cached_tokens": 60, "cache_write_tokens": 25 } } } }),
        json!("[DONE]"),
    ])
    .await;
    let [
        start,
        delta,
        ProviderEvent::ThinkingSignature(signature),
        text,
        call_start,
        call_input,
        call,
        response,
    ] = events.as_slice()
    else {
        panic!("{events:?}");
    };
    // The call as the model writes it, for display, then whole.
    assert_eq!(
        (call_start, call_input),
        (
            &ProviderEvent::ToolCallStart {
                tool_use_id: "call_1|fc_1".into(),
                tool_name: "shell_exec".into(),
            },
            &ProviderEvent::ToolCallInput {
                tool_use_id: "call_1|fc_1".into(),
                partial_json: "{\"script\":".into(),
            }
        )
    );
    assert_eq!(
        (start, delta, text),
        (
            &ProviderEvent::ThinkingStart,
            &ProviderEvent::ThinkingDelta("think".into()),
            &ProviderEvent::TextDelta("hello ".into())
        )
    );
    // The signature is the whole item, the fields Demi does not read
    // included, tagged as this provider's.
    let item: Value = serde_json::from_str(signature.strip_prefix("codex:").unwrap()).unwrap();
    assert_eq!(item, reasoning);
    assert_eq!(
        call,
        &ProviderEvent::ToolCall(ToolCall {
            tool_use_id: "call_1|fc_1".into(),
            tool_name: "shell_exec".into(),
            input: json!({ "script": "pwd" }),
        })
    );
    // The input count includes what the API read from its cache and what
    // it wrote to it; Demi keeps the three apart.
    let usage = TokenUsage {
        input_tokens: 15,
        output_tokens: 7,
        cache_read_tokens: 60,
        cache_write_tokens: 25,
    };
    assert_eq!(response, &ProviderEvent::Response(Some(usage)));
}

#[tokio::test]
async fn a_finished_items_text_is_emitted_only_when_no_delta_streamed_it() {
    let message = |text: &str| json!({ "type": "response.output_item.done", "item": { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": text, "annotations": [] }, { "type": "output_audio", "data": "…" }] } });
    let events = events(&[
        json!({ "type": "response.output_item.added", "item": { "type": "reasoning", "id": "rs_1" } }),
        json!({ "type": "response.output_item.done", "item": { "type": "reasoning", "id": "rs_1", "content": [{ "text": "raw reasoning" }], "encrypted_content": "enc" } }),
        json!({ "type": "response.output_text.delta", "delta": "因为" }),
        json!({ "type": "response.output_text.delta", "delta": "天空是蓝的" }),
        message("因为天空是蓝的"),
        message("and the next message"),
        json!({ "type": "response.output_item.done", "item": { "type": "web_search_call", "status": "completed" } }),
        json!({ "type": "response.output_item.done", "item": { "type": "message", "content": [{ "type": "refusal", "refusal": "no" }] } }),
        json!({ "type": "response.completed" }),
    ])
    .await;
    let texts: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            ProviderEvent::TextDelta(text) | ProviderEvent::ThinkingDelta(text) => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        [
            "raw reasoning",
            "因为",
            "天空是蓝的",
            "and the next message",
            "no"
        ]
    );
    // A completion without usage gives a response without usage, never
    // zeros.
    assert_eq!(events.last(), Some(&ProviderEvent::Response(None)));
}
