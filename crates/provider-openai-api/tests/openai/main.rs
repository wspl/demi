//! The OpenAI API provider at its boundaries: the requests it sends to a
//! scripted vendor over both wires and the answers it reads back, its
//! failures, its cancellation, what it says about itself, and an OpenAI or
//! xAI key's images and videos, and a compatible API's videos. No test
//! calls the real vendor.

mod cancellation;
mod failures;
mod images;
mod request;
mod videos;

use std::sync::Arc;

use demi_provider_common::VendorPolicy;
pub(crate) use demi_provider_common::testing::run;
use demi_provider_common::{
    InferenceRequest, Provider, ProviderRuntime, RuntimeEnv, Secret,
    testing::{FixedClock, MockResponse, MockVendor},
};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider};
use demi_shared_types::WireApi;

/// When the scripted vendor answers.
pub(crate) const NOW: &str = "2026-09-18T14:00:00.000Z";

/// A provider whose endpoint is `base` on the scripted vendor.
pub(crate) fn provider_at(
    vendor: &MockVendor,
    base: &str,
    wire: WireApi,
    policy: VendorPolicy,
) -> OpenAiProvider {
    provider_of(vendor, base, wire, policy, Some("openai"))
}

/// A provider of an entry added from vendor `vendor_id`, none for a
/// compatible API, whose endpoint is `base` on the scripted vendor.
pub(crate) fn provider_of(
    vendor: &MockVendor,
    base: &str,
    wire: WireApi,
    policy: VendorPolicy,
    vendor_id: Option<&str>,
) -> OpenAiProvider {
    let config = OpenAiConfig {
        api_key: Some(Secret::try_from("sk-test".to_owned()).unwrap()),
        base_url: Some(vendor.url(base).parse().unwrap()),
        wire,
        policy,
        vendor_id: vendor_id.map(str::to_owned),
        makes_images: false,
        makes_videos: false,
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
            http: demi_provider_common::ProviderHttp::new(reqwest::Client::new()),
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
