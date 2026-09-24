//! The Responses stream mapped onto a run's events (`providers.md` § A run,
//! § Reading vendor input), over recorded frames.

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use demi_core::{FailureSource, ProviderErrorDiagnostics, ProviderFailureFacts, Timestamp, TokenUsage};
use demi_provider::{
    ErrorCode, ProviderEvent, ProviderFailure, ToolCall, read_http_failure,
    testing::FixedClock,
    wire::{
        Vendor,
        responses::{decode_frame, map_events, sse_events},
    },
};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const NOW: &str = "2026-09-18T14:00:00.000Z";

fn vendor(reader: demi_provider::FailureReader) -> Vendor {
    Vendor {
        label: "Codex",
        reader,
        clock: Arc::new(FixedClock(NOW.parse::<Timestamp>().unwrap())),
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

async fn events_with(frames: &[Value], reader: demi_provider::FailureReader) -> Vec<ProviderEvent> {
    let cancel = CancellationToken::new();
    let events = sse_events(body(frames), "Codex", cancel.clone());
    map_events(events, vendor(reader), "codex:", cancel).collect().await
}

async fn events(frames: &[Value]) -> Vec<ProviderEvent> {
    events_with(frames, read_http_failure).await
}

fn failure_of(events: Vec<ProviderEvent>) -> ProviderFailure {
    match <[ProviderEvent; 1]>::try_from(events) {
        Ok([ProviderEvent::Error(failure)]) => failure,
        other => panic!("{other:?}"),
    }
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
        json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 100, "output_tokens": 7, "input_tokens_details": { "cached_tokens": 60 } } } }),
        json!("[DONE]"),
    ])
    .await;
    let [start, delta, ProviderEvent::ThinkingSignature(signature), text, call, response] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!((start, delta, text), (&ProviderEvent::ThinkingStart, &ProviderEvent::ThinkingDelta("think".into()), &ProviderEvent::TextDelta("hello ".into())));
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
    let usage = TokenUsage {
        input_tokens: 40,
        output_tokens: 7,
        cache_read_tokens: 60,
        cache_write_tokens: 0,
    };
    assert_eq!(response, &ProviderEvent::Response(usage));
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
            ProviderEvent::TextDelta(text) | ProviderEvent::ThinkingDelta(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["raw reasoning", "因为", "天空是蓝的", "and the next message", "no"]);
    assert_eq!(events.last(), Some(&ProviderEvent::Response(TokenUsage::default())));
}

#[tokio::test]
async fn a_stream_that_ends_without_its_completion_still_responds_with_zero_usage() {
    let events = events(&[json!({ "type": "response.output_text.delta", "delta": "hi" })]).await;
    assert_eq!(events, [ProviderEvent::TextDelta("hi".into()), ProviderEvent::Response(TokenUsage::default())]);
}

#[tokio::test]
async fn a_failure_keeps_the_frame_as_its_record_and_takes_the_wait_from_the_providers_reader() {
    // A field in a form Demi does not read survives: the record is the text
    // as it came.
    let frame = r#"{"type":"error","error":{"type":"usage_limit_reached","message":"The usage limit has been reached","resets_at":"soon"},"status_code":"429"}"#;
    let in_a_minute: demi_provider::FailureReader = |diagnostics, received_at| {
        assert_eq!(diagnostics.upstream.as_deref(), Some(r#"{"type":"error","error":{"type":"usage_limit_reached","message":"The usage limit has been reached","resets_at":"soon"},"status_code":"429"}"#));
        let later = Timestamp::from_millisecond(received_at.as_millisecond() + 60_000).unwrap();
        ProviderFailureFacts { retry_at: Some(later) }
    };
    let failure = failure_of(events_with(&[Value::String(frame.into())], in_a_minute).await);
    assert_eq!(failure.message, "The usage limit has been reached");
    assert_eq!(failure.code, Some(ErrorCode::RateLimit));
    assert_eq!(failure.retry_after, Some(Duration::from_secs(60)));
    let diagnostics = failure.diagnostics.unwrap();
    assert_eq!((diagnostics.source, diagnostics.provider_code.as_deref()), (FailureSource::Stream, Some("usage_limit_reached")));
    assert_eq!(diagnostics.upstream.as_deref(), Some(frame));
}

#[tokio::test]
async fn failed_incomplete_and_error_events_end_the_run_classified_and_naming_the_vendor() {
    let cases = [
        (json!({ "type": "response.failed", "response": { "error": { "code": "context_length_exceeded", "message": "too long" } } }), "too long", Some(ErrorCode::ContextLengthExceeded), Some("context_length_exceeded")),
        (json!({ "type": "response.failed" }), "Codex response failed", None, None),
        (json!({ "type": "response.incomplete", "response": { "incomplete_details": { "reason": "max_output_tokens" } } }), "Incomplete Codex response returned, reason: max_output_tokens", Some(ErrorCode::ContextLengthExceeded), None),
        (json!({ "type": "response.incomplete", "response": { "incomplete_details": { "reason": "content_filter" } } }), "Incomplete Codex response returned, reason: content_filter", Some(ErrorCode::Incomplete), None),
        (json!({ "type": "error", "code": "server_error", "message": "backend failed" }), "backend failed", Some(ErrorCode::Overloaded), Some("server_error")),
        (json!({ "type": "error", "error": { "type": "invalid_request_error", "message": "Invalid prompt_cache_key" }, "status": 400 }), "Invalid prompt_cache_key", Some(ErrorCode::Vendor("invalid_request_error".into())), Some("invalid_request_error")),
        // A code that is not a string reads as absent, so the message still
        // reaches the user.
        (json!({ "type": "error", "message": "upstream failed", "code": 500 }), "upstream failed", None, None),
        (json!({ "type": "error" }), "Codex stream error", None, None),
    ];
    for (frame, message, code, provider_code) in cases {
        let events = events(&[frame.clone(), json!({ "type": "response.output_text.delta", "delta": "never read" })]).await;
        let failure = failure_of(events);
        assert_eq!((failure.message.as_str(), &failure.code), (message, &code), "{frame}");
        let diagnostics = failure.diagnostics.unwrap();
        assert_eq!(diagnostics.provider_code.as_deref(), provider_code, "{frame}");
        let upstream: Value = serde_json::from_str(diagnostics.upstream.as_deref().unwrap()).unwrap();
        assert_eq!(upstream, frame);
    }
}

#[tokio::test]
async fn a_failure_keeps_the_request_and_response_ids_it_carries() {
    let failure = failure_of(
        events(&[json!({ "type": "response.failed", "response": { "id": "resp-1", "error": { "code": "server_error", "message": "Failed. Please include the request ID req-1 in your message." } } })]).await,
    );
    assert_eq!(failure.code, Some(ErrorCode::Overloaded));
    let ProviderErrorDiagnostics { provider_request_id, provider_response_id, .. } = failure.diagnostics.unwrap();
    assert_eq!((provider_request_id.as_deref(), provider_response_id.as_deref()), (Some("req-1"), Some("resp-1")));
    let named = failure_of(events(&[json!({ "type": "error", "error": { "message": "failed", "request_id": "req-2" } })]).await);
    assert_eq!(named.diagnostics.unwrap().provider_request_id.as_deref(), Some("req-2"));
}

#[tokio::test]
async fn a_malformed_mapped_event_is_a_protocol_failure_that_names_its_field() {
    let cases = [
        (json!({ "type": "response.output_item.done", "item": { "type": "message", "content": 42 } }), "content"),
        (json!({ "type": "response.output_text.delta", "delta": 42 }), "delta"),
        (json!({ "type": "response.completed", "response": { "usage": { "input_tokens": "100" } } }), "input_tokens"),
        (json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 12.5 } } }), "input_tokens"),
        (json!({ "type": "response.output_item.done", "item": { "type": "reasoning", "summary": [{ "type": "summary_text" }] } }), "text"),
    ];
    for (frame, field) in cases {
        let failure = failure_of(events(std::slice::from_ref(&frame)).await);
        assert_eq!(failure.code, None, "{frame}");
        assert!(failure.message.contains(field), "{frame}: {}", failure.message);
        let diagnostics = failure.diagnostics.unwrap();
        assert_eq!(diagnostics.source, FailureSource::Stream);
        assert_eq!(diagnostics.upstream, Some(frame.to_string()));
    }
    // An event or item type Demi does not map decodes to nothing.
    assert_eq!(decode_frame(r#"{"type":"response.queued","id":"r1"}"#).unwrap(), None);
    assert_eq!(decode_frame(" [DONE] ").unwrap(), None);
    assert!(decode_frame(r#"{"delta":"no type"}"#).is_err());
}

#[tokio::test]
async fn a_cancelled_run_ends_without_a_further_event() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let frames = body(&[json!({ "type": "response.output_text.delta", "delta": "hi" })]);
    let events: Vec<ProviderEvent> = map_events(sse_events(frames, "Codex", cancel.clone()), vendor(read_http_failure), "codex:", cancel)
        .collect()
        .await;
    assert!(events.is_empty(), "{events:?}");
}
