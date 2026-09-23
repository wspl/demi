//! Provider entries (`web-api.md` § Model configuration and provider
//! inspection, `providers.md` § Credential vault, `models.md`): API-key
//! entries sealed at rest and answered without their key, the vendor list
//! from a scripted models.dev, each catalog source with its cache, and the
//! provider test through a real family against a scripted vendor.

use std::sync::Arc;
use std::time::Duration;

use demi_backend::FamilyRegistry;
use demi_provider::CatalogError;
use demi_provider::testing::{MockResponse, MockVendor};
use demi_web_api::auth::Role;
use demi_web_api::error::ErrorCode;
use demi_web_api::providers::{
    Availability, CredentialKind, ModelCatalog, ProviderAnswer, ProviderDto, Providers, TestResult, VendorCatalog,
    WireApi,
};
use demi_web_api::settings::InstanceMode;
use jiff::SignedDuration;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::families::{Directory, ScriptedKey, catalog};
use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD, Session, TestBackend};

fn configured(id: &str) -> Value {
    json!({
        "id": id,
        "displayName": format!("{id} display"),
        "contextWindow": 64_000,
        "outputLimit": 4_000,
        "thinkingEfforts": ["low", "high"],
        "acceptedExtensions": ["pdf"],
        "fastTier": "priority"
    })
}

async fn create(backend: &TestBackend, session: &Session, body: Value) -> ProviderDto {
    let answer = backend.post("/api/providers", Some(session), body).await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    answer.json::<ProviderAnswer>().provider
}

async fn catalog_of(backend: &TestBackend, session: &Session, query: &str) -> ModelCatalog {
    let answer = backend.get(&format!("/api/models{query}"), Some(session)).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    answer.json()
}

fn scripted(directory: &Arc<Directory>) -> FamilyRegistry {
    FamilyRegistry::builtin().with(
        "scripted",
        ScriptedKey {
            directory: directory.clone(),
            wires: &[],
        },
    )
}

#[tokio::test]
async fn an_api_key_entry_is_sealed_at_rest_and_answered_without_its_key() {
    let directory = Arc::new(Directory::default());
    let families = crate::accounts::scripts(None).families.with(
        "scripted",
        ScriptedKey {
            directory: directory.clone(),
            wires: &[],
        },
    );
    let harness = Harness::new().with_mode(InstanceMode::Isolated).with_families(families);
    let (backend, master) = harness.start_set_up().await;
    let created = create(
        &backend,
        &master,
        json!({
            "source": "custom", "providerType": "anthropic", "label": " Work ", "apiKey": "sk-ant-secret-1",
            "baseUrl": "https://proxy.example/v1", "models": [configured("claude-work")]
        }),
    )
    .await;
    assert_eq!(
        (created.kind, created.provider_type.as_str(), created.label.as_str()),
        (CredentialKind::ApiKey, "anthropic", "Work")
    );
    assert_eq!(
        created.base_url.as_ref().map(|url| url.as_str()),
        Some("https://proxy.example/v1")
    );
    assert_eq!((created.wire_api, created.vendor_id.clone()), (None, None));
    assert_eq!(created.models.as_ref().unwrap().0[0].id.as_str(), "claude-work");
    let listed = backend.get("/api/providers", Some(&master)).await;
    assert_eq!(listed.json::<Providers>().providers, std::slice::from_ref(&created));
    assert!(!String::from_utf8_lossy(&listed.body).contains("sk-ant-secret-1"));
    // The database holds the configuration sealed.
    let sealed: Vec<u8> = harness
        .control_database()
        .query_row(
            "SELECT config FROM providers WHERE id = ?1",
            [created.id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!sealed.windows(15).any(|window| window == b"sk-ant-secret-1"));

    for (body, refusal) in [
        (
            json!({ "source": "custom", "providerType": "nope", "label": "x", "apiKey": "k" }),
            ErrorCode::UnknownProviderType,
        ),
        (
            json!({ "source": "custom", "providerType": "anthropic", "wireApi": "responses", "label": "x", "apiKey": "k" }),
            ErrorCode::InvalidBody,
        ),
        (
            json!({ "source": "custom", "providerType": "anthropic", "label": "x", "apiKey": "k", "models": [configured("m"), configured("m")] }),
            ErrorCode::InvalidBody,
        ),
        (
            json!({ "source": "custom", "providerType": "anthropic", "label": "x", "apiKey": "two\nlines" }),
            ErrorCode::InvalidBody,
        ),
        (
            json!({ "source": "custom", "providerType": "anthropic", "label": "  ", "apiKey": "k" }),
            ErrorCode::InvalidBody,
        ),
        (
            json!({ "providerType": "anthropic", "label": "x", "apiKey": "k" }),
            ErrorCode::InvalidBody,
        ),
    ] {
        let answer = backend.post("/api/providers", Some(&master), body.clone()).await;
        assert_eq!(answer.refusal(), (StatusCode::BAD_REQUEST, refusal), "{body}");
    }

    // An edit replaces the key and returns to the family's endpoint and the
    // live catalog; the answer never shows the key.
    let path = format!("/api/providers/{}", created.id);
    let edited = backend
        .patch(
            &path,
            &master,
            json!({ "label": "Personal", "apiKey": "sk-ant-secret-2", "baseUrl": null, "models": null }),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK);
    assert!(!String::from_utf8_lossy(&edited.body).contains("sk-ant-secret"));
    let edited = edited.json::<ProviderAnswer>().provider;
    assert_eq!(
        (edited.label.as_str(), edited.base_url.clone(), edited.models.clone()),
        ("Personal", None, None)
    );
    let live = catalog_of(&backend, &master, "").await;
    assert_eq!(
        live.providers[0].models[0].model.id, "claude-opus-4-8",
        "the anthropic directory"
    );

    // API-key entries of one family repeat.
    let body = json!({ "source": "custom", "providerType": "anthropic", "label": "Second", "apiKey": "k" });
    let (second, third) = (
        create(&backend, &master, body.clone()).await,
        create(&backend, &master, body).await,
    );
    assert_ne!(second.id, third.id);
    for extra in [second, third] {
        backend.delete(&format!("/api/providers/{}", extra.id), &master).await;
    }

    // Another user of an isolated instance reaches none of it, and keeps a
    // subscription entry of a family the master holds too.
    harness.add_user("alice@example.test", "alice-pass-1", Role::User);
    let alice = backend.login("alice@example.test", "alice-pass-1").await;
    assert_eq!(
        backend
            .get("/api/providers", Some(&alice))
            .await
            .json::<Providers>()
            .providers,
        []
    );
    for user in [&master, &alice] {
        let imported = backend
            .post(
                "/api/providers/setup-token",
                Some(user),
                json!({ "token": format!("token-of-{}", user.user.email.as_str()), "label": "Claude" }),
            )
            .await;
        assert_eq!(imported.status, StatusCode::CREATED);
        let path = format!("/api/providers/{}", imported.json::<ProviderAnswer>().provider.id);
        assert_eq!(backend.delete(&path, user).await.status, StatusCode::NO_CONTENT);
    }
    assert_eq!(
        backend.delete(&path, &alice).await.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ProviderNotFound)
    );
    assert_eq!(
        backend.get(&format!("{path}/status"), Some(&alice)).await.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ProviderNotFound)
    );

    assert_eq!(backend.delete(&path, &master).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        backend
            .get("/api/providers", Some(&master))
            .await
            .json::<Providers>()
            .providers,
        []
    );
    assert_eq!(catalog_of(&backend, &master, "").await.providers, []);
    assert_eq!(
        backend
            .patch(&path, &master, json!({ "label": "Gone" }))
            .await
            .refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ProviderNotFound)
    );
    backend.close().await;
}

#[tokio::test]
async fn on_a_shared_instance_only_the_master_configures_and_everyone_infers_with_its_entries() {
    let directory = Arc::new(Directory::default());
    let harness = Harness::new().with_families(scripted(&directory));
    let (backend, master) = harness.start_set_up().await;
    harness.add_user("admin@example.test", "admin-pass-1", Role::Admin);
    harness.add_user("bob@example.test", "bob-pass-1", Role::User);
    let admin = backend.login("admin@example.test", "admin-pass-1").await;
    let bob = backend.login("bob@example.test", "bob-pass-1").await;
    let body = json!({ "source": "custom", "providerType": "scripted", "label": "Instance", "apiKey": "k", "models": [configured("m")] });
    for user in [&admin, &bob] {
        let refused = backend.post("/api/providers", Some(user), body.clone()).await;
        assert_eq!(refused.refusal(), (StatusCode::FORBIDDEN, ErrorCode::Forbidden));
    }
    let shared = create(&backend, &master, body).await;
    let listed = backend
        .get("/api/providers", Some(&bob))
        .await
        .json::<Providers>()
        .providers;
    assert_eq!(listed, std::slice::from_ref(&shared));
    let models = catalog_of(&backend, &bob, "").await;
    assert_eq!(models.providers[0].provider_id, shared.id);
    assert_eq!(models.providers[0].availability, Availability::Available {});
    let path = format!("/api/providers/{}", shared.id);
    assert_eq!(
        backend.delete(&path, &bob).await.refusal(),
        (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
    );
    assert_eq!(
        backend
            .post(&format!("{path}/test"), Some(&bob), json!({ "modelId": "m" }))
            .await
            .refusal(),
        (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
    );
    backend.close().await;

    // The entries stay the master's own under the other mode, and nobody
    // else's.
    let backend = harness.start_in_mode(InstanceMode::Isolated).await;
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let bob = backend.login("bob@example.test", "bob-pass-1").await;
    assert_eq!(
        backend
            .get("/api/providers", Some(&master))
            .await
            .json::<Providers>()
            .providers
            .len(),
        1
    );
    assert_eq!(
        backend
            .get("/api/providers", Some(&bob))
            .await
            .json::<Providers>()
            .providers,
        []
    );
    backend.close().await;
}

fn models_dev_document() -> Value {
    json!({
        "deepseek": {
            "id": "deepseek", "name": "DeepSeek", "npm": "@ai-sdk/openai-compatible",
            "api": "https://api.deepseek.com", "doc": "https://api-docs.deepseek.com",
            "models": {
                "deepseek-v4": {
                    "name": "DeepSeek V4", "reasoning": true, "tool_call": true, "attachment": false,
                    "limit": { "context": 128_000, "output": 32_000 },
                    "cost": { "input": 0.3, "output": 1.2, "cache_read": 0.03, "cache_write": 0 }
                },
                "deepseek-v4-flash": { "name": "DeepSeek V4 Flash", "limit": { "context": 128_000 } }
            }
        },
        "minimax": {
            "id": "minimax", "name": "MiniMax", "npm": "@ai-sdk/anthropic", "api": "https://api.minimax.io/anthropic/v1",
            "models": { "minimax-m3": { "name": "MiniMax M3", "limit": { "context": 200_000, "output": 64_000 } } }
        },
        "zai": { "id": "zai", "name": "z.ai", "npm": "@ai-sdk/google", "models": {} },
        "amazon-bedrock": {
            "id": "amazon-bedrock", "name": "Amazon Bedrock", "npm": "@ai-sdk/amazon-bedrock",
            "models": { "anthropic.claude-sonnet": { "name": "Claude Sonnet on Bedrock" } }
        },
        "github-copilot": {
            "id": "github-copilot", "name": "GitHub Copilot", "npm": "@ai-sdk/openai-compatible",
            "api": "https://api.githubcopilot.com", "models": { "gpt-5.5": { "name": "GPT-5.5" } }
        }
    })
}

fn served(document: &Value) -> MockResponse {
    MockResponse::status(200)
        .header("etag", "\"fixture\"")
        .chunk(document.to_string())
}

#[tokio::test]
async fn a_vendor_entry_takes_its_family_wire_and_endpoint_from_models_dev_and_reads_its_live_models() {
    let vendor = MockVendor::start().await;
    let directory = Arc::new(Directory::default());
    let families = FamilyRegistry::builtin().with(
        "openai",
        ScriptedKey {
            directory: directory.clone(),
            wires: &[WireApi::Responses, WireApi::ChatCompletions],
        },
    );
    let harness = Harness::new()
        .with_families(families)
        .with_models_dev(vendor.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;
    vendor.respond(served(&models_dev_document()));
    // Only vendors a registered family speaks to, by name; Bedrock's package
    // has no family, Copilot is excluded by name, and z.ai's family is not
    // registered here.
    let offered = backend
        .get("/api/providers/catalog", Some(&master))
        .await
        .json::<VendorCatalog>();
    let offered: Vec<Value> = offered
        .vendors
        .iter()
        .map(|vendor| serde_json::to_value(vendor).unwrap())
        .collect();
    assert_eq!(
        offered,
        [
            json!({ "id": "deepseek", "name": "DeepSeek", "providerType": "openai", "wireApi": "chat-completions",
                    "baseUrl": "https://api.deepseek.com", "doc": "https://api-docs.deepseek.com" }),
            json!({ "id": "minimax", "name": "MiniMax", "providerType": "anthropic",
                    "baseUrl": "https://api.minimax.io/anthropic/v1", "doc": null }),
        ]
    );
    let unknown = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({ "source": "vendor", "vendorId": "amazon-bedrock", "label": "x", "apiKey": "k" }),
        )
        .await;
    assert_eq!(unknown.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::UnknownVendor));

    let deepseek = create(
        &backend,
        &master,
        json!({ "source": "vendor", "vendorId": "deepseek", "label": "DeepSeek", "apiKey": "sk-1" }),
    )
    .await;
    assert_eq!(
        (
            deepseek.provider_type.as_str(),
            deepseek.wire_api,
            deepseek.vendor_id.as_deref()
        ),
        ("openai", Some(WireApi::ChatCompletions), Some("deepseek"))
    );
    assert_eq!(
        deepseek.base_url.as_ref().map(|url| url.as_str()),
        Some("https://api.deepseek.com/")
    );
    // The entry's catalog is the vendor's live list, read again from
    // models.dev with the copy's validator.
    vendor.respond(MockResponse::status(304));
    let live = catalog_of(&backend, &master, "").await;
    assert_eq!(vendor.requests()[1].header("if-none-match"), Some("\"fixture\""));
    let models: Vec<&str> = live.providers[0]
        .models
        .iter()
        .map(|model| model.model.id.as_str())
        .collect();
    assert_eq!(models, ["deepseek-v4", "deepseek-v4-flash"]);
    let v4 = &live.providers[0].models[0];
    assert_eq!(
        (
            v4.model.context_window,
            v4.model.output_limit,
            v4.model.supports_reasoning
        ),
        (Some(128_000), Some(32_000), Some(true))
    );
    assert_eq!(
        (v4.selection.provider_id.as_str(), v4.selection.model.context_window),
        (deepseek.id.as_str(), 128_000)
    );
    assert!(!live.providers[0].stale);
    assert_eq!(
        directory.reads(),
        0,
        "a vendor entry's catalog is the vendor's, never its family's"
    );

    // A typed list replaces the live one, and removing it returns to it.
    let path = format!("/api/providers/{}", deepseek.id);
    backend
        .patch(&path, &master, json!({ "models": [configured("deepseek-v4")] }))
        .await;
    let typed = catalog_of(&backend, &master, "?refresh=true").await;
    assert_eq!(typed.providers[0].models.len(), 1);
    assert_eq!(vendor.requests().len(), 2, "a configured list is never fetched");
    backend.patch(&path, &master, json!({ "models": null })).await;
    vendor.respond(MockResponse::status(304));
    let again = catalog_of(&backend, &master, "").await;
    assert_eq!(again.providers[0].models.len(), 2);
    backend.close().await;
}

#[tokio::test]
async fn a_directory_catalog_is_cached_refreshed_on_demand_and_kept_after_a_failed_refresh() {
    let directory = Arc::new(Directory::default());
    let harness = Harness::new().with_families(scripted(&directory));
    let (backend, master) = harness.start_set_up().await;
    let entry = create(
        &backend,
        &master,
        json!({ "source": "custom", "providerType": "scripted", "label": "Scripted", "apiKey": "k" }),
    )
    .await;
    let ids = |catalog: &ModelCatalog| -> Vec<String> {
        catalog.providers[0]
            .models
            .iter()
            .map(|model| model.model.id.clone())
            .collect()
    };
    directory.answer(Ok(catalog(&["a", "b"])));
    let first = catalog_of(&backend, &master, "").await;
    assert_eq!(
        (ids(&first), first.providers[0].stale, directory.reads()),
        (vec!["a".to_owned(), "b".to_owned()], false, 1)
    );
    assert_eq!(
        first.providers[0].source_fetched_at,
        "2026-09-24T07:00:00.000Z".parse().unwrap()
    );
    catalog_of(&backend, &master, "").await;
    assert_eq!(directory.reads(), 1, "a fresh record answers without the source");

    for query in ["?refresh=1", "?refresh=TRUE", "?refresh="] {
        let refused = backend.get(&format!("/api/models{query}"), Some(&master)).await;
        assert_eq!(
            refused.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery),
            "{query}"
        );
    }
    directory.answer(Ok(catalog(&["a", "b", "c"])));
    assert_eq!(ids(&catalog_of(&backend, &master, "?refresh=true").await).len(), 3);
    directory.answer(Err(CatalogError::Unavailable("the directory is down".into())));
    let failed = catalog_of(&backend, &master, "?refresh=true").await;
    assert_eq!((ids(&failed).len(), failed.providers[0].stale), (3, true));
    assert_eq!(failed.providers[0].warnings, ["the directory is down"]);

    // A restart serves the stored record before it asks the source.
    backend.close().await;
    let backend = harness.start().await;
    let restored = catalog_of(&backend, &master, "").await;
    assert_eq!((ids(&restored).len(), directory.reads()), (3, 3));

    // Expired: the record at once, marked stale, while one refresh runs.
    harness.clock.advance(SignedDuration::from_mins(16));
    directory.answer(Ok(catalog(&["d"])));
    let expired = catalog_of(&backend, &master, "").await;
    assert_eq!((ids(&expired).len(), expired.providers[0].stale), (3, true));
    let mut refreshed = expired;
    for _ in 0..100 {
        if ids(&refreshed) == ["d"] {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        refreshed = catalog_of(&backend, &master, "").await;
    }
    assert_eq!(
        (ids(&refreshed), refreshed.providers[0].stale, directory.reads()),
        (vec!["d".to_owned()], false, 4)
    );

    // A configured list is read from the entry and never fetched.
    let path = format!("/api/providers/{}", entry.id);
    backend
        .patch(&path, &master, json!({ "models": [configured("typed")] }))
        .await;
    let typed = catalog_of(&backend, &master, "?refresh=true").await;
    assert_eq!(
        (ids(&typed), typed.providers[0].stale, directory.reads()),
        (vec!["typed".to_owned()], false, 4)
    );
    assert_eq!(typed.providers[0].source_fetched_at, demi_core::Timestamp::UNIX_EPOCH);
    let model = &typed.providers[0].models[0];
    assert_eq!(
        (
            model.selection.model.output_limit,
            model.model.service_tiers[0].id.as_str(),
            model.model.accepted_extensions.clone()
        ),
        (Some(4_000), "priority", Some(vec![demi_core::FileExtension::Pdf]))
    );
    backend.close().await;
}

/// A Messages API stream that answers `ok`.
fn answered() -> MockResponse {
    let frames = [
        json!({ "type": "message_start", "message": {
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-4-8", "content": [],
            "usage": { "input_tokens": 3, "output_tokens": 1 } } }),
        json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
        json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "ok" } }),
        json!({ "type": "content_block_stop", "index": 0 }),
        json!({ "type": "message_stop" }),
    ];
    let text: String = frames
        .iter()
        .map(|frame| format!("event: {}\ndata: {frame}\n\n", frame["type"].as_str().unwrap()))
        .collect();
    MockResponse::event_stream(text)
}

#[tokio::test]
async fn the_provider_test_sends_one_real_request_through_the_entrys_family() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let entry = create(
        &backend,
        &master,
        json!({ "source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test", "baseUrl": vendor.url("/v1") }),
    )
    .await;
    let path = format!("/api/providers/{}/test", entry.id);
    vendor.respond(answered());
    let passed = backend
        .post(&path, Some(&master), json!({ "modelId": "claude-opus-4-8" }))
        .await;
    assert_eq!(
        passed.json::<TestResult>(),
        TestResult::Passed {
            model: "Claude Opus 4.8".into()
        }
    );
    let sent = &vendor.requests()[0];
    assert_eq!(
        (sent.uri.path(), sent.header("x-api-key")),
        ("/v1/messages", Some("sk-ant-test"))
    );
    assert_eq!(sent.json()["system"], "Reply with the word ok.");

    vendor.respond(
        MockResponse::status(401)
            .chunk(r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#),
    );
    let refused = backend
        .post(&path, Some(&master), json!({ "modelId": "claude-opus-4-8" }))
        .await
        .json::<TestResult>();
    let TestResult::Failed { message, model } = refused else {
        panic!("the vendor refused the key: {refused:?}");
    };
    assert!(
        message.contains("HTTP 401") && message.contains("invalid x-api-key"),
        "{message}"
    );
    assert_eq!(model.as_deref(), Some("Claude Opus 4.8"));

    let unknown = backend
        .post(&path, Some(&master), json!({ "modelId": "no-such-model" }))
        .await
        .json::<TestResult>();
    assert_eq!(
        unknown,
        TestResult::Failed {
            message: "This provider lists no model no-such-model".into(),
            model: None
        }
    );
    let account = backend
        .post(
            &path,
            Some(&master),
            json!({ "modelId": "claude-opus-4-8", "credentialId": "cred-1" }),
        )
        .await;
    assert_eq!(account.refusal(), (StatusCode::NOT_FOUND, ErrorCode::AccountNotFound));
    assert_eq!(vendor.requests().len(), 2);
    backend.close().await;
}
