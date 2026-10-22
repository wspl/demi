//! The Codex catalog (`crates/provider-codex/src/models.rs`).

use std::sync::Arc;

use demi_provider_codex::{CodexProvider, TransportMode};
use demi_provider_common::{
    Provider,
    testing::{ManualClock, MockResponse, MockVendor, guarded},
};
use demi_shared_types::{ProviderModelList, ServiceTier};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::{
    NOW, NOW_SECONDS, NPM_LATEST, access_token, fresh_token, pool_with, provider, provider_at,
    releases_at, secret,
};

const MODELS: &str = "/backend-api/codex/models";

/// What a catalog read that must not wait for the registry is guarded as.
const UNWAITED: &str = "a catalog that does not wait for the registry";

/// The version the provider sends before it has read the registry.
const BUILT_IN: &str = "0.162.1";

/// The registry's record of `@openai/codex` naming `version` as `latest`.
fn latest(version: &str) -> MockResponse {
    let record = json!({ "name": "@openai/codex", "version": version, "bin": { "codex": "bin/codex.js" } });
    MockResponse::status(200)
        .header("content-type", "application/json")
        .chunk(record.to_string())
}

/// The `client_version` of each catalog request so far.
fn catalog_versions(vendor: &MockVendor) -> Vec<String> {
    vendor
        .requests()
        .iter()
        .filter(|request| request.uri.path() == MODELS)
        .map(|request| {
            let query = request.uri.query().unwrap_or_default();
            query.strip_prefix("client_version=").unwrap_or(query).to_owned()
        })
        .collect()
}

/// How many times the registry was read.
fn registry_reads(vendor: &MockVendor) -> usize {
    let requests = vendor.requests();
    requests.iter().filter(|request| request.uri.path() == NPM_LATEST).count()
}

/// Reads the catalog, answered with `body`, until a request carries
/// `version`, which a read of the registry in the background brings; the
/// catalog that request read.
async fn catalog_at(
    provider: &CodexProvider,
    vendor: &MockVendor,
    version: &str,
    body: &str,
) -> ProviderModelList {
    let polled = async {
        loop {
            vendor.respond_at(MODELS, answer_text(body));
            let list = provider.list_models().await.unwrap();
            if catalog_versions(vendor).last().map(String::as_str) == Some(version) {
                return list;
            }
        }
    };
    guarded(&format!("a catalog request at version {version}"), polled).await
}

fn fixture() -> Value {
    json!({ "models": [
        {
            "slug": "gpt-5.5", "visibility": "list", "priority": 2, "display_name": "GPT-5.5", "context_window": 272_000,
            "input_modalities": ["text", "image"], "tool_mode": "default",
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
    answer_text(&body.to_string())
}

fn answer_text(body: &str) -> MockResponse {
    MockResponse::status(200)
        .header("content-type", "application/json")
        .chunk(body.to_owned())
}

#[tokio::test]
async fn a_registry_that_fails_leaves_the_version_last_read_and_is_read_again_after_six_hours() {
    let vendor = MockVendor::start().await;
    let clock = Arc::new(ManualClock::new(NOW.parse().unwrap()));
    // A token that outlives the hours the test moves the clock by.
    let token = access_token(NOW_SECONDS + 24 * 3_600);
    let pool = pool_with(secret(&token, "refresh-1", NOW)).await;
    let releases = releases_at(&vendor, clock.clone());
    let provider = provider_at(&vendor, &pool, TransportMode::Sse, clock.clone(), releases);

    // The registry cannot be reached: the catalog asks with the built-in
    // version, without waiting for the read.
    let (open, gate) = watch::channel(false);
    vendor.respond_at(NPM_LATEST, MockResponse::status(503).held(gate));
    vendor.respond_at(MODELS, answer(&fixture()));
    guarded(UNWAITED, provider.list_models()).await.unwrap();
    assert_eq!(catalog_versions(&vendor), [BUILT_IN]);
    open.send(true).unwrap();

    // Once the registry answers, a later catalog asks with what it named.
    vendor.respond_at(NPM_LATEST, latest("0.163.0"));
    catalog_at(&provider, &vendor, "0.163.0", &fixture().to_string()).await;
    let reads = registry_reads(&vendor);

    // Six hours later the version is read again in the background, and the
    // catalog keeps asking with the last one read while that read waits and
    // after it fails.
    clock.advance(jiff::SignedDuration::from_hours(6));
    let (open, gate) = watch::channel(false);
    vendor.respond_at(NPM_LATEST, MockResponse::status(503).held(gate));
    vendor.respond_at(MODELS, answer(&fixture()));
    let before = vendor.requests().len();
    guarded(UNWAITED, provider.list_models()).await.unwrap();
    // The catalog request and the registry's.
    vendor.received(before + 2).await;
    assert_eq!(registry_reads(&vendor), reads + 1);
    assert_eq!(catalog_versions(&vendor).last().unwrap(), "0.163.0");
    open.send(true).unwrap();
    vendor.respond_at(NPM_LATEST, latest("0.164.0"));
    let before = catalog_versions(&vendor).len();
    catalog_at(&provider, &vendor, "0.164.0", &fixture().to_string()).await;
    let since = &catalog_versions(&vendor)[before..];
    assert!(
        since.iter().all(|version| ["0.163.0", "0.164.0"].contains(&version.as_str())),
        "{since:?}"
    );
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

    let requests = vendor.requests();
    let request = requests
        .iter()
        .find(|request| request.uri.path() == MODELS)
        .unwrap();
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
        (gpt.supports_tools, gpt.supports_reasoning),
        (Some(true), Some(true))
    );
    // The backend documents no file part: a model reads images, and no PDF
    // (`crates/shared-types/src/model.rs`).
    assert_eq!(
        gpt.accepted_extensions.as_deref(),
        Some(&demi_shared_types::IMAGE_FILE_EXTENSIONS[..])
    );
    assert_eq!(
        gpt.supported_thinking_efforts.as_deref(),
        Some(&["low", "medium", "high", "xhigh", "ultra"].map(String::from)[..])
    );
    assert_eq!(gpt.can_disable_thinking, Some(false));
    let fast = ServiceTier {
        id: "priority".into(),
        label: "Fast".into(),
        description: Some("1.5x speed, increased usage".into()),
        fast: true,
    };
    assert_eq!(gpt.service_tiers, [fast]);
    let mini = &list.models[2];
    assert_eq!(
        (mini.supports_tools, mini.supports_reasoning),
        (None, Some(false))
    );
    assert_eq!(mini.accepted_extensions.as_deref(), Some(&[][..]));
    // A tool field present even as null names a tool.
    assert_eq!(
        (
            list.models[0].supports_tools,
            list.models[0].default_service_tier_id.as_deref()
        ),
        (Some(true), Some("priority"))
    );
}
