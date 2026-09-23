//! The events a run makes of Gemini's stream (`providers.md` § A run,
//! § Endpoints).

use demi_core::{FailureSource, TokenUsage};
use demi_provider::{
    ErrorCode, ProviderEvent, ProviderFailure, ToolCall,
    testing::{MockResponse, MockVendor, inference_request},
};
use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::{chunks, run, runtime};

async fn events_of(response: MockResponse) -> Vec<ProviderEvent> {
    let vendor = MockVendor::start().await;
    vendor.respond(response);
    run(runtime(&vendor).as_mut(), inference_request()).await
}

fn failure_of(events: Vec<ProviderEvent>) -> ProviderFailure {
    match <[ProviderEvent; 1]>::try_from(events) {
        Ok([ProviderEvent::Error(failure)]) => failure,
        other => panic!("{other:?}"),
    }
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
        json!({ "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 4, "thoughtsTokenCount": 20, "cachedContentTokenCount": 3 } }),
    ]))
    .await;
    // Thinking is billed apart from the answer, and both are output.
    let usage = TokenUsage {
        input_tokens: 10,
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
            ProviderEvent::Response(usage),
        ]
    );
}

#[tokio::test]
async fn a_signed_call_opens_thinking_and_a_signature_after_thoughts_closes_it() {
    let events = events_of(chunks(&[
        parts(json!([{ "functionCall": { "name": "look", "id": "c1" }, "thoughtSignature": "sig-1" }])),
        parts(json!([{ "text": "plan", "thought": true }, { "text": "", "thoughtSignature": "sig-2" }, { "text": "answer" }])),
    ]))
    .await;
    assert_eq!(
        events[..6],
        [
            ProviderEvent::ThinkingStart,
            ProviderEvent::ThinkingSignature("google:sig-1".into()),
            ProviderEvent::ToolCall(ToolCall { tool_use_id: "c1".into(), tool_name: "look".into(), input: json!({}) }),
            ProviderEvent::ThinkingStart,
            ProviderEvent::ThinkingDelta("plan".into()),
            ProviderEvent::ThinkingSignature("google:sig-2".into()),
        ]
    );
    assert_eq!(events[6], ProviderEvent::TextDelta("answer".into()));
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
    assert!(all.iter().all(|id| id.starts_with("shell_exec_")), "{all:?}");
}

#[tokio::test]
async fn a_stream_error_fails_the_run_with_the_chunk_as_its_record() {
    let chunk = json!({ "error": { "status": "RESOURCE_EXHAUSTED", "message": "quota exceeded" } });
    let failure = failure_of(events_of(chunks(&[chunk.clone(), parts(json!([{ "text": "never read" }]))])).await);
    assert_eq!((failure.message.as_str(), failure.code), ("quota exceeded", Some(ErrorCode::RateLimit)));
    let diagnostics = failure.diagnostics.unwrap();
    assert_eq!((diagnostics.source, diagnostics.provider_code.as_deref()), (FailureSource::Stream, Some("RESOURCE_EXHAUSTED")));
    assert_eq!(diagnostics.upstream, Some(chunk.to_string()));
}

#[tokio::test]
async fn a_chunk_that_breaks_its_shape_is_a_protocol_failure_naming_the_field() {
    for (chunk, field) in [
        (json!({ "candidates": { "content": { "parts": [{ "text": "hello" }] } } }), "candidates"),
        (json!({ "usageMetadata": { "promptTokenCount": "10" } }), "promptTokenCount"),
        (json!({ "candidates": [{ "content": { "parts": [{ "functionCall": { "name": "" } }] } }] }), "name"),
    ] {
        let failure = failure_of(events_of(chunks(&[chunk.clone()])).await);
        assert_eq!(failure.code, None, "{chunk}");
        assert!(failure.message.contains(field), "{chunk}: {}", failure.message);
        assert_eq!(failure.diagnostics.unwrap().upstream, Some(chunk.to_string()));
    }
}

#[tokio::test]
async fn a_refused_request_fails_with_the_vendor_record() {
    let body = r#"{"error":{"code":429,"message":"Resource has been exhausted","status":"RESOURCE_EXHAUSTED"}}"#;
    let failure = failure_of(events_of(MockResponse::status(429).chunk(body)).await);
    assert_eq!(failure.message, format!("Google API request failed with HTTP 429: {body}"));
    assert_eq!(failure.code, Some(ErrorCode::RateLimit));
}

#[tokio::test]
async fn cancelling_mid_stream_ends_the_run_without_an_event_and_drops_the_connection() {
    let vendor = MockVendor::start().await;
    let first = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hel\"}]}}]}\n\n";
    vendor.respond(MockResponse::event_stream(first).stay_open());
    let mut runtime = runtime(&vendor);
    let request = inference_request();
    let cancel = request.cancel.clone();
    let mut events = runtime.run(request);
    assert_eq!(events.next().await, Some(ProviderEvent::TextDelta("hel".into())));
    cancel.cancel();
    assert_eq!(events.next().await, None);
    vendor.disconnected().await;

    let request = inference_request();
    request.cancel.cancel();
    let vendor = MockVendor::start().await;
    assert!(run(crate::runtime(&vendor).as_mut(), request).await.is_empty());
    assert!(vendor.requests().is_empty());
}
