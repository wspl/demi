//! The models.dev document (`crates/provider-common/src/models_dev.rs`), served
//! by a scripted vendor.
//!
//! The tests cover one copy per backend, revalidated conditionally, shared by
//! concurrent readers, kept as a stale copy after a failed request, and its
//! models mapped onto the catalog's shape.

use std::sync::Arc;
use std::time::Duration;

use demi_provider_common::models_dev::ModelsDevClient;
use demi_provider_common::testing::{FixedClock, MockResponse, MockVendor};
use demi_shared_types::Timestamp;
use serde_json::{Value, json};

const NOW: &str = "2026-09-08T00:00:00.000Z";

fn client(vendor: &MockVendor) -> ModelsDevClient {
    ModelsDevClient::new(
        reqwest::Client::new(),
        vendor.url("/api.json").parse().unwrap(),
        Arc::new(FixedClock(NOW.parse().unwrap())),
    )
}

fn document() -> Value {
    json!({
        "deepseek": {
            "id": "deepseek",
            "name": "DeepSeek",
            "npm": "@ai-sdk/openai-compatible",
            "api": "https://api.deepseek.com",
            "env": ["DEEPSEEK_API_KEY"],
            "models": {
                "deepseek-v4": {
                    "name": "DeepSeek V4",
                    "description": "The flagship",
                    "reasoning": true,
                    "reasoning_options": [
                        { "type": "budget", "min": 1024 },
                        { "type": "effort", "values": [null, "low", "", "high"] }
                    ],
                    "tool_call": true,
                    // The flag says it reads files; the modalities say
                    // which: images, and no PDF.
                    "attachment": true,
                    "modalities": { "input": ["text", "image"], "output": ["text"] },
                    "release_date": "2026-06-01",
                    "limit": { "context": 128_000, "output": 32_000 },
                    "cost": { "input": 0.3, "output": 1.2, "cache_read": 0.03 }
                },
                "deepseek-v4-flash": {
                    "limit": { "context": 128_000.5, "output": 0 }
                },
                "deepseek-v4-docs": {
                    "attachment": true,
                    "modalities": { "input": ["text", "image", "pdf"], "output": ["text"] }
                },
                "deepseek-v4-text": {
                    "modalities": { "input": ["text"], "output": ["text"] }
                }
            }
        },
        "minimax": { "id": "minimax", "name": "MiniMax", "models": {} }
    })
}

fn served(document: &Value) -> MockResponse {
    MockResponse::status(200)
        .header("content-type", "application/json")
        .header("etag", "\"v1\"")
        .header("last-modified", "Mon, 07 Sep 2026 12:00:00 GMT")
        .chunk(document.to_string())
}

#[tokio::test]
async fn a_catalog_read_revalidates_the_copy_and_a_304_keeps_its_content_date() {
    let vendor = MockVendor::start().await;
    let client = client(&vendor);
    vendor.respond(served(&document()));
    // Concurrent readers share one request.
    let (first, second, third) =
        tokio::join!(client.refreshed(), client.refreshed(), client.current());
    let first = first.unwrap();
    assert_eq!(
        (first.fetched_at, first.stale),
        (NOW.parse::<Timestamp>().unwrap(), false)
    );
    assert!(first.warnings.is_empty());
    for other in [second.unwrap(), third.unwrap()] {
        assert_eq!((other.fetched_at, other.stale), (first.fetched_at, false));
    }
    assert_eq!(vendor.requests().len(), 1);
    assert_eq!(
        vendor.requests()[0].header("accept"),
        Some("application/json")
    );

    // The vendor list reuses a copy confirmed less than a day ago.
    client.current().await.unwrap();
    assert_eq!(vendor.requests().len(), 1);

    // A catalog read asks again, conditionally, and a 304 keeps the copy.
    vendor.respond(MockResponse::status(304));
    let confirmed = client.refreshed().await.unwrap();
    let revalidation = &vendor.requests()[1];
    assert_eq!(revalidation.header("if-none-match"), Some("\"v1\""));
    assert_eq!(
        revalidation.header("if-modified-since"),
        Some("Mon, 07 Sep 2026 12:00:00 GMT")
    );
    assert_eq!(
        (confirmed.fetched_at, confirmed.stale),
        (first.fetched_at, false)
    );
    assert_eq!(confirmed.vendor_models("deepseek").unwrap().models.len(), 4);

    // A day after the confirmation, the vendor list asks again too.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(24 * 60 * 60)).await;
    vendor.respond(MockResponse::status(304));
    client.current().await.unwrap();
    assert_eq!(vendor.requests().len(), 3);
}

#[tokio::test]
async fn a_failed_read_returns_the_kept_copy_marked_stale_and_without_a_copy_it_fails() {
    let vendor = MockVendor::start().await;
    let client = client(&vendor);
    vendor.respond(MockResponse::status(500));
    assert_eq!(
        client.refreshed().await.unwrap_err().to_string(),
        "models.dev catalog request failed with HTTP 500"
    );
    vendor.respond(served(
        &json!({ "vendor": { "id": "vendor", "name": "Vendor", "models": [] } }),
    ));
    let malformed = client.refreshed().await.unwrap_err().to_string();
    assert!(malformed.contains("vendor.models"), "{malformed}");

    vendor.respond(served(&document()));
    let good = client.refreshed().await.unwrap();
    vendor.respond(MockResponse::status(503));
    let stale = client.refreshed().await.unwrap();
    assert_eq!((stale.fetched_at, stale.stale), (good.fetched_at, true));
    assert_eq!(
        stale.warnings,
        ["Using stale models.dev catalog: models.dev catalog request failed with HTTP 503"]
    );
    let list = stale.vendor_models("deepseek").unwrap();
    assert_eq!(
        (list.stale, list.warnings.clone()),
        (true, stale.warnings.clone())
    );
    // A document that cannot be read after a good one is a failed read too.
    vendor.respond(served(&json!([])));
    assert!(client.refreshed().await.unwrap().stale);
}
