//! The Anthropic Messages API provider at its boundaries: the request it
//! sends to a scripted vendor, the events it makes of recorded event streams,
//! its failures, its cancellation and what it says about itself. No test
//! calls the real vendor.

mod cancellation;
mod failures;
mod metadata;
mod request;
mod stream;

use std::sync::Arc;

use demi_provider::{
    InferenceRequest, Provider, ProviderEvent, ProviderRuntime, RuntimeEnv, Secret,
    testing::{FixedClock, MockResponse, MockVendor},
};
use demi_provider_anthropic_api::{AnthropicConfig, AnthropicProvider};
use futures_util::StreamExt;

/// When the scripted vendor answers.
pub(crate) const NOW: &str = "2026-09-18T14:00:00.000Z";

/// A provider whose endpoint is `base` on the scripted vendor.
pub(crate) fn provider_at(vendor: &MockVendor, base: &str) -> AnthropicProvider {
    let config = AnthropicConfig {
        id: "anthropic-work".into(),
        display_name: "Work".into(),
        api_key: Secret::try_from("sk-ant-test".to_owned()).unwrap(),
        base_url: Some(vendor.url(base).parse().unwrap()),
    };
    AnthropicProvider::new(config, Arc::new(FixedClock(NOW.parse().unwrap())))
}

/// A runtime of a provider at `/v1` on the scripted vendor.
pub(crate) fn runtime(vendor: &MockVendor) -> Box<dyn ProviderRuntime> {
    provider_at(vendor, "/v1")
        .runtime(RuntimeEnv {
            http: reqwest::Client::new(),
        })
        .unwrap()
}

pub(crate) async fn run(runtime: &mut dyn ProviderRuntime, request: InferenceRequest) -> Vec<ProviderEvent> {
    runtime.run(request).collect().await
}

/// A recorded event stream: each payload as the vendor frames it, with its
/// `event:` line.
pub(crate) fn recorded(payloads: &[serde_json::Value]) -> MockResponse {
    let text: String = payloads
        .iter()
        .map(|payload| format!("event: {}\ndata: {payload}\n\n", payload["type"].as_str().unwrap()))
        .collect();
    MockResponse::event_stream(text)
}

/// A stream that ends at once, with no usage.
pub(crate) fn stop() -> MockResponse {
    recorded(&[serde_json::json!({ "type": "message_stop" })])
}
