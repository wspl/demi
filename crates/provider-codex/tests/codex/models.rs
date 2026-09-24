//! The Codex catalog (`models.md` § The Codex catalog).

use demi_core::ServiceTier;
use demi_provider::{
    CatalogError, Provider,
    testing::{MockResponse, MockVendor},
};
use demi_provider_codex::TransportMode;
use serde_json::{Value, json};

use crate::{NOW, fresh_token, pool_with, provider, secret};

const MODELS: &str = "/backend-api/codex/models";

fn fixture() -> Value {
    json!({ "models": [
        {
            "slug": "gpt-5.5", "visibility": "list", "priority": 2, "display_name": "GPT-5.5", "context_window": 272_000,
            "input_modalities": ["text", "image"], "tool_mode": "default", "default_reasoning_level": "medium",
            "service_tiers": [{ "id": "priority", "name": "Fast", "description": "1.5x speed, increased usage" }],
            "additional_speed_tiers": ["fast"],
            "supported_reasoning_levels": [{ "effort": "low" }, { "effort": "medium" }, { "effort": "high" }, { "effort": "xhigh" }, { "effort": "ultra" }],
        },
        { "slug": "gpt-5.4-mini", "visibility": "list", "priority": 3, "display_name": "GPT-5.4-Mini", "input_modalities": ["text"], "supported_reasoning_levels": [] },
        { "slug": "first", "visibility": "list", "priority": 1, "display_name": "First", "supported_reasoning_levels": [{ "effort": "low" }], "default_service_tier": "priority", "apply_patch_tool_type": null },
        { "slug": "codex-auto-review", "display_name": "Codex Auto Review", "visibility": "hide", "priority": 0, "supported_reasoning_levels": [{ "effort": "medium" }] },
        { "slug": "internal", "display_name": "Internal", "visibility": "none", "priority": 0, "supported_reasoning_levels": [] },
    ] })
}

fn answer(body: &Value) -> MockResponse {
    MockResponse::status(200)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

#[tokio::test]
async fn the_catalog_lists_the_pickers_models_by_priority_with_their_efforts_and_tiers() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(MODELS, answer(&fixture()));
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let list = provider(&vendor, &pool, TransportMode::Sse)
        .list_models()
        .await
        .unwrap();

    let request = &vendor.requests()[0];
    assert_eq!(
        request.uri.to_string(),
        "/backend-api/codex/models?client_version=0.153.4"
    );
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {}", fresh_token()).as_str())
    );
    assert_eq!(
        (
            request.header("chatgpt-account-id"),
            request.header("accept")
        ),
        (Some("acct-1"), Some("application/json"))
    );

    let ids: Vec<&str> = list.models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(ids, ["first", "gpt-5.5", "gpt-5.4-mini"]);
    assert_eq!(list.default_model_id.as_deref(), Some("first"));
    assert_eq!(
        (list.source_fetched_at, list.stale),
        (NOW.parse().unwrap(), false)
    );
    let gpt = &list.models[1];
    assert_eq!(
        (
            gpt.display_name.as_str(),
            gpt.context_window,
            gpt.output_limit
        ),
        ("GPT-5.5", Some(272_000), None)
    );
    assert_eq!(
        (
            gpt.supports_tools,
            gpt.supports_attachments,
            gpt.supports_reasoning
        ),
        (Some(true), Some(true), Some(true))
    );
    assert_eq!(
        gpt.supported_thinking_efforts.as_deref(),
        Some(&["low", "medium", "high", "xhigh", "ultra"].map(String::from)[..])
    );
    assert_eq!(
        (
            gpt.default_thinking_effort.as_deref(),
            gpt.can_disable_thinking
        ),
        (Some("medium"), Some(false))
    );
    let fast = ServiceTier {
        id: "priority".into(),
        label: "Fast".into(),
        description: Some("1.5x speed, increased usage".into()),
        fast: true,
    };
    assert_eq!(gpt.service_tiers, [fast]);
    let mini = &list.models[2];
    assert_eq!(
        (
            mini.supports_tools,
            mini.supports_attachments,
            mini.supports_reasoning
        ),
        (None, Some(false), Some(false))
    );
    // A tool field present even as null names a tool.
    assert_eq!(
        (
            list.models[0].supports_tools,
            list.models[0].default_service_tier_id.as_deref()
        ),
        (Some(true), Some("priority"))
    );
}

#[tokio::test]
async fn a_catalog_refused_with_401_refreshes_once_and_reads_again() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(MODELS, MockResponse::status(401).chunk("unauthorized"));
    let refreshed = json!({ "access_token": "new-access", "refresh_token": "refresh-2" });
    vendor.respond_at(
        "/oauth/token",
        MockResponse::status(200).chunk(refreshed.to_string()),
    );
    vendor.respond_at(MODELS, answer(&fixture()));
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let list = provider(&vendor, &pool, TransportMode::Sse)
        .list_models()
        .await
        .unwrap();
    assert_eq!(list.models.len(), 3);
    let requests = vendor.requests();
    let authorizations: Vec<Option<&str>> = requests
        .iter()
        .filter(|request| request.uri.path() == MODELS)
        .map(|request| request.header("authorization"))
        .collect();
    assert_eq!(
        authorizations,
        [
            Some(format!("Bearer {}", fresh_token()).as_str()),
            Some("Bearer new-access")
        ]
    );
}

#[tokio::test]
async fn a_catalog_that_cannot_be_read_is_refused_and_one_that_fails_is_unavailable() {
    let vendor = MockVendor::start().await;
    let mut malformed = fixture();
    malformed["models"][0]["supported_reasoning_levels"] = json!([{ "effort": 3 }]);
    vendor.respond_at(MODELS, answer(&malformed));
    let mut mistyped = fixture();
    mistyped["models"][0]["default_reasoning_level"] = json!(3);
    vendor.respond_at(MODELS, answer(&mistyped));
    vendor.respond_at(MODELS, MockResponse::status(503).chunk("overloaded"));
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    for field in ["effort", "default_reasoning_level"] {
        match provider.list_models().await {
            Err(CatalogError::Invalid(message)) => assert!(message.contains(field), "{message}"),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(
        provider.list_models().await,
        Err(CatalogError::Unavailable(
            "Codex models request failed with HTTP 503".into()
        ))
    );
}
