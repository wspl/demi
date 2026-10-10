//! The Chat Completions stream mapped onto a run's events (`providers.md` §
//! A run, § Vendors from models.dev), over recorded frames.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    ErrorCode, ProviderEvent, ProviderFailure, ToolCall, read_http_failure,
    testing::FixedClock,
    wire::{Vendor, chat_completions::map_sse},
};
use demi_shared_types::{FailureSource, Timestamp, TokenUsage};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn vendor() -> Vendor {
    Vendor {
        label: "Grok Build",
        reader: read_http_failure,
        clock: Arc::new(FixedClock(
            "2026-09-18T14:00:00.000Z".parse::<Timestamp>().unwrap(),
        )),
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
    map_sse(body(frames), vendor(), CancellationToken::new())
        .collect()
        .await
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

fn failure_of(events: Vec<ProviderEvent>) -> ProviderFailure {
    match <[ProviderEvent; 1]>::try_from(events) {
        Ok([ProviderEvent::Error(failure)]) => failure,
        other => panic!("{other:?}"),
    }
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
            ProviderEvent::Response(usage),
        ]
    );
}

#[tokio::test]
async fn reasoning_content_is_thinking_that_starts_once() {
    let events = events(&[
        json!({ "choices": [{ "delta": { "role": "assistant", "content": null, "reasoning_content": "" } }] }),
        json!({ "choices": [{ "delta": { "content": null, "reasoning_content": "think " } }] }),
        json!({ "choices": [{ "delta": { "content": null, "reasoning_content": "more" } }] }),
        json!({ "choices": [{ "delta": { "content": "answer" }, "finish_reason": "stop" }] }),
    ])
    .await;
    assert_eq!(
        events,
        [
            ProviderEvent::ThinkingStart,
            ProviderEvent::ThinkingDelta("think ".into()),
            ProviderEvent::ThinkingDelta("more".into()),
            ProviderEvent::TextDelta("answer".into()),
            ProviderEvent::Response(TokenUsage::default()),
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
            ProviderEvent::Response(TokenUsage::default()),
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

#[tokio::test]
async fn a_body_that_ends_before_a_finish_reason_fails_even_with_usage_and_keeps_the_text() {
    // A gateway that sends running totals in every chunk, cut mid-sentence.
    let events = events(&[
        json!({ "choices": [{ "delta": { "content": "expected 200, got" } }], "usage": { "prompt_tokens": 9, "completion_tokens": 232 } }),
        json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": "call-1", "function": { "name": "read_file", "arguments": "{}" } }] } }] }),
    ])
    .await;
    let [ProviderEvent::TextDelta(text), .., ProviderEvent::Error(failure)] = &events[..] else {
        panic!("{events:?}");
    };
    assert_eq!(text, "expected 200, got");
    assert!(!events.iter().any(|event| matches!(event, ProviderEvent::ToolCall(_))), "{events:?}");
    assert_eq!(
        (failure.message.as_str(), &failure.code),
        ("The provider's stream ended before the reply was complete", &Some(ErrorCode::Overloaded))
    );
}

#[tokio::test]
async fn a_reply_ended_for_the_output_limit_or_a_filter_fails_with_the_vendors_reason() {
    for (reason, message) in [
        ("length", "The reply reached the model's output limit (length)"),
        ("content_filter", "The provider's filter stopped the reply (content_filter)"),
        ("insufficient_system_resource", "The provider ended the reply before it was complete (insufficient_system_resource)"),
    ] {
        let finish = json!({ "choices": [{ "delta": {}, "finish_reason": reason }] });
        let events = events(&[
            json!({ "choices": [{ "delta": { "content": "half" } }] }),
            finish.clone(),
            json!({ "choices": [], "usage": { "prompt_tokens": 9, "completion_tokens": 4 } }),
            json!("[DONE]"),
        ])
        .await;
        let [ProviderEvent::TextDelta(text), ProviderEvent::Error(failure)] = &events[..] else {
            panic!("{events:?}");
        };
        assert_eq!(text, "half");
        assert_eq!((failure.message.as_str(), &failure.code), (message, &Some(ErrorCode::Incomplete)));
        let diagnostics = failure.diagnostics.as_ref().unwrap();
        assert_eq!(
            (diagnostics.source, diagnostics.provider_code.as_deref(), diagnostics.upstream.clone()),
            (FailureSource::Stream, Some(reason), Some(finish.to_string()))
        );
    }
}

#[tokio::test]
async fn a_vendor_error_ends_the_run_with_the_chunk_as_its_record() {
    let frame = json!({ "error": { "message": "quota exceeded", "type": "insufficient_quota" } });
    let failure = failure_of(
        events(&[
            frame.clone(),
            json!({ "choices": [{ "delta": { "content": "never read" } }] }),
        ])
        .await,
    );
    assert_eq!(
        (failure.message.as_str(), failure.code),
        ("quota exceeded", Some(ErrorCode::RateLimit))
    );
    let diagnostics = failure.diagnostics.unwrap();
    assert_eq!(
        (diagnostics.source, diagnostics.provider_code.as_deref()),
        (FailureSource::Stream, Some("insufficient_quota"))
    );
    assert_eq!(diagnostics.upstream, Some(frame.to_string()));

    let unnamed = failure_of(events(&[json!({ "error": {} })]).await);
    assert_eq!(
        (unnamed.message.as_str(), unnamed.code),
        ("Grok Build stream error", None)
    );
}

#[tokio::test]
async fn a_malformed_chunk_is_a_protocol_failure_that_names_its_field() {
    for (frame, field) in [
        (
            json!({ "choices": [{ "delta": { "content": { "text": "hi" } } }] }),
            "content",
        ),
        (
            json!({ "choices": [{ "delta": { "tool_calls": "none" } }] }),
            "tool_calls",
        ),
        (
            json!({ "usage": { "prompt_tokens": "12" } }),
            "prompt_tokens",
        ),
    ] {
        let failure = failure_of(events(std::slice::from_ref(&frame)).await);
        assert_eq!(failure.code, None, "{frame}");
        assert!(
            failure.message.contains(field),
            "{frame}: {}",
            failure.message
        );
        assert_eq!(
            failure.diagnostics.unwrap().upstream,
            Some(frame.to_string())
        );
    }
}

#[tokio::test]
async fn a_cancelled_run_ends_without_a_further_event() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let frames = body(&[json!({ "choices": [{ "delta": { "content": "hi" } }] })]);
    let events: Vec<ProviderEvent> = map_sse(frames, vendor(), cancel).collect().await;
    assert!(events.is_empty(), "{events:?}");
}
