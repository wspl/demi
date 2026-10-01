//! A run's failures on either wire: a refusal with the vendor's record and
//! wait, and a request that gets no answer (`failures-and-recovery.md`). The
//! events of a stream are the shared mappers' (`demi_provider_common::wire`).

use std::{sync::Arc, time::Duration};

use demi_shared_types::{FailureSource, WireApi};
use demi_provider_common::{
    ErrorCode, Provider, ProviderEvent, RuntimeEnv, Secret,
    testing::{FixedClock, MockResponse, MockVendor, inference_request},
};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider, VendorPolicy};

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
