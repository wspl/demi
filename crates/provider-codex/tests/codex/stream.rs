//! The events a Codex run makes over server-sent events, and its failures
//! (`providers.md` § A run, `failures-and-recovery.md`).

use std::time::Duration;

use demi_core::{FailureSource, ProviderErrorDiagnostics, Timestamp, TokenUsage};
use demi_provider::{
    ErrorCode, HttpFailureRecord, ProviderEvent, ProviderFailure, ToolCall,
    testing::{MockResponse, MockVendor, inference_request},
};
use demi_provider_codex::{CodexConfig, CodexProvider, TransportMode, read_codex_failure};
use futures_util::StreamExt;
use serde_json::json;

use crate::{NOW, RESPONSES, fresh_token, pool_with, provider, run, runtime_of, secret, stream};

async fn events_of(response: MockResponse) -> (Vec<ProviderEvent>, MockVendor) {
    let vendor = MockVendor::start().await;
    vendor.respond_at(RESPONSES, response);
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let events = run(runtime_of(&provider).as_mut(), inference_request()).await;
    (events, vendor)
}

fn failure_of(events: Vec<ProviderEvent>) -> ProviderFailure {
    match <[ProviderEvent; 1]>::try_from(events) {
        Ok([ProviderEvent::Error(failure)]) => failure,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_run_streams_thinking_text_tool_calls_and_usage() {
    let (events, _vendor) = events_of(stream(&[
        json!({ "type": "response.output_item.added", "item": { "type": "reasoning", "id": "rs_1" } }),
        json!({ "type": "response.reasoning_text.delta", "delta": "think" }),
        json!({ "type": "response.output_item.done", "item": { "type": "reasoning", "id": "rs_1", "encrypted_content": "enc" } }),
        json!({ "type": "response.output_text.delta", "delta": "hello" }),
        json!({ "type": "response.output_item.done", "item": { "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "shell_exec", "arguments": "{\"script\":\"pwd\"}" } }),
        json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 10, "output_tokens": 3 } } }),
    ]))
    .await;
    let usage = TokenUsage {
        input_tokens: 10,
        output_tokens: 3,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    };
    assert_eq!(
        events,
        [
            ProviderEvent::ThinkingStart,
            ProviderEvent::ThinkingDelta("think".into()),
            ProviderEvent::ThinkingSignature(
                r#"codex:{"type":"reasoning","id":"rs_1","encrypted_content":"enc"}"#.into()
            ),
            ProviderEvent::TextDelta("hello".into()),
            ProviderEvent::ToolCall(ToolCall {
                tool_use_id: "call_1|fc_1".into(),
                tool_name: "shell_exec".into(),
                input: json!({ "script": "pwd" })
            }),
            ProviderEvent::Response(usage),
        ]
    );
}

#[tokio::test]
async fn a_stream_that_ends_without_completion_responds_with_zero_usage() {
    let (events, _vendor) = events_of(stream(&[
        json!({ "type": "response.output_text.delta", "delta": "hi" }),
    ]))
    .await;
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("hi".into()),
            ProviderEvent::Response(TokenUsage::default())
        ]
    );
}

#[tokio::test]
async fn a_usage_limit_in_the_stream_names_when_it_lifts() {
    // 2026-09-22T07:37:39Z, as Unix seconds.
    let frame = json!({ "type": "error", "error": { "type": "usage_limit_reached", "message": "The usage limit has been reached", "plan_type": "pro", "resets_at": 1_790_062_659, "resets_in_seconds": 321_250 }, "status_code": 429 });
    let (events, _vendor) = events_of(stream(std::slice::from_ref(&frame))).await;
    let failure = failure_of(events);
    assert_eq!(
        (failure.message.as_str(), failure.code.clone()),
        (
            "The usage limit has been reached",
            Some(ErrorCode::RateLimit)
        )
    );
    let lifts = "2026-09-22T07:37:39.000Z"
        .parse::<Timestamp>()
        .unwrap()
        .as_millisecond()
        - NOW.parse::<Timestamp>().unwrap().as_millisecond();
    assert_eq!(
        failure.retry_after,
        Some(Duration::from_millis(lifts.unsigned_abs()))
    );
    let diagnostics = failure.diagnostics.unwrap();
    assert_eq!(
        (diagnostics.source, diagnostics.provider_code.as_deref()),
        (FailureSource::Stream, Some("usage_limit_reached"))
    );
    assert_eq!(diagnostics.upstream, Some(frame.to_string()));
}

#[tokio::test]
async fn a_refused_request_keeps_the_whole_answer_and_is_not_retried_by_the_provider() {
    let body = r#"{"error":{"code":"server_error","message":"backend failed"}}"#;
    let (events, vendor) = events_of(
        MockResponse::status(500)
            .header("x-request-id", "req-http-1")
            .header("retry-after", "2")
            .chunk(body),
    )
    .await;
    assert_eq!(vendor.requests().len(), 1);
    let failure = failure_of(events);
    assert_eq!(
        failure.message,
        format!("Codex API request failed with HTTP 500: {body}")
    );
    assert_eq!(
        (failure.code.clone(), failure.retry_after),
        (Some(ErrorCode::Overloaded), Some(Duration::from_secs(2)))
    );
    let diagnostics = failure.diagnostics.unwrap();
    assert_eq!(
        (diagnostics.source, diagnostics.http_status),
        (FailureSource::Http, Some(500))
    );
    assert_eq!(diagnostics.provider_code.as_deref(), Some("server_error"));
    assert_eq!(
        diagnostics.provider_request_id.as_deref(),
        Some("req-http-1")
    );
    let record = HttpFailureRecord::read(&diagnostics).unwrap();
    assert_eq!(record.body, body);
    let names: Vec<&str> = record
        .headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(
        names.contains(&"retry-after") && names.contains(&"x-request-id") && names.is_sorted(),
        "{names:?}"
    );
}

#[tokio::test]
async fn response_headers_that_do_not_arrive_in_time_fail_the_run_as_overloaded() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(RESPONSES, MockResponse::silent());
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let mut config = CodexConfig::new("codex", "Codex", Some(crate::ACCOUNT.into()));
    config.backend_url = vendor.url("/backend-api").parse().unwrap();
    config.auth_url = vendor.url("").parse().unwrap();
    config.transport = TransportMode::Sse;
    config.header_timeout = Duration::from_millis(50);
    let clock = std::sync::Arc::new(demi_provider::testing::FixedClock(NOW.parse().unwrap()));
    let snapshots = std::sync::Arc::new(demi_provider::quota::MemorySnapshots::new());
    let provider = CodexProvider::new(
        config,
        std::sync::Arc::new(pool),
        snapshots,
        reqwest::Client::new(),
        clock,
    );
    let failure = failure_of(run(runtime_of(&provider).as_mut(), inference_request()).await);
    assert_eq!(
        failure.message,
        "Codex SSE response headers timed out after 50ms"
    );
    assert_eq!(failure.code, Some(ErrorCode::Overloaded));
    assert_eq!(
        failure.diagnostics.unwrap().source,
        FailureSource::Transport
    );
}

#[tokio::test]
async fn cancelling_mid_stream_stops_the_download() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(
        RESPONSES,
        MockResponse::event_stream(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hel\"}\n\n",
        )
        .stay_open(),
    );
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
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

fn diagnostics(source: FailureSource, upstream: String) -> ProviderErrorDiagnostics {
    ProviderErrorDiagnostics {
        source,
        client_request_id: None,
        provider_request_id: None,
        provider_response_id: None,
        provider_code: None,
        http_status: None,
        upstream: Some(upstream),
    }
}

#[test]
fn the_codex_reader_finds_when_a_usage_limit_lifts() {
    let received: Timestamp = NOW.parse().unwrap();
    let at: Timestamp = "2026-09-22T07:37:39.000Z".parse().unwrap();
    let read = |source, upstream: &str| {
        read_codex_failure(&diagnostics(source, upstream.to_owned()), received).retry_at
    };
    let frame = r#"{"type":"error","error":{"type":"usage_limit_reached","message":"The usage limit has been reached","plan_type":"pro","resets_at":1790062659,"resets_in_seconds":321250},"status_code":429}"#;
    assert_eq!(read(FailureSource::Stream, frame), Some(at));
    let relative = r#"{"type":"event","event":{"type":"error","error":{"resets_in_seconds":90}}}"#;
    assert_eq!(
        read(FailureSource::Stream, relative),
        Some("2026-09-18T14:01:30.000Z".parse().unwrap())
    );
    let failed = r#"{"type":"response.failed","response":{"error":{"resets_at":1790062659}}}"#;
    assert_eq!(read(FailureSource::Stream, failed), Some(at));
    let http = HttpFailureRecord {
        status: 429,
        headers: Vec::new(),
        body: r#"{"error":{"resets_at":1790062659}}"#.into(),
    };
    assert_eq!(read(FailureSource::Http, &http.to_json()), Some(at));
    // Without the limit's fields, the standard Retry-After reading applies.
    let waited = HttpFailureRecord {
        status: 503,
        headers: vec![("retry-after".into(), "30".into())],
        body: "busy".into(),
    };
    assert_eq!(
        read(FailureSource::Http, &waited.to_json()),
        Some("2026-09-18T14:00:30.000Z".parse().unwrap())
    );
    // A field in a form the reader does not expect names no time.
    assert_eq!(
        read(
            FailureSource::Stream,
            r#"{"type":"error","error":{"resets_at":"soon"}}"#
        ),
        None
    );
}
