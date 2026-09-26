//! The Google Gemini provider at its boundaries: the request it sends to a
//! scripted vendor, the events it makes of recorded streams, its failures,
//! its cancellation and what it says about itself. No test calls the real
//! vendor.

mod provider;
mod request;
mod stream;

use std::sync::Arc;

use demi_provider::{
    InferenceRequest, Provider, ProviderRuntime, RuntimeEnv, Secret,
    testing::{FixedClock, MockResponse, MockVendor, sse_body},
};
use demi_provider_google::{GoogleConfig, GoogleProvider};
pub(crate) use demi_provider::testing::run;

pub(crate) const NOW: &str = "2026-09-18T14:00:00.000Z";

pub(crate) fn provider_at(vendor: &MockVendor, base: &str) -> GoogleProvider {
    let config = GoogleConfig {
        api_key: Secret::try_from("google-key".to_owned()).unwrap(),
        base_url: Some(vendor.url(base).parse().unwrap()),
    };
    GoogleProvider::new(config, Arc::new(FixedClock(NOW.parse().unwrap())))
}

pub(crate) fn runtime(vendor: &MockVendor) -> Box<dyn ProviderRuntime> {
    provider_at(vendor, "/v1beta")
        .runtime(RuntimeEnv {
            http: reqwest::Client::new(),
        })
        .unwrap()
}

/// A stream of `chunks`, one frame each.
pub(crate) fn chunks(chunks: &[serde_json::Value]) -> MockResponse {
    MockResponse::event_stream(sse_body(chunks))
}

/// The body the vendor received for `request`.
pub(crate) async fn body_of(request: InferenceRequest) -> serde_json::Value {
    let vendor = MockVendor::start().await;
    vendor.respond(chunks(&[]));
    run(runtime(&vendor).as_mut(), request).await;
    vendor.requests()[0].json()
}
