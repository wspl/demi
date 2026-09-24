//! The events a run makes of what the endpoint streams back, and its
//! failures (`providers.md` § A run, `failures-and-recovery.md`).

use std::time::Duration;

use demi_core::{FailureSource, TokenUsage, WireApi};
use demi_provider::{
    ErrorCode, Provider, ProviderEvent, RuntimeEnv, Secret,
    testing::{FixedClock, MockResponse, MockVendor, inference_request, sse_body},
};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider, VendorPolicy};
use serde_json::json;
use std::sync::Arc;

use crate::{NOW, run, runtime};

async fn events_of(wire: WireApi, response: MockResponse) -> Vec<ProviderEvent> {
    let vendor = MockVendor::start().await;
    vendor.respond(response);
    run(
        runtime(&vendor, wire, VendorPolicy::default()).as_mut(),
        inference_request(),
    )
    .await
}

#[tokio::test]
async fn the_responses_runtime_maps_the_stream_the_endpoint_sends_back() {
    let body = sse_body(&[
        json!({ "type": "response.output_text.delta", "delta": "hi" }),
        json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 3, "output_tokens": 1 } } }),
    ]);
    let events = events_of(WireApi::Responses, MockResponse::event_stream(body)).await;
    let usage = TokenUsage {
        input_tokens: 3,
        output_tokens: 1,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    };
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("hi".into()),
            ProviderEvent::Response(usage)
        ]
    );
}

#[tokio::test]
async fn the_chat_completions_runtime_maps_the_stream_the_endpoint_sends_back() {
    let body = format!(
        "{}data: [DONE]\n\n",
        sse_body(&[json!({ "choices": [{ "delta": { "content": "hi" } }] })])
    );
    let events = events_of(WireApi::ChatCompletions, MockResponse::event_stream(body)).await;
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("hi".into()),
            ProviderEvent::Response(TokenUsage::default())
        ]
    );
}

#[tokio::test]
async fn a_malformed_stream_event_fails_the_run_naming_its_field() {
    let body = sse_body(&[
        json!({ "type": "response.output_item.done", "item": { "type": "message", "content": 42 } }),
    ]);
    let events = events_of(WireApi::Responses, MockResponse::event_stream(body)).await;
    let [ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(failure.code, None);
    assert!(
        failure
            .message
            .starts_with("OpenAI API stream sent a frame Demi cannot read"),
        "{}",
        failure.message
    );
    assert!(failure.message.contains("content"), "{}", failure.message);
    assert_eq!(
        failure.diagnostics.as_ref().unwrap().source,
        FailureSource::Stream
    );
}

#[tokio::test]
async fn a_refused_request_fails_with_the_vendor_record_and_the_wait_it_names() {
    let body = r#"{"error":{"message":"Rate limit reached","type":"requests","code":"rate_limit_exceeded"}}"#;
    for wire in [WireApi::Responses, WireApi::ChatCompletions] {
        let events = events_of(
            wire,
            MockResponse::status(429)
                .header("retry-after", "20")
                .chunk(body),
        )
        .await;
        let [ProviderEvent::Error(failure)] = events.as_slice() else {
            panic!("{events:?}");
        };
        assert_eq!(
            failure.message,
            format!("OpenAI API request failed with HTTP 429: {body}")
        );
        assert_eq!(
            (failure.code.clone(), failure.retry_after),
            (Some(ErrorCode::RateLimit), Some(Duration::from_secs(20)))
        );
        assert_eq!(failure.diagnostics.as_ref().unwrap().http_status, Some(429));
    }
}

#[tokio::test]
async fn a_request_without_an_answer_fails_as_overloaded() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let config = OpenAiConfig {
        id: "openai".into(),
        display_name: "OpenAI API".into(),
        api_key: Secret::try_from("sk-test".to_owned()).unwrap(),
        base_url: Some(format!("http://{address}/v1").parse().unwrap()),
        wire: WireApi::Responses,
        policy: VendorPolicy::default(),
    };
    let provider = OpenAiProvider::new(config, Arc::new(FixedClock(NOW.parse().unwrap())));
    let mut runtime = provider
        .runtime(RuntimeEnv {
            http: reqwest::Client::new(),
        })
        .unwrap();
    let events = run(runtime.as_mut(), inference_request()).await;
    let [ProviderEvent::Error(failure)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(failure.code, Some(ErrorCode::Overloaded));
    assert!(
        !failure.message.contains(&address.to_string()),
        "{}",
        failure.message
    );
    assert_eq!(
        failure.diagnostics.as_ref().unwrap().source,
        FailureSource::Transport
    );
}
