//! The events a run makes of Gemini's stream
//! (`crates/provider-common/src/contract.rs`,
//! `crates/provider-common/src/endpoint.rs`).

use demi_provider_common::{
    ProviderEvent, ToolCall,
    testing::{MockResponse, MockVendor, inference_request},
};
use demi_shared_types::TokenUsage;
use serde_json::{Value, json};

use crate::{chunks, run, runtime};

async fn events_of(response: MockResponse) -> Vec<ProviderEvent> {
    let vendor = MockVendor::start().await;
    vendor.respond(response);
    run(runtime(&vendor).as_mut(), inference_request()).await
}

fn parts(parts: Value) -> Value {
    json!({ "candidates": [{ "content": { "parts": parts } }] })
}

#[tokio::test]
async fn thought_parts_stream_as_thinking_and_a_function_call_emits_its_signature_first() {
    let events = events_of(chunks(&[
        parts(json!([{ "text": "weighing options", "thought": true }])),
        parts(json!([{ "functionCall": { "name": "shell_exec", "args": { "command": "ls" }, "id": "c1" }, "thoughtSignature": "sig-1" }])),
        parts(json!([{ "text": "done" }])),
        json!({ "candidates": [{ "finishReason": "STOP" }], "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 4, "thoughtsTokenCount": 20, "cachedContentTokenCount": 3 } }),
    ]))
    .await;
    // Thinking is billed apart from the answer, and both are output; the
    // prompt count includes what the vendor read from its cache.
    let usage = TokenUsage {
        input_tokens: 7,
        output_tokens: 24,
        cache_read_tokens: 3,
        cache_write_tokens: 0,
    };
    assert_eq!(
        events,
        [
            ProviderEvent::ThinkingStart,
            ProviderEvent::ThinkingDelta("weighing options".into()),
            ProviderEvent::ThinkingSignature("google:sig-1".into()),
            ProviderEvent::ToolCall(ToolCall {
                tool_use_id: "c1".into(),
                tool_name: "shell_exec".into(),
                input: json!({ "command": "ls" }),
            }),
            ProviderEvent::TextDelta("done".into()),
            ProviderEvent::Response(Some(usage)),
        ]
    );
}

#[tokio::test]
async fn a_call_without_an_id_gets_a_new_unique_one() {
    let call = || parts(json!([{ "functionCall": { "name": "shell_exec", "args": {} } }]));
    let ids = |events: Vec<ProviderEvent>| -> Vec<String> {
        events
            .into_iter()
            .filter_map(|event| match event {
                ProviderEvent::ToolCall(call) => Some(call.tool_use_id),
                _ => None,
            })
            .collect()
    };
    // Two runs of different runtimes stand for a restart: ids never repeat.
    let first = ids(events_of(chunks(&[call(), call()])).await);
    let second = ids(events_of(chunks(&[call()])).await);
    let all: std::collections::HashSet<&String> = first.iter().chain(&second).collect();
    assert_eq!(all.len(), 3, "{first:?} {second:?}");
    assert!(
        all.iter().all(|id| id.starts_with("shell_exec_")),
        "{all:?}"
    );
}
