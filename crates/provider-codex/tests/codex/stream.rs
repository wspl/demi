//! The events a Codex run makes over server-sent events, and its failures
//! (`crates/provider-common/src/contract.rs`).

use demi_provider_codex::{TransportMode, read_codex_failure};
use demi_provider_common::{
    ErrorCode, HttpFailureRecord, ProviderEvent, ProviderFailure, ToolCall,
    testing::{MockResponse, MockVendor, inference_request},
};
use demi_shared_types::{FailureSource, ProviderErrorDiagnostics, Timestamp, TokenUsage};
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
            ProviderEvent::Response(Some(usage)),
        ]
    );
}

/// Codex's own sign of a used-up account, as the Codex CLI reads it, is a
/// usage limit, which waiting a few seconds does not fix; any other 429
/// only throttles. One mock vendor per case, about 10 ms each.
#[tokio::test]
async fn a_used_up_account_is_a_usage_limit_and_any_other_429_throttles() {
    let cases = [
        (
            r#"{"error":{"type":"usage_limit_reached","message":"The usage limit has been reached","plan_type":"plus","resets_at":1790062659}}"#,
            ErrorCode::UsageLimit,
        ),
        (
            r#"{"error":{"type":"insufficient_quota","message":"You exceeded your current quota"}}"#,
            ErrorCode::UsageLimit,
        ),
        (
            r#"{"error":{"code":"credit_balance_exhausted","message":"Your credit balance is exhausted"}}"#,
            ErrorCode::UsageLimit,
        ),
        (
            r#"{"error":{"code":"project_spend_limit_exceeded","message":"Spend limit reached"}}"#,
            ErrorCode::UsageLimit,
        ),
        (
            r#"{"error":{"code":"organization_usage_limit_exceeded","message":"Usage limit reached"}}"#,
            ErrorCode::UsageLimit,
        ),
        (
            r#"{"error":{"type":"rate_limit_exceeded","message":"Rate limit reached for requests"}}"#,
            ErrorCode::RateLimit,
        ),
        ("", ErrorCode::RateLimit),
    ];
    for (body, code) in cases {
        let (events, _vendor) = events_of(MockResponse::status(429).chunk(body)).await;
        assert_eq!(failure_of(events).code, Some(code), "{body}");
    }
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
