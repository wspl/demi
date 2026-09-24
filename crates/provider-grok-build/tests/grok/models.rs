//! The Grok Build catalog (`models.md` § Directories).

use demi_provider::{
    CatalogError, Provider,
    testing::{MockResponse, MockVendor},
};
use serde_json::json;

use crate::{ACCOUNT, NOW, json_answer, pool_with, provider, secret};

const MODELS: &str = "/v1/models";

#[tokio::test]
async fn the_catalog_reads_the_proxys_models_each_time_from_an_envelope_or_a_bare_list() {
    let vendor = MockVendor::start().await;
    let envelope = json!({ "object": "list", "data": [
        {
            "id": "frontier", "name": "Frontier", "description": "frontier", "context_window": 500_000,
            "supports_reasoning_effort": true, "reasoning_effort": "medium",
            "reasoning_efforts": [{ "id": "high", "value": "high", "default": true }, { "id": "medium", "default": false }, { "value": "low" }],
        },
        { "model": "fast", "context_window": 200_000 },
        { "name": "no id" },
    ] });
    vendor.respond_at(MODELS, json_answer(200, envelope));
    vendor.respond_at(MODELS, json_answer(200, json!([{ "id": "grok-test" }])));
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));

    let list = provider.list_models().await.unwrap();
    let ids: Vec<&str> = list.models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(ids, ["frontier", "fast"]);
    assert_eq!(
        (
            list.default_model_id.as_deref(),
            list.source_fetched_at,
            list.stale
        ),
        (Some("frontier"), NOW.parse().unwrap(), false)
    );
    let frontier = &list.models[0];
    assert_eq!(
        (frontier.display_name.as_str(), frontier.context_window),
        ("Frontier", Some(500_000))
    );
    assert_eq!(
        frontier.supported_thinking_efforts.as_deref(),
        Some(&["high", "medium", "low"].map(String::from)[..])
    );
    assert_eq!(
        (
            frontier.default_thinking_effort.as_deref(),
            frontier.supports_reasoning
        ),
        (Some("high"), Some(true))
    );
    let fast = &list.models[1];
    assert_eq!(
        (
            fast.display_name.as_str(),
            fast.supports_reasoning,
            fast.supported_thinking_efforts.as_ref()
        ),
        ("fast", None, None)
    );
    assert!(list.models.iter().all(
        |model| model.supports_attachments == Some(true) && model.supports_tools == Some(true)
    ));
    let sent = &vendor.requests()[0];
    assert_eq!(
        (sent.header("authorization"), sent.header("accept")),
        (Some("Bearer session-token"), Some("application/json"))
    );

    // Every read asks the proxy again.
    let second = provider.list_models().await.unwrap();
    assert_eq!(second.models[0].id, "grok-test");
    assert_eq!(vendor.requests().len(), 2);
}

#[tokio::test]
async fn a_catalog_that_cannot_be_read_is_a_failed_refresh() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(
        MODELS,
        json_answer(
            200,
            json!({ "data": [{ "id": "frontier", "context_window": "wide" }] }),
        ),
    );
    vendor.respond_at(MODELS, json_answer(200, json!({ "data": [] })));
    vendor.respond_at(MODELS, MockResponse::status(503).chunk("down"));
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    match provider.list_models().await {
        Err(CatalogError::Invalid(message)) => {
            assert!(message.contains("context_window"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        provider.list_models().await,
        Err(CatalogError::Invalid(
            "Grok Build models answer lists no model".into()
        ))
    );
    assert_eq!(
        provider.list_models().await,
        Err(CatalogError::Unavailable(
            "Grok Build models request failed with HTTP 503".into()
        ))
    );
    let staged = crate::provider(
        &vendor,
        &demi_provider::credentials::MemoryCredentialPool::new(),
        None,
    );
    assert_eq!(
        staged.list_models().await,
        Err(CatalogError::Unauthenticated(
            "No Grok account is signed in".into()
        ))
    );
}
