//! Failure codes, the HTTP failure record and its standard reading
//! (`failures-and-recovery.md` § The failure record, § Reading a failure).

use std::time::Duration;

use demi_core::{FailureSource, ProviderErrorDiagnostics, ProviderFailureFacts, Timestamp};
use demi_provider::{
    ErrorCode, HttpFailureRecord, ProviderFailure, http_failure, read_http_failure, retry_at,
    testing::{FixedClock, MockResponse, MockVendor},
};
use http::{HeaderMap, HeaderValue, StatusCode};
use serde_json::json;

fn time(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn record(source: FailureSource, upstream: &str) -> ProviderErrorDiagnostics {
    ProviderErrorDiagnostics {
        source,
        client_request_id: None,
        provider_request_id: None,
        provider_response_id: None,
        provider_code: None,
        http_status: None,
        upstream: Some(upstream.to_owned()),
    }
}

#[test]
fn an_http_status_decides_the_code() {
    let cases = [
        (401, "", Some(ErrorCode::AuthExpired)),
        (403, "", Some(ErrorCode::AuthExpired)),
        (429, "", Some(ErrorCode::RateLimit)),
        (408, "", Some(ErrorCode::Overloaded)),
        (409, "", Some(ErrorCode::Overloaded)),
        (425, "", Some(ErrorCode::Overloaded)),
        (500, "", Some(ErrorCode::Overloaded)),
        (529, "", Some(ErrorCode::Overloaded)),
        (400, "prompt is too long: 213000 tokens", Some(ErrorCode::ContextLengthExceeded)),
        (400, "Context length exceeded", Some(ErrorCode::ContextLengthExceeded)),
        (400, "bad request", None),
        (404, "", None),
    ];
    for (status, message, code) in cases {
        assert_eq!(ErrorCode::from_http(status, message), code, "{status} {message}");
    }
}

#[test]
fn a_vendor_failure_classifies_by_whole_words_and_falls_back_to_the_vendor_code() {
    let cases = [
        (None, "maximum context length", Some(ErrorCode::ContextLengthExceeded)),
        (None, "max_tokens: 64000 > 32000 tokens", Some(ErrorCode::ContextLengthExceeded)),
        (None, "rate limit reached", Some(ErrorCode::RateLimit)),
        (Some("rate_limit_error"), "Number of requests exceeded", Some(ErrorCode::RateLimit)),
        (None, "insufficient balance", Some(ErrorCode::RateLimit)),
        (Some("insufficient_quota"), "You exceeded your plan", Some(ErrorCode::RateLimit)),
        (Some("usage_limit_reached"), "The usage limit has been reached", Some(ErrorCode::RateLimit)),
        (None, "invalid api key", Some(ErrorCode::AuthExpired)),
        (Some("authentication_error"), "invalid x-api-key", Some(ErrorCode::AuthExpired)),
        (None, "service unavailable", Some(ErrorCode::Overloaded)),
        (Some("overloaded_error"), "Overloaded", Some(ErrorCode::Overloaded)),
        (Some("server_error"), "backend failed", Some(ErrorCode::Overloaded)),
        (Some("internal-error"), "backend failed", Some(ErrorCode::Overloaded)),
        (Some("api_error"), "Internal server error", Some(ErrorCode::Overloaded)),
        (None, "Codex SSE response headers timed out after 20000ms", Some(ErrorCode::Overloaded)),
        (None, "connect timeout", Some(ErrorCode::Overloaded)),
        (None, "socket hang up", Some(ErrorCode::Overloaded)),
        (None, "read ECONNRESET", Some(ErrorCode::Overloaded)),
        // A vendor code stands when no word decides.
        (Some("invalid_request_error"), "Invalid prompt_cache_key", Some(ErrorCode::Vendor("invalid_request_error".into()))),
        (Some("custom"), "something else", Some(ErrorCode::Vendor("custom".into()))),
        // Words inside other words and a bare "limit" decide nothing.
        (Some("invalid_request_error"), "Could not generate the schema", Some(ErrorCode::Vendor("invalid_request_error".into()))),
        (Some("invalid_request_error"), "tools: exceeds the limit of 128", Some(ErrorCode::Vendor("invalid_request_error".into()))),
        (None, "iterate over the unlimited list", None),
        // `usage` decides only as part of a usage limit.
        (Some("invalid_request_error"), "Invalid usage of the tools parameter", Some(ErrorCode::Vendor("invalid_request_error".into()))),
        (None, "usage limit exceeded for this month", Some(ErrorCode::RateLimit)),
        (Some(""), "nothing to see", None),
    ];
    for (code, message, expected) in cases {
        assert_eq!(ErrorCode::classify(code, message), expected, "{code:?} {message}");
    }
}

#[test]
fn a_code_is_written_as_its_string() {
    for code in ["rate_limit", "overloaded", "context_length_exceeded", "auth_missing", "custom_vendor"] {
        let parsed: ErrorCode = code.parse().unwrap();
        assert_eq!(parsed.to_string(), code);
    }
    assert_eq!("rate_limit".parse::<ErrorCode>().unwrap(), ErrorCode::RateLimit);
}

#[test]
fn a_record_lists_lowercase_headers_sorted_with_repeats_in_arrival_order() {
    let mut headers = HeaderMap::new();
    headers.append("X-Request-Id", HeaderValue::from_static("req-9"));
    headers.append("Set-Cookie", HeaderValue::from_static("b=2"));
    headers.append("retry-after", HeaderValue::from_static("120"));
    headers.append("set-cookie", HeaderValue::from_static("a=1"));
    let body = r#"{"error":{"message":"slow down"}}"#.to_owned();
    let record = HttpFailureRecord::from_response(StatusCode::TOO_MANY_REQUESTS, &headers, body);
    assert_eq!(
        record.to_json(),
        r#"{"status":429,"headers":[["retry-after","120"],["set-cookie","b=2"],["set-cookie","a=1"],["x-request-id","req-9"]],"body":"{\"error\":{\"message\":\"slow down\"}}"}"#
    );
}

#[tokio::test]
async fn an_http_failure_keeps_the_vendor_text_the_whole_response_and_its_wait() {
    let vendor = MockVendor::start().await;
    vendor.respond(
        MockResponse::status(429)
            .header("retry-after", "120")
            .header("x-request-id", "req-9")
            .header("set-cookie", "a=b")
            .chunk(r#"{"error":{"message":"slow down"}}"#),
    );
    vendor.respond(MockResponse::status(502).chunk("<html>Bad gateway</html>"));
    let client = reqwest::Client::new();
    let clock = FixedClock(time("2026-09-18T14:00:00.000Z"));

    let response = client.get(vendor.url("/v1/messages")).send().await.unwrap();
    let failure = http_failure(response, "Acme", read_http_failure, &clock).await;
    assert_eq!(failure.message, r#"Acme API request failed with HTTP 429: {"error":{"message":"slow down"}}"#);
    assert_eq!(failure.code, Some(ErrorCode::RateLimit));
    assert_eq!(failure.retry_after, Some(Duration::from_secs(120)));
    let diagnostics = failure.diagnostics.unwrap();
    assert_eq!((diagnostics.source, diagnostics.http_status), (FailureSource::Http, Some(429)));
    let record: serde_json::Value = serde_json::from_str(diagnostics.upstream.as_deref().unwrap()).unwrap();
    assert_eq!(record.as_object().unwrap().keys().collect::<Vec<_>>(), ["status", "headers", "body"]);
    assert_eq!(record["body"], json!(r#"{"error":{"message":"slow down"}}"#));
    let pairs = record["headers"].as_array().unwrap();
    for pair in [json!(["retry-after", "120"]), json!(["set-cookie", "a=b"]), json!(["x-request-id", "req-9"])] {
        assert!(pairs.contains(&pair), "{pair}");
    }
    let names: Vec<&str> = pairs.iter().map(|pair| pair[0].as_str().unwrap()).collect();
    assert!(names.is_sorted(), "{names:?}");

    let response = client.get(vendor.url("/v1/messages")).send().await.unwrap();
    let failure = http_failure(response, "Acme", read_http_failure, &clock).await;
    assert_eq!(failure.message, "Acme API request failed with HTTP 502: <html>Bad gateway</html>");
    assert_eq!(failure.code, Some(ErrorCode::Overloaded));
    assert_eq!(failure.retry_after, None);
    let record: serde_json::Value = serde_json::from_str(failure.diagnostics.unwrap().upstream.as_deref().unwrap()).unwrap();
    assert_eq!(record["body"], json!("<html>Bad gateway</html>"));
}

#[test]
fn retry_after_names_seconds_after_receipt_or_an_http_date_and_nothing_else() {
    let received = time("2026-09-18T14:00:00.000Z");
    let named = [
        ("120", "2026-09-18T14:02:00.000Z"),
        (" 120\t", "2026-09-18T14:02:00.000Z"),
        ("0", "2026-09-18T14:00:00.000Z"),
        ("1.5", "2026-09-18T14:00:01.500Z"),
        ("1.2349", "2026-09-18T14:00:01.234Z"),
        ("Tue, 22 Sep 2026 07:37:39 GMT", "2026-09-22T07:37:39.000Z"),
        ("Tuesday, 22-Sep-26 07:37:39 GMT", "2026-09-22T07:37:39.000Z"),
        ("Tue Sep 22 07:37:39 2026", "2026-09-22T07:37:39.000Z"),
    ];
    for (value, moment) in named {
        assert_eq!(retry_at(value, received), Some(time(moment)), "{value}");
    }
    for value in ["1e3", "0x10", "2026-09-22T07:37:39Z", "-5", "+5", "soon", "", "1.", ".5", "99999999999999999999"] {
        assert_eq!(retry_at(value, received), None, "{value}");
    }
}

#[test]
fn the_standard_reading_names_a_time_only_for_an_http_record_with_the_header() {
    let received = time("2026-09-18T14:00:00.000Z");
    let with_header = HttpFailureRecord {
        status: 429,
        headers: vec![("retry-after".into(), "90".into())],
        body: "slow down".into(),
    };
    let facts = read_http_failure(&record(FailureSource::Http, &with_header.to_json()), received);
    assert_eq!(facts, ProviderFailureFacts { retry_at: Some(time("2026-09-18T14:01:30.000Z")) });

    let without = HttpFailureRecord { headers: Vec::new(), ..with_header };
    let unread = [
        record(FailureSource::Http, &without.to_json()),
        record(FailureSource::Stream, r#"{"retry-after":"90"}"#),
        record(FailureSource::Http, "not a record"),
    ];
    for diagnostics in unread {
        assert_eq!(read_http_failure(&diagnostics, received).retry_at, None, "{diagnostics:?}");
    }
}

#[test]
fn the_wait_is_set_only_when_the_reader_names_a_time() {
    let now = time("2026-09-18T14:00:00.000Z");
    let failed = ProviderFailure::protocol("x", "x");
    let later: fn(&ProviderErrorDiagnostics, Timestamp) -> ProviderFailureFacts =
        |_, _| ProviderFailureFacts { retry_at: Some("2026-09-18T14:00:30.000Z".parse().unwrap()) };
    let earlier: fn(&ProviderErrorDiagnostics, Timestamp) -> ProviderFailureFacts =
        |_, _| ProviderFailureFacts { retry_at: Some("2026-09-18T13:00:00.000Z".parse().unwrap()) };
    let none: fn(&ProviderErrorDiagnostics, Timestamp) -> ProviderFailureFacts =
        |_, _| ProviderFailureFacts { retry_at: None };

    assert_eq!(failed.clone().with_retry_wait(later, now).retry_after, Some(Duration::from_secs(30)));
    assert_eq!(failed.clone().with_retry_wait(earlier, now).retry_after, Some(Duration::ZERO));
    assert_eq!(failed.clone().with_retry_wait(none, now), failed);
    let bare = ProviderFailure { diagnostics: None, ..failed };
    assert_eq!(bare.clone().with_retry_wait(later, now), bare);
}

#[tokio::test]
async fn a_request_without_an_answer_is_overloaded_with_no_record_and_no_endpoint() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let error = reqwest::Client::new()
        .get(format!("http://{address}/v1/messages"))
        .send()
        .await
        .unwrap_err();
    let failure = ProviderFailure::transport("Acme", error);
    assert_eq!(failure.code, Some(ErrorCode::Overloaded));
    assert!(failure.message.starts_with("Acme API request failed: "), "{}", failure.message);
    assert!(!failure.message.contains("127.0.0.1"), "{}", failure.message);
    let diagnostics = failure.diagnostics.unwrap();
    assert_eq!((diagnostics.source, diagnostics.upstream), (FailureSource::Transport, None));
}
