//! Provider entries (`web-api-settings.md` § Model configuration and provider
//! inspection, `providers.md` § Credential vault, `models.md`): API-key
//! entries sealed at rest and answered without their key, the master's
//! entries serving every user's conversations on a shared instance and
//! nobody else's on an isolated one, the vendor list from a scripted
//! models.dev, each catalog source with its cache, and the provider test
//! through a real family against a scripted vendor.

use std::sync::Arc;
use std::time::Duration;

use demi_backend_providers::llm::families::{
    FamilyArgs, FamilyError, FamilyRegistry, ProviderFamily,
};
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_provider_common::CatalogError;
use demi_provider_common::testing::{MockResponse, MockVendor, RecordedRequest, sse_body};
use demi_shared_types::WireApi;
use demi_web_api_protocol::auth::Role;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::ProviderId;
use demi_web_api_protocol::providers::{
    Availability, CatalogProvider, CredentialKind, ModelCatalog, ProviderAnswer, ProviderDto, Providers, TestResult,
    VendorCatalog,
};
use demi_web_api_protocol::settings::InstanceMode;
use jiff::SignedDuration;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::{self, FIRST, Socket};
use crate::families::{Directory, LoginScript, QuotaScript, ScriptedKey, ScriptedSubscription, catalog};
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
    let answer = backend
        .get(&format!("/api/models{query}"), Some(session))
        .await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    answer.json()
}

fn scripted(directory: &Arc<Directory>) -> FamilyRegistry {
    demi_backend::families::builtin().with(
        "scripted",
        ScriptedKey {
            directory: directory.clone(),
            wires: &[],
            process_host: false,
        },
    )
}

#[tokio::test]
async fn an_api_key_entry_is_sealed_at_rest_and_answered_without_its_key() {
    let directory = Arc::new(Directory::default());
    let scripts = crate::accounts::scripts(None);
    let families = scripts.families.clone().with(
        "scripted",
        ScriptedKey {
            directory: directory.clone(),
            wires: &[],
            process_host: false,
        },
    );
    let harness = Harness::new()
        .with_mode(InstanceMode::Isolated)
        .with_families(families);
    let (backend, master) = harness.start_set_up().await;
    let created = create(
        &backend,
        &master,
        json!({
            "source": "compatible", "providerType": "anthropic", "label": " Work ", "apiKey": "sk-ant-secret-1",
            "baseUrl": "https://proxy.example/v1", "models": [configured("claude-work")]
        }),
    )
    .await;
    assert_eq!(
        (
            created.kind,
            created.provider_type.as_str(),
            created.label.as_str()
        ),
        (CredentialKind::ApiKey, "anthropic", "Work")
    );
    assert_eq!(
        created.endpoint.as_ref().map(|url| url.as_str()),
        Some("https://proxy.example/v1")
    );
    assert_eq!((created.wire_api, created.vendor_id.clone()), (None, None));
    assert_eq!(
        created.models.as_ref().unwrap().0[0].id.as_str(),
        "claude-work"
    );
    let listed = backend.get("/api/providers", Some(&master)).await;
    assert_eq!(
        listed.json::<Providers>().providers,
        std::slice::from_ref(&created)
    );
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
    assert!(
        !sealed
            .windows(15)
            .any(|window| window == b"sk-ant-secret-1")
    );

    for (body, refusal) in [
        (
            json!({ "source": "family", "providerType": "nope", "label": "x", "apiKey": "k" }),
            ErrorCode::UnknownProviderType,
        ),
        (
            json!({ "source": "family", "providerType": "anthropic", "wireApi": "responses", "label": "x", "apiKey": "k" }),
            ErrorCode::InvalidBody,
        ),
        (
            json!({ "source": "family", "providerType": "anthropic", "label": "x", "apiKey": "k", "models": [configured("m"), configured("m")] }),
            ErrorCode::DuplicateModel,
        ),
        (
            json!({ "source": "family", "providerType": "anthropic", "label": "x", "apiKey": "two\nlines" }),
            ErrorCode::InvalidBody,
        ),
        (
            json!({ "source": "family", "providerType": "anthropic", "label": "  ", "apiKey": "k" }),
            ErrorCode::InvalidBody,
        ),
        (
            json!({ "providerType": "anthropic", "label": "x", "apiKey": "k" }),
            ErrorCode::InvalidBody,
        ),
    ] {
        let answer = backend
            .post("/api/providers", Some(&master), body.clone())
            .await;
        assert_eq!(
            answer.refusal(),
            (StatusCode::BAD_REQUEST, refusal),
            "{body}"
        );
    }

    // An edit replaces the key and the endpoint, and clearing the list leaves
    // a compatible entry without models; the answer never shows the key.
    let path = format!("/api/providers/{}", created.id);
    let edited = backend
        .patch(
            &path,
            &master,
            json!({ "label": "Personal", "apiKey": "sk-ant-secret-2", "baseUrl": "https://other.example/v1", "models": null }),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK);
    assert!(!String::from_utf8_lossy(&edited.body).contains("sk-ant-secret"));
    let edited = edited.json::<ProviderAnswer>().provider;
    assert_eq!(
        (
            edited.label.as_str(),
            edited.endpoint.as_ref().map(|url| url.as_str().to_owned()),
            edited.models.clone()
        ),
        ("Personal", Some("https://other.example/v1".to_owned()), None)
    );
    assert_eq!(catalog_of(&backend, &master, "").await.providers[0].models, []);

    // API-key entries of one family repeat.
    let body = json!({ "source": "family", "providerType": "anthropic", "label": "Second", "apiKey": "k" });
    let (second, third) = (
        create(&backend, &master, body.clone()).await,
        create(&backend, &master, body).await,
    );
    assert_ne!(second.id, third.id);
    for extra in [second, third] {
        backend
            .delete(&format!("/api/providers/{}", extra.id), &master)
            .await;
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
        let entry = crate::accounts::device_entry(&backend, user, &scripts).await;
        let path = format!("/api/providers/{}", entry.id);
        assert_eq!(
            backend.delete(&path, user).await.status,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        backend.delete(&path, &alice).await.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ProviderNotFound)
    );
    assert_eq!(
        backend
            .get(&format!("{path}/status"), Some(&alice))
            .await
            .refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ProviderNotFound)
    );

    assert_eq!(
        backend.delete(&path, &master).await.status,
        StatusCode::NO_CONTENT
    );
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
    let body = json!({ "source": "family", "providerType": "scripted", "label": "Instance", "apiKey": "k", "models": [configured("m")] });
    for user in [&admin, &bob] {
        let refused = backend
            .post("/api/providers", Some(user), body.clone())
            .await;
        assert_eq!(
            refused.refusal(),
            (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
        );
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
            .post(
                &format!("{path}/test"),
                Some(&bob),
                json!({ "modelId": "m" })
            )
            .await
            .refusal(),
        (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
    );
    // A user's conversation infers with the instance's entry.
    conversations::create(&backend, &bob, FIRST).await;
    conversations::choose(&backend, &bob, FIRST, shared.id.as_str(), "m").await;
    let mut socket = Socket::connect(&backend, &bob, FIRST).await;
    socket.open().await;
    let turn = socket.chat("m1", "hello").await;
    assert!(
        serde_json::to_string(&turn).unwrap().contains("\"ok\""),
        "{turn:?}"
    );
    drop(socket);
    backend.close().await;

    // The entries stay the master's own under the other mode, and nobody
    // else's: another user's conversation cannot open with them, nor select
    // them.
    let backend = harness.start_in_mode(InstanceMode::Isolated).await;
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let bob = backend.login("bob@example.test", "bob-pass-1").await;
    let mut socket = Socket::connect(&backend, &bob, FIRST).await;
    socket.send(&ClientFrame::Open { from: None, edge: None }).await;
    let ServerFrame::Error { code, .. } = socket.frame().await else {
        panic!("another user's entry is not the user's to open with");
    };
    assert_eq!(code.as_deref(), Some("provider_not_found"));
    let chosen = backend
        .patch(
            &format!("/api/conversations/{FIRST}"),
            &bob,
            json!({ "model": { "providerId": shared.id, "modelId": "m" } }),
        )
        .await;
    assert_eq!(
        chosen.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ProviderNotFound)
    );
    drop(socket);
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
                    "modalities": { "input": ["text"], "output": ["text"] },
                    "limit": { "context": 128_000, "output": 32_000 },
                    "cost": { "input": 0.3, "output": 1.2, "cache_read": 0.03, "cache_write": 0 }
                },
                "deepseek-v4-flash": {
                    "name": "DeepSeek V4 Flash", "tool_call": true,
                    "modalities": { "input": ["text"], "output": ["text"] }, "limit": { "context": 128_000 }
                }
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

pub(crate) fn served(document: &Value) -> MockResponse {
    MockResponse::status(200)
        .header("etag", "\"fixture\"")
        .chunk(document.to_string())
}

#[tokio::test]
async fn a_vendor_entry_takes_its_family_wire_and_endpoint_from_models_dev_and_reads_its_live_models()
 {
    let vendor = MockVendor::start().await;
    let directory = Arc::new(Directory::default());
    let families = demi_backend::families::builtin().with(
        "openai",
        ScriptedKey {
            directory: directory.clone(),
            wires: &[WireApi::Responses, WireApi::ChatCompletions],
            process_host: false,
        },
    );
    let harness = Harness::new()
        .with_families(families)
        .with_models_dev(vendor.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;
    vendor.respond(served(&models_dev_document()));
    // Only vendors a family speaks to, by name: Bedrock's package has no
    // family, and Copilot is excluded by name. A vendor that names no
    // endpoint and has no official one starts without.
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
            json!({ "id": "zai", "name": "z.ai", "providerType": "google", "baseUrl": null, "doc": null }),
        ]
    );
    let unknown = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({ "source": "vendor", "vendorId": "amazon-bedrock", "label": "x", "apiKey": "k" }),
        )
        .await;
    assert_eq!(
        unknown.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::UnknownVendor)
    );

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
        deepseek.endpoint.as_ref().map(|url| url.as_str()),
        Some("https://api.deepseek.com/")
    );
    // The entry's catalog is the vendor's live list, read again from
    // models.dev with the copy's validator.
    vendor.respond(MockResponse::status(304));
    let live = catalog_of(&backend, &master, "").await;
    assert_eq!(
        vendor.requests()[1].header("if-none-match"),
        Some("\"fixture\"")
    );
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
        (
            v4.selection.provider_id.as_str(),
            v4.selection.model.context_window
        ),
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
        .patch(
            &path,
            &master,
            json!({ "models": [configured("deepseek-v4")] }),
        )
        .await;
    let typed = catalog_of(&backend, &master, "?refresh=true").await;
    assert_eq!(typed.providers[0].models.len(), 1);
    assert_eq!(
        vendor.requests().len(),
        2,
        "a configured list is never fetched"
    );
    backend
        .patch(&path, &master, json!({ "models": null }))
        .await;
    vendor.respond(MockResponse::status(304));
    let again = catalog_of(&backend, &master, "").await;
    assert_eq!(again.providers[0].models.len(), 2);
    backend.close().await;
}

/// A subscription entry's catalog is its provider's directory, cached and
/// refreshed; an API-key entry's configured list is never fetched.
#[tokio::test]
async fn a_directory_catalog_is_cached_refreshed_on_demand_and_kept_after_a_failed_refresh() {
    let directory = Arc::new(Directory::default());
    let login = Arc::new(LoginScript::default());
    let quota = Arc::new(QuotaScript::free(Vec::new()));
    let scripts = crate::accounts::Scripts {
        login: login.clone(),
        quota: quota.clone(),
        families: scripted(&directory).with(
            "device",
            ScriptedSubscription {
                login,
                quota,
                directory: directory.clone(),
                runs: None,
                process_host: false,
            },
        ),
    };
    let harness = Harness::new().with_families(scripts.families.clone());
    let (backend, master) = harness.start_set_up().await;
    directory.answer(Ok(catalog(&["a", "b"])));
    scripts.login.signs_in("first");
    let entry = crate::accounts::device_entry(&backend, &master, &scripts).await;
    let reads = directory.reads();
    let of = |catalog: &ModelCatalog, id: &ProviderId| -> CatalogProvider {
        catalog
            .providers
            .iter()
            .find(|provider| provider.provider_id == *id)
            .expect("the entry has a catalog")
            .clone()
    };
    let ids = |catalog: &ModelCatalog| -> Vec<String> {
        of(catalog, &entry.id)
            .models
            .iter()
            .map(|model| model.model.id.clone())
            .collect()
    };
    let first = catalog_of(&backend, &master, "").await;
    assert_eq!(
        (ids(&first), of(&first, &entry.id).stale, directory.reads()),
        (vec!["a".to_owned(), "b".to_owned()], false, reads.max(1))
    );
    assert_eq!(
        of(&first, &entry.id).source_fetched_at,
        "2026-09-24T07:00:00.000Z".parse().unwrap()
    );
    let reads = directory.reads();
    catalog_of(&backend, &master, "").await;
    assert_eq!(
        directory.reads(),
        reads,
        "a fresh record answers without the source"
    );

    for query in ["?refresh=1", "?refresh=TRUE", "?refresh="] {
        let refused = backend
            .get(&format!("/api/models{query}"), Some(&master))
            .await;
        assert_eq!(
            refused.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery),
            "{query}"
        );
    }
    directory.answer(Ok(catalog(&["a", "b", "c"])));
    assert_eq!(
        ids(&catalog_of(&backend, &master, "?refresh=true").await).len(),
        3
    );
    directory.answer(Err(CatalogError::Unavailable(
        "the directory is down".into(),
    )));
    let failed = catalog_of(&backend, &master, "?refresh=true").await;
    assert_eq!((ids(&failed).len(), of(&failed, &entry.id).stale), (3, true));
    assert_eq!(of(&failed, &entry.id).warnings, ["the directory is down"]);

    // A restart serves the stored record before it asks the source.
    let reads = directory.reads();
    backend.close().await;
    let backend = harness.start().await;
    let restored = catalog_of(&backend, &master, "").await;
    assert_eq!((ids(&restored).len(), directory.reads()), (3, reads));

    // Expired: the record at once, marked stale, while one refresh runs.
    harness.clock.advance(SignedDuration::from_mins(16));
    directory.answer(Ok(catalog(&["d"])));
    let expired = catalog_of(&backend, &master, "").await;
    assert_eq!((ids(&expired).len(), of(&expired, &entry.id).stale), (3, true));
    let mut refreshed = expired;
    for _ in 0..100 {
        if ids(&refreshed) == ["d"] {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        refreshed = catalog_of(&backend, &master, "").await;
    }
    assert_eq!(
        (
            ids(&refreshed),
            of(&refreshed, &entry.id).stale,
            directory.reads()
        ),
        (vec!["d".to_owned()], false, reads + 1)
    );

    // A configured list is read from the entry and never fetched.
    let reads = directory.reads();
    let typed = create(
        &backend,
        &master,
        json!({ "source": "family", "providerType": "scripted", "apiKey": "k", "models": [configured("typed")] }),
    )
    .await;
    let answer = catalog_of(&backend, &master, "?refresh=true").await;
    let listed = of(&answer, &typed.id);
    assert_eq!(
        (
            listed.models.iter().map(|model| model.model.id.as_str()).collect::<Vec<_>>(),
            listed.stale,
            listed.source_fetched_at,
        ),
        (vec!["typed"], false, demi_shared_types::Timestamp::UNIX_EPOCH)
    );
    assert_eq!(directory.reads(), reads + 1, "only the subscription's directory is read");
    let model = &listed.models[0];
    assert_eq!(
        (
            model.selection.model.output_limit,
            model.model.service_tiers[0].id.as_str(),
            model.model.accepted_extensions.clone()
        ),
        (
            Some(4_000),
            "priority",
            Some(vec![demi_shared_types::FileExtension::Pdf])
        )
    );
    backend.close().await;
}

/// A Messages API stream that answers `ok`.
/// Tests a new entry of `family`, on `wire` when it names one, whose vendor
/// streams `stream`, and answers the request the vendor received.
async fn tested(
    backend: &TestBackend,
    master: &Session,
    vendor: &MockVendor,
    family: &str,
    wire: Option<&str>,
    stream: String,
) -> RecordedRequest {
    let mut body = json!({
        "source": "compatible", "providerType": family, "label": family, "apiKey": "sk-2",
        "baseUrl": vendor.url("/v1"), "models": [configured("m")]
    });
    if let Some(wire) = wire {
        body["wireApi"] = json!(wire);
    }
    let entry = create(backend, master, body).await;
    vendor.respond(MockResponse::event_stream(stream));
    let path = format!("/api/providers/{}/test", entry.id);
    let result = backend
        .post(&path, Some(master), json!({ "modelId": "m" }))
        .await
        .json::<TestResult>();
    let passed = TestResult::Passed {
        model: "m display".into(),
    };
    assert_eq!(result, passed, "{family} {wire:?}");
    vendor.requests().pop().expect("the test sent a request")
}

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
        .map(|frame| {
            format!(
                "event: {}\ndata: {frame}\n\n",
                frame["type"].as_str().unwrap()
            )
        })
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
        json!({
            "source": "compatible", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test",
            "baseUrl": vendor.url("/v1"),
            "models": [{ "id": "claude-opus-4-8", "displayName": "Claude Opus 4.8", "contextWindow": 1_000_000,
                "outputLimit": null, "thinkingEfforts": [], "acceptedExtensions": null, "fastTier": null }]
        }),
    )
    .await;
    let path = format!("/api/providers/{}/test", entry.id);
    vendor.respond(answered());
    let passed = backend
        .post(
            &path,
            Some(&master),
            json!({ "modelId": "claude-opus-4-8" }),
        )
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
    // No later request extends a connection test, so nothing in it is
    // marked for the vendor's cache.
    let body = sent.json();
    assert_eq!(
        body["system"],
        json!([{ "type": "text", "text": "Reply with the word ok." }])
    );
    assert!(!body.to_string().contains("cache_control"), "{body}");

    vendor.respond(MockResponse::status(401).chunk(
        r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
    ));
    let refused = backend
        .post(
            &path,
            Some(&master),
            json!({ "modelId": "claude-opus-4-8" }),
        )
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

    // An answer whose stream fails at once, which the provider reports to
    // the usage ledger as a call before its failure, fails the test.
    vendor.respond(MockResponse::event_stream(
        "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n",
    ));
    let failed = backend
        .post(&path, Some(&master), json!({ "modelId": "claude-opus-4-8" }))
        .await
        .json::<TestResult>();
    let TestResult::Failed { message, .. } = failed else {
        panic!("the stream failed: {failed:?}");
    };
    assert!(message.contains("Overloaded"), "{message}");

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
    assert_eq!(
        account.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::AccountNotFound)
    );
    assert_eq!(vendor.requests().len(), 3);

    // The other API-key families build from their entries alike: an
    // `openai` entry on the wire it names or on Responses, and a `google`
    // entry.
    let compatible = format!(
        "{}data: [DONE]\n\n",
        sse_body(&[json!({ "choices": [{ "delta": { "content": "ok" }, "finish_reason": "stop" }] })])
    );
    let completed = sse_body(&[json!({
        "type": "response.completed", "response": { "usage": { "input_tokens": 1, "output_tokens": 1 } }
    })]);
    let chat = tested(
        &backend,
        &master,
        &vendor,
        "openai",
        Some("chat-completions"),
        compatible,
    )
    .await;
    assert_eq!(
        (chat.uri.path(), chat.header("authorization")),
        ("/v1/chat/completions", Some("Bearer sk-2"))
    );
    let responses = tested(&backend, &master, &vendor, "openai", None, completed).await;
    assert_eq!(responses.uri.path(), "/v1/responses");
    let finished = sse_body(&[json!({
        "candidates": [{ "content": { "parts": [{ "text": "ok" }] }, "finishReason": "STOP" }]
    })]);
    let google = tested(&backend, &master, &vendor, "google", None, finished).await;
    assert_eq!(
        (google.uri.path(), google.header("x-goog-api-key")),
        ("/v1/models/m:streamGenerateContent", Some("sk-2"))
    );
    backend.close().await;
}

/// An API-key family whose provider cannot be built, as an entry whose
/// stored settings its family no longer takes.
struct Refusing;

impl ProviderFamily for Refusing {
    fn credential(&self) -> CredentialKind {
        CredentialKind::ApiKey
    }

    fn provider(
        &self,
        _: FamilyArgs,
    ) -> Result<Arc<dyn demi_provider_common::Provider>, FamilyError> {
        Err(FamilyError::Invalid("the scripted family refuses".into()))
    }
}

#[tokio::test]
async fn an_entry_whose_provider_cannot_be_built_answers_its_status_with_the_reason() {
    let families = demi_backend::families::builtin().with("refusing", Refusing);
    let harness = Harness::new().with_families(families);
    let (backend, master) = harness.start_set_up().await;
    let created = create(
        &backend,
        &master,
        json!({
            "source": "family", "providerType": "refusing", "label": "Broken", "apiKey": "key",
            "models": [configured("m")]
        }),
    )
    .await;
    let status = backend
        .get(
            &format!("/api/providers/{}/status", created.id.as_str()),
            Some(&master),
        )
        .await;
    assert_eq!(status.status, StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = status.json();
    assert_eq!(body["code"], "provider_status_failed");
    assert_eq!(body["message"], "the scripted family refuses");
    backend.close().await;
}

/// Adaptive thinking is Anthropic's own: a vendor from models.dev on
/// `@ai-sdk/anthropic` receives an effort as its token budget, and an entry
/// that names no vendor, Anthropic's, the effort itself (`models.md` §
/// Thinking and service tiers).
#[tokio::test]
async fn an_anthropic_compatible_vendor_receives_an_effort_as_a_token_budget() {
    let vendor = MockVendor::start().await;
    let catalog = json!({
        "moonshot": {
            "id": "moonshot", "name": "Moonshot", "npm": "@ai-sdk/anthropic",
            "api": vendor.url("/v1"), "models": {}
        }
    });
    vendor.respond_at("/api.json", served(&catalog));
    let harness = Harness::new().with_models_dev(vendor.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;
    let leveled = json!({
        "id": "m", "displayName": "M", "contextWindow": 100000, "outputLimit": 64000,
        "thinkingEfforts": ["low", "high"], "acceptedExtensions": null, "fastTier": null
    });
    let compatible = create(
        &backend,
        &master,
        json!({
            "source": "vendor", "vendorId": "moonshot", "label": "Moonshot", "apiKey": "key",
            "models": [leveled.clone()]
        }),
    )
    .await;
    let anthropic = create(
        &backend,
        &master,
        json!({
            "source": "compatible", "providerType": "anthropic", "label": "Anthropic", "apiKey": "key",
            "baseUrl": vendor.url("/v1"), "models": [leveled]
        }),
    )
    .await;
    conversations::create(&backend, &master, FIRST).await;
    let path = format!("/api/conversations/{FIRST}");
    let mut sent = Vec::new();
    for provider in [&compatible, &anthropic] {
        let body = json!({
            "model": { "providerId": provider.id.as_str(), "modelId": "m" },
            "thinkingEffort": "high"
        });
        assert_eq!(
            backend.patch(&path, &master, body).await.status,
            StatusCode::OK
        );
        let mut socket = Socket::connect(&backend, &master, FIRST).await;
        socket.open().await;
        vendor.respond(conversations::answer(&["Done."], 1, 1));
        socket
            .chat(&format!("m-{}", sent.len()), "Think hard.")
            .await;
        let request = vendor
            .requests()
            .into_iter()
            .rfind(|request| request.uri.path() == "/v1/messages")
            .expect("the turn reached the vendor")
            .json();
        sent.push(request);
    }
    assert_eq!(
        sent[0]["thinking"],
        json!({ "type": "enabled", "budget_tokens": 32_768 })
    );
    assert_eq!(sent[0].get("output_config"), None);
    assert_eq!(sent[1]["thinking"]["type"], "adaptive");
    assert_eq!(sent[1]["output_config"], json!({ "effort": "high" }));
    backend.close().await;
}

/// The models.dev answer that confirms the kept copy.
fn unchanged() -> MockResponse {
    MockResponse::status(304)
}

/// The availability `GET /api/models` gives the entry `id`.
async fn availability_of(backend: &TestBackend, session: &Session, id: &ProviderId) -> Value {
    let catalog = catalog_of(backend, session, "").await;
    let entry = catalog
        .providers
        .iter()
        .find(|provider| &provider.provider_id == id)
        .expect("the entry is in the catalog");
    serde_json::to_value(&entry.availability).unwrap()
}

fn setup(missing: &[&str]) -> Value {
    json!({
        "type": "unavailable", "reason": "setup",
        "message": "Finish setting up this provider in Settings", "missing": missing
    })
}

/// Google as models.dev lists it: two models an agent can work with, one of
/// them stating an output limit above its window; a text model that calls
/// tools but states no window, which compaction needs; and a video and a
/// speech model whose output limits exceed their windows, as Veo's and the
/// TTS models' do.
fn google_with_media_models() -> Value {
    let agentic = |name: &str, context: u32, output: u32| {
        json!({
            "name": name, "tool_call": true,
            "modalities": { "input": ["text", "image"], "output": ["text"] },
            "limit": { "context": context, "output": output }
        })
    };
    json!({
        "google": {
            "id": "google", "name": "Google", "npm": "@ai-sdk/google",
            "models": {
                "gemini-3.8-flash": agentic("Gemini 3.8 Flash", 1_048_576, 65_536),
                "gemini-3.8-nano": agentic("Gemini 3.8 Nano", 32_768, 65_536),
                "gemini-3.8-flash-live": {
                    "name": "Gemini 3.8 Flash Live", "tool_call": true,
                    "modalities": { "input": ["text"], "output": ["text"] }
                },
                "veo-3.1-lite-generate-preview": {
                    "name": "Veo 3.1 Lite", "modalities": { "input": ["text"], "output": ["video"] },
                    "limit": { "context": 480, "output": 8_192 }
                },
                "gemini-3.8-flash-preview-tts": {
                    "name": "Gemini 3.8 Flash TTS", "tool_call": false,
                    "modalities": { "input": ["text"], "output": ["audio"] },
                    "limit": { "context": 8_192, "output": 16_384 }
                }
            }
        }
    })
}

// About 0.1 s: a backend and a scripted models.dev, no inference.
#[tokio::test]
async fn a_vendor_entry_offers_the_models_an_agent_can_work_with_and_the_page_adds_one_to_them() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_models_dev(vendor.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;
    vendor.respond(served(&google_with_media_models()));
    for _ in 0..4 {
        vendor.respond(unchanged());
    }
    let google = create(
        &backend,
        &master,
        json!({ "source": "vendor", "vendorId": "google", "apiKey": "sk-google" }),
    )
    .await;

    // The windowless, video and speech models stay out, and an output limit
    // above the window states none.
    let catalog = catalog_of(&backend, &master, "").await;
    let offered: Vec<(&str, Option<u32>, Option<u32>)> = catalog.providers[0]
        .models
        .iter()
        .map(|model| {
            (
                model.model.id.as_str(),
                model.model.context_window,
                model.model.output_limit,
            )
        })
        .collect();
    assert_eq!(
        offered,
        [
            ("gemini-3.8-flash", Some(1_048_576), Some(65_536)),
            ("gemini-3.8-nano", Some(32_768), None)
        ]
    );

    // The page adds a model as it does to an entry that follows its
    // vendor: the vendor's models as the catalog states them, and the new
    // one, as one complete list.
    let mut models: Vec<Value> = catalog.providers[0]
        .models
        .iter()
        .map(|model| {
            let model = &model.model;
            json!({
                "id": model.id, "displayName": model.display_name,
                "contextWindow": model.context_window, "outputLimit": model.output_limit,
                "thinkingEfforts": model.supported_thinking_efforts.clone().unwrap_or_default(),
                "acceptedExtensions": model.accepted_extensions, "fastTier": null
            })
        })
        .collect();
    models.push(configured("gemini-custom"));
    let added = backend
        .patch(
            &format!("/api/providers/{}", google.id),
            &master,
            json!({ "models": models }),
        )
        .await;
    assert_eq!(
        added.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&added.body)
    );
    let listed: Vec<String> = added
        .json::<ProviderAnswer>()
        .provider
        .models
        .expect("the entry keeps its own list")
        .0
        .iter()
        .map(|model| model.id.as_str().to_owned())
        .collect();
    assert_eq!(listed, ["gemini-3.8-flash", "gemini-3.8-nano", "gemini-custom"]);
    backend.close().await;
}

#[tokio::test]
async fn an_entry_added_with_only_its_kind_is_incomplete_until_what_it_lacks_is_saved() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_models_dev(vendor.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;
    vendor.respond(served(&json!({
        "google": {
            "id": "google", "name": "Google", "npm": "@ai-sdk/google",
            "models": { "gemini-3.8-flash": {
                "name": "Gemini 3.8 Flash", "tool_call": true,
                "modalities": { "input": ["text"], "output": ["text"] }, "limit": { "context": 1_048_576 }
            } }
        }
    })));
    for _ in 0..4 {
        vendor.respond(unchanged());
    }

    // A vendor alone: named after the vendor, its models listed, and the
    // key the one part it lacks.
    let google = create(
        &backend,
        &master,
        json!({ "source": "vendor", "vendorId": "google" }),
    )
    .await;
    assert_eq!((google.label.as_str(), google.has_api_key), ("Google", false));
    assert_eq!(
        availability_of(&backend, &master, &google.id).await,
        setup(&["apiKey"])
    );
    let catalog = catalog_of(&backend, &master, "").await;
    assert_eq!(catalog.providers[0].models.len(), 1, "{catalog:?}");
    let status = backend
        .get(&format!("/api/providers/{}/status", google.id), Some(&master))
        .await;
    assert_eq!(status.status, StatusCode::OK);

    // The family's own vendor's entry with an empty list lacks both, in
    // order; it uses its vendor's endpoint, which is never missing.
    let custom = create(
        &backend,
        &master,
        json!({ "source": "family", "providerType": "openai", "models": [] }),
    )
    .await;
    assert_eq!(custom.label, "OpenAI");
    assert_eq!(
        availability_of(&backend, &master, &custom.id).await,
        setup(&["apiKey", "models"])
    );

    // Each part saved takes its place off the list, and the last one makes
    // the entry available.
    let patched = backend
        .patch(
            &format!("/api/providers/{}", google.id),
            &master,
            json!({ "apiKey": "sk-google" }),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK);
    assert!(patched.json::<ProviderAnswer>().provider.has_api_key);
    assert_eq!(
        availability_of(&backend, &master, &google.id).await,
        json!({ "type": "available" })
    );
    let path = format!("/api/providers/{}", custom.id);
    let patched = backend
        .patch(
            &path,
            &master,
            json!({ "apiKey": "sk-custom" }),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK);
    assert_eq!(
        availability_of(&backend, &master, &custom.id).await,
        setup(&["models"])
    );
    let patched = backend
        .patch(&path, &master, json!({ "models": [configured("m")] }))
        .await;
    assert_eq!(patched.status, StatusCode::OK);
    assert_eq!(
        availability_of(&backend, &master, &custom.id).await,
        json!({ "type": "available" })
    );
    backend.close().await;
}

/// A compatible entry is created with nothing but its kind and edited
/// after: its protocol changes any time and keeps its endpoint, key and
/// models, its key is optional and sent only when set, and its endpoint is
/// what it cannot work without. One backend and a scripted vendor, three
/// test requests; well under a second.
#[tokio::test]
async fn a_compatible_entry_starts_empty_and_takes_its_protocol_endpoint_and_key_at_any_time() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    // Created at once as Anthropic Messages with no endpoint, key or
    // models; the second takes the first free name.
    let compatible = create(&backend, &master, json!({ "source": "compatible" })).await;
    assert_eq!(
        (
            compatible.label.as_str(),
            compatible.provider_type.as_str(),
            compatible.wire_api,
            compatible.compatible,
            compatible.endpoint.clone(),
            compatible.has_api_key,
            compatible.models.clone(),
        ),
        ("Compatible API", "anthropic", None, true, None, false, None)
    );
    let second = create(&backend, &master, json!({ "source": "compatible" })).await;
    assert_eq!(second.label, "Compatible API (2)");
    // Its endpoint and a model are what it lacks; a key it may never need.
    assert_eq!(
        availability_of(&backend, &master, &compatible.id).await,
        setup(&["endpoint", "models"])
    );

    // An endpoint and a model make it available, without a key, and its
    // requests then carry none.
    let path = format!("/api/providers/{}", compatible.id);
    let set = backend
        .patch(
            &path,
            &master,
            json!({ "baseUrl": vendor.url("/v1"), "models": [configured("m")] }),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK);
    assert_eq!(
        availability_of(&backend, &master, &compatible.id).await,
        json!({ "type": "available" })
    );
    let test = format!("{path}/test");
    vendor.respond(answered());
    let passed = backend
        .post(&test, Some(&master), json!({ "modelId": "m" }))
        .await
        .json::<TestResult>();
    assert!(matches!(passed, TestResult::Passed { .. }), "{passed:?}");
    let sent = vendor.requests().pop().unwrap();
    assert_eq!((sent.uri.path(), sent.header("x-api-key")), ("/v1/messages", None));

    // A key saved later is sent; the protocol changes and keeps the
    // endpoint, the key and the models, and the next request speaks it.
    let keyed = backend
        .patch(&path, &master, json!({ "apiKey": "sk-proxy" }))
        .await;
    assert!(keyed.json::<ProviderAnswer>().provider.has_api_key);
    let switched = backend
        .patch(
            &path,
            &master,
            json!({ "providerType": "openai", "wireApi": "chat-completions" }),
        )
        .await
        .json::<ProviderAnswer>()
        .provider;
    assert_eq!(
        (
            switched.provider_type.as_str(),
            switched.wire_api,
            switched.endpoint.as_ref().map(|url| url.as_str().to_owned()),
            switched.has_api_key,
            switched.models.as_ref().map(|models| models.0.len()),
        ),
        (
            "openai",
            Some(WireApi::ChatCompletions),
            Some(vendor.url("/v1")),
            true,
            Some(1)
        )
    );
    vendor.respond(MockResponse::event_stream(format!(
        "{}data: [DONE]\n\n",
        sse_body(&[json!({ "choices": [{ "delta": { "content": "ok" }, "finish_reason": "stop" }] })])
    )));
    let passed = backend
        .post(&test, Some(&master), json!({ "modelId": "m" }))
        .await
        .json::<TestResult>();
    assert!(matches!(passed, TestResult::Passed { .. }), "{passed:?}");
    let sent = vendor.requests().pop().unwrap();
    assert_eq!(
        (sent.uri.path(), sent.header("authorization")),
        ("/v1/chat/completions", Some("Bearer sk-proxy"))
    );
    // Back to Anthropic Messages: the wire goes with the family.
    let back = backend
        .patch(&path, &master, json!({ "providerType": "anthropic" }))
        .await
        .json::<ProviderAnswer>()
        .provider;
    assert_eq!((back.provider_type.as_str(), back.wire_api), ("anthropic", None));

    // The key and the endpoint can be removed again; without an endpoint
    // the entry is incomplete once more.
    let removed = backend
        .patch(&path, &master, json!({ "apiKey": null, "baseUrl": null }))
        .await
        .json::<ProviderAnswer>()
        .provider;
    assert_eq!((removed.has_api_key, removed.endpoint), (false, None));
    assert_eq!(
        availability_of(&backend, &master, &compatible.id).await,
        setup(&["endpoint"])
    );

    // A model list whose ids repeat is refused as such.
    let repeated = backend
        .patch(
            &path,
            &master,
            json!({ "models": [configured("m"), configured("m")] }),
        )
        .await;
    assert_eq!(
        repeated.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::DuplicateModel)
    );

    // A model added by Custom has no window yet: it is kept, and listed
    // with its window unknown.
    let mut custom = configured("custom");
    custom["contextWindow"] = Value::Null;
    let kept = backend
        .patch(&path, &master, json!({ "models": [configured("m"), custom] }))
        .await;
    assert_eq!(kept.status, StatusCode::OK, "{}", String::from_utf8_lossy(&kept.body));
    let listed = catalog_of(&backend, &master, "").await;
    let windows: Vec<_> = listed
        .providers
        .iter()
        .find(|provider| provider.provider_id == compatible.id)
        .expect("the entry is listed")
        .models
        .iter()
        .map(|model| (model.model.id.as_str(), model.model.context_window))
        .collect();
    assert_eq!(windows, [("m", Some(64_000)), ("custom", None)]);

    // A vendor entry, the family's own included, keeps its vendor's
    // endpoint and protocol.
    let anthropic = create(
        &backend,
        &master,
        json!({ "source": "family", "providerType": "anthropic", "apiKey": "sk-ant-1" }),
    )
    .await;
    assert_eq!(
        (
            anthropic.compatible,
            anthropic.vendor_id.as_deref(),
            anthropic.endpoint.as_ref().map(|url| url.as_str())
        ),
        (false, Some("anthropic"), Some("https://api.anthropic.com/v1"))
    );
    let own = format!("/api/providers/{}", anthropic.id);
    for change in [
        json!({ "baseUrl": "https://proxy.example/v1" }),
        json!({ "providerType": "openai" }),
    ] {
        let refused = backend.patch(&own, &master, change.clone()).await;
        assert_eq!(
            refused.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::EndpointKind),
            "{change}"
        );
    }
    let by_vendor = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({ "source": "vendor", "vendorId": "anthropic", "baseUrl": "https://proxy.example/v1" }),
        )
        .await;
    assert_eq!(
        by_vendor.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)
    );
    backend.close().await;
}

/// A models.dev document where Google publishes Gemini, among models no
/// agent can work with, an aggregator and a reseller list one of its models
/// again, and a vendor lists 22 models with "flash" in their names.
fn searched_document() -> Value {
    let gemini = |name: &str, date: &str, output: u32| {
        json!({
            "name": name, "release_date": date, "reasoning": true, "tool_call": true,
            "reasoning_options": [{ "type": "effort", "values": ["low", "medium", "high"] }],
            "modalities": { "input": ["text", "image", "pdf"], "output": ["text"] },
            "limit": { "context": 1_048_576, "output": output }
        })
    };
    // Newer than every match, but no agent can work with them: one speaks,
    // one draws, one calls no tool, one states no window.
    let mut speaks = gemini("Gemini 3.8 Flash TTS", "2026-10-01", 8_192);
    speaks["modalities"]["output"] = json!(["audio"]);
    let mut draws = gemini("Gemini 3.8 Flash Image", "2026-10-01", 8_192);
    draws["modalities"]["output"] = json!(["image"]);
    let mut toolless = gemini("Gemini 3.8 Flash Chat", "2026-10-01", 8_192);
    toolless["tool_call"] = json!(false);
    let mut windowless = gemini("Gemini 3.8 Flash Live", "2026-10-01", 8_192);
    windowless["limit"] = json!({ "output": 8_192 });
    let mut flagged = gemini("Gemini 3.8 Flash", "2026-09-01", 65_536);
    flagged["canonical_model_id"] = json!("google/gemini-3.8-flash");
    let mut routed = gemini("Google: Gemini 3.8 Flash", "2026-09-01", 32_768);
    routed["canonical_model_id"] = json!("google/gemini-3.8-flash");
    let many: serde_json::Map<String, Value> = (1..=22)
        .map(|day| {
            (
                format!("spark-flash-{day}"),
                json!({
                    "name": format!("Spark Flash {day}"), "release_date": format!("2025-01-{day:02}"),
                    "tool_call": true, "modalities": { "output": ["text"] },
                    "limit": { "context": 128_000 }
                }),
            )
        })
        .collect();
    json!({
        "acme": {
            "id": "acme", "name": "Acme", "npm": "@ai-sdk/openai-compatible", "api": "https://acme.example.test/v1",
            "models": { "gemini-3.8-flash": gemini("Gemini 3.8 Flash (Acme)", "2026-09-01", 8_192) }
        },
        "google": {
            "id": "google", "name": "Google", "npm": "@ai-sdk/google",
            "models": {
                "gemini-3.5-flash": gemini("Gemini 3.5 Flash", "2026-03-01", 65_536),
                "gemini-3.8-flash": flagged,
                "gemini-3.8-flash-lite": gemini("Gemini 3.8 Flash Lite", "2026-09-15", 65_536),
                "gemini-3.8-pro": gemini("Gemini 3.8 Pro", "2026-09-20", 65_536),
                "gemini-3.8-flash-tts": speaks,
                "gemini-3.8-flash-image": draws,
                "gemini-3.8-flash-chat": toolless,
                "gemini-3.8-flash-live": windowless
            }
        },
        "openrouter": {
            "id": "openrouter", "name": "OpenRouter", "npm": "@ai-sdk/openai-compatible",
            "api": "https://openrouter.example.test/api/v1",
            "models": { "google/gemini-3.8-flash": routed }
        },
        "sparks": { "id": "sparks", "name": "Sparks", "npm": "@ai-sdk/openai-compatible", "models": many }
    })
}

async fn searched(backend: &TestBackend, session: &Session, query: &str) -> Vec<Value> {
    let answer = backend
        .get(&format!("/api/models/search?{query}"), Some(session))
        .await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    answer.json::<Value>()["results"]
        .as_array()
        .expect("results are a list")
        .clone()
}

#[tokio::test]
async fn the_model_search_matches_every_word_lists_a_model_once_newest_first_and_fills_from_the_entrys_vendor()
 {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_models_dev(vendor.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;
    vendor.respond(served(&searched_document()));

    // Every word, in any case and order, of the models that write text,
    // call tools and state their window; the model three vendors list is
    // one match, filled from its publisher.
    let found = searched(&backend, &master, "q=FLASH%20gemini").await;
    let refs: Vec<&str> = found
        .iter()
        .map(|result| result["ref"].as_str().unwrap())
        .collect();
    assert_eq!(
        refs,
        [
            "google/gemini-3.8-flash-lite",
            "google/gemini-3.8-flash",
            "google/gemini-3.5-flash"
        ]
    );
    assert_eq!(
        found[1],
        json!({
            "ref": "google/gemini-3.8-flash", "displayName": "Gemini 3.8 Flash", "id": "gemini-3.8-flash",
            "vendor": "google", "contextWindow": 1_048_576, "outputLimit": 65_536,
            "thinkingEfforts": ["low", "medium", "high"],
            "acceptedExtensions": ["png", "jpg", "jpeg", "gif", "webp", "pdf"], "releaseDate": "2026-09-01"
        })
    );

    // From an entry whose vendor lists the model, the vendor's own listing.
    let routed = create(
        &backend,
        &master,
        json!({ "source": "vendor", "vendorId": "openrouter" }),
    )
    .await;
    let found = searched(
        &backend,
        &master,
        &format!("q=gemini%203.8%20flash&entry={}", routed.id),
    )
    .await;
    let filled: Vec<(&str, u64)> = found
        .iter()
        .map(|result| {
            (
                result["ref"].as_str().unwrap(),
                result["outputLimit"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        filled,
        [
            ("google/gemini-3.8-flash-lite", 65_536),
            ("openrouter/google/gemini-3.8-flash", 32_768)
        ]
    );

    // At most twenty, and a query without a word finds nothing.
    assert_eq!(searched(&backend, &master, "q=flash").await.len(), 20);
    assert_eq!(searched(&backend, &master, "q=%20").await, Vec::<Value>::new());
    // Every search read the copy the first one fetched.
    assert_eq!(vendor.requests().len(), 1);
    backend.close().await;
}
