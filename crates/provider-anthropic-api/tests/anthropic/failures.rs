//! How a run fails when the vendor refuses the request, does not answer, or
//! breaks off (`failures-and-recovery.md`).

use std::time::Duration;

use demi_core::FailureSource;
use demi_provider::{
    ErrorCode, ProviderEvent, ProviderFailure, Provider, RuntimeEnv, Secret,
    testing::{FixedClock, MockResponse, MockVendor, inference_request},
};
use demi_provider_anthropic_api::{AnthropicConfig, AnthropicProvider};
use std::sync::Arc;

use crate::{NOW, run, runtime};

async fn failure_of(response: MockResponse) -> ProviderFailure {
    let vendor = MockVendor::start().await;
    vendor.respond(response);
    let events = run(runtime(&vendor).as_mut(), inference_request()).await;
    match <[ProviderEvent; 1]>::try_from(events) {
        Ok([ProviderEvent::Error(failure)]) => failure,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_refused_request_fails_with_the_vendor_record_and_the_wait_it_names() {
    let body = r#"{"type":"error","error":{"type":"rate_limit_error","message":"Number of request tokens has exceeded your per-minute rate limit"}}"#;
    let failure = failure_of(
        MockResponse::status(429)
            .header("content-type", "application/json")
            .header("retry-after", "30")
            .header("request-id", "req_1")
            .chunk(body),
    )
    .await;
    assert_eq!(failure.message, format!("Anthropic API request failed with HTTP 429: {body}"));
    assert_eq!(failure.code, Some(ErrorCode::RateLimit));
    assert_eq!(failure.retry_after, Some(Duration::from_secs(30)));
    let diagnostics = failure.diagnostics.unwrap();
    assert_eq!((diagnostics.source, diagnostics.http_status), (FailureSource::Http, Some(429)));
    let record: serde_json::Value = serde_json::from_str(diagnostics.upstream.as_deref().unwrap()).unwrap();
    assert_eq!(record["body"], body);
    assert!(record["headers"].as_array().unwrap().contains(&serde_json::json!(["request-id", "req_1"])));
}

#[tokio::test]
async fn the_status_decides_the_code_of_a_refusal() {
    let cases = [
        (529, r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#, Some(ErrorCode::Overloaded)),
        (401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#, Some(ErrorCode::AuthExpired)),
        (400, r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 213000 tokens > 200000 maximum"}}"#, Some(ErrorCode::ContextLengthExceeded)),
        (404, r#"{"type":"error","error":{"type":"not_found_error","message":"model: claude-x"}}"#, None),
    ];
    for (status, body, code) in cases {
        let failure = failure_of(MockResponse::status(status).chunk(body)).await;
        assert_eq!(failure.code, code, "{status}");
        assert_eq!(failure.retry_after, None, "{status}");
    }
}

#[tokio::test]
async fn a_request_without_an_answer_fails_as_overloaded() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let config = AnthropicConfig {
        id: "anthropic".into(),
        display_name: "Anthropic API".into(),
        api_key: Secret::try_from("sk-ant-test".to_owned()).unwrap(),
        base_url: Some(format!("http://{address}/v1").parse().unwrap()),
    };
    let provider = AnthropicProvider::new(config, Arc::new(FixedClock(NOW.parse().unwrap())));
    let mut runtime = provider.runtime(RuntimeEnv { http: reqwest::Client::new() }).unwrap();
    let events = run(runtime.as_mut(), inference_request()).await;
    let [ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(failure.code, Some(ErrorCode::Overloaded));
    assert!(failure.message.starts_with("Anthropic API request failed: "), "{}", failure.message);
    assert!(!failure.message.contains(&address.to_string()), "{}", failure.message);
    assert_eq!(failure.diagnostics.as_ref().unwrap().source, FailureSource::Transport);
}

#[tokio::test]
async fn a_stream_that_breaks_off_fails_as_overloaded_after_what_arrived() {
    let vendor = MockVendor::start().await;
    vendor.respond(
        MockResponse::event_stream(
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n",
        )
        .break_off(),
    );
    let events = run(runtime(&vendor).as_mut(), inference_request()).await;
    let [ProviderEvent::TextDelta(text), ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(text, "partial");
    assert_eq!(failure.code, Some(ErrorCode::Overloaded));
    assert_eq!(failure.diagnostics.as_ref().unwrap().source, FailureSource::Transport);
}
