//! The OpenAI API provider at its boundaries: the requests it sends to a
//! scripted vendor over both wires and the answers it reads back, its
//! failures, its cancellation and what it says about itself. No test calls
//! the real vendor.

mod cancellation;
mod failures;
mod metadata;
mod request;

use std::sync::Arc;

use demi_shared_types::WireApi;
use demi_provider_common::{
    InferenceRequest, Provider, ProviderRuntime, RuntimeEnv, Secret,
    testing::{FixedClock, MockResponse, MockVendor},
};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider, VendorPolicy};
pub(crate) use demi_provider_common::testing::run;

/// When the scripted vendor answers.
pub(crate) const NOW: &str = "2026-09-18T14:00:00.000Z";

/// A provider whose endpoint is `base` on the scripted vendor.
pub(crate) fn provider_at(
    vendor: &MockVendor,
    base: &str,
    wire: WireApi,
    policy: VendorPolicy,
) -> OpenAiProvider {
    let config = OpenAiConfig {
        api_key: Secret::try_from("sk-test".to_owned()).unwrap(),
        base_url: Some(vendor.url(base).parse().unwrap()),
        wire,
        policy,
    };
    OpenAiProvider::new(config, Arc::new(FixedClock(NOW.parse().unwrap())))
}

/// A runtime of a provider at `/v1` on the scripted vendor.
pub(crate) fn runtime(
    vendor: &MockVendor,
    wire: WireApi,
    policy: VendorPolicy,
) -> Box<dyn ProviderRuntime> {
    provider_at(vendor, "/v1", wire, policy)
        .runtime(RuntimeEnv {
            http: reqwest::Client::new(),
        })
        .unwrap()
}

/// A stream that ends at once.
pub(crate) fn done() -> MockResponse {
    MockResponse::event_stream("data: [DONE]\n\n")
}

/// The body the vendor received for `request` over `wire`.
pub(crate) async fn body_of(
    wire: WireApi,
    policy: VendorPolicy,
    request: InferenceRequest,
) -> serde_json::Value {
    let vendor = MockVendor::start().await;
    vendor.respond(done());
    run(runtime(&vendor, wire, policy).as_mut(), request).await;
    vendor.requests()[0].json()
}
