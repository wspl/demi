//! The agent's `demi providers` (`models.md` § Managing providers from a
//! conversation): reads need no permission and the first change asks for
//! Manage Providers; then the agent adds an entry from the vendor list,
//! searches models.dev and adds a model filled from a match, which starts
//! the entry's own list from its vendor's models, and saves a
//! key it pipes in from a file without the key reaching its transcript;
//! a subscription entry's list refuses with the page's words. The model is
//! the scripted family of the subagent scenarios, models.dev a scripted
//! vendor, and the device a real runner. No test calls a real model.

use std::sync::Arc;

use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_web_api_protocol::providers::{LoginState, Providers};
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::accounts::{awaited, ended, start_login};
use crate::agent_profiles::{assert_shows, last_root};
use crate::conversations::{FIRST, Socket};
use crate::permissions::{decide, permissions};
use crate::providers::served;
use crate::subagents::{Scripts, say, shell, tree_among};
use crate::support::Harness;

/// The key the agent pipes in; no output may show it.
const KEY: &str = "sk-secret-0123456789";

fn document() -> Value {
    json!({
        "google": {
            "id": "google", "name": "Google", "npm": "@ai-sdk/google",
            "models": {
                "gemini-3.8-flash": {
                    "name": "Gemini 3.8 Flash", "release_date": "2026-09-01", "reasoning": true, "tool_call": true,
                    "reasoning_options": [{ "type": "effort", "values": ["low", "medium", "high"] }],
                    "modalities": { "input": ["text", "image", "pdf"], "output": ["text"] },
                    "limit": { "context": 1_048_576, "output": 65_536 }
                },
                "gemini-3.8-pro": { "name": "Gemini 3.8 Pro", "limit": { "context": 1_048_576 } }
            }
        }
    })
}

// About 0.3 s: a real device runs the shell jobs of two turns, the
// second woken by the user's decision, and a login signs an account in.
#[tokio::test]
async fn an_agent_adds_a_provider_and_a_model_from_models_dev_under_the_permission_without_seeing_a_key()
 {
    let vendor = MockVendor::start().await;
    let scripts = Arc::new(Scripts::default());
    let logins = crate::accounts::scripts(None);
    let harness = Harness::new().with_models_dev(vendor.url("/api.json"));
    let (_harness, backend, master, _device, root, _tree) =
        tree_among(&scripts, harness, logins.families.clone()).await;
    vendor.respond(served(&document()));
    for _ in 0..8 {
        vendor.respond(MockResponse::status(304));
    }
    // A subscription entry, whose models come from its provider.
    logins.login.approve(true);
    let login = start_login(
        &backend,
        &master,
        "/api/providers/subscription-login",
        json!({ "providerType": "device", "label": "Claude Code" }),
    )
    .await;
    let state = awaited(&backend, &master, &login, ended).await;
    assert!(matches!(state, LoginState::Completed { .. }), "{state:?}");
    std::fs::write(format!("{root}/key.txt"), format!("{KEY}\n")).unwrap();
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // A read needs no permission; the first change asks for it.
    let add = "demi providers add --vendor google";
    scripts.root(
        FIRST,
        vec![
            shell("t1", &format!("demi providers list; {add}")),
            say("asked"),
        ],
    );
    socket.chat("m1", "Add Google").await;
    let asked = last_root(&scripts);
    assert_shows(&asked, "Tree · API key · Ready\n* m  M");
    assert_shows(&asked, "Claude Code · subscription · Ready\n");
    assert_shows(
        &asked,
        "demi providers add: this conversation needs the user's permission to manage providers; the request was sent to the user, and you will be told when the user decides",
    );
    let requested = permissions(&backend, &master, FIRST).await;
    assert_eq!(requested.requests.len(), 1, "{requested:?}");
    let request = &requested.requests[0];
    assert_eq!(request.command(), add);
    let category = &crate::permissions::categories(request)[0];
    assert_eq!(
        (category.id.as_str(), category.action.as_deref()),
        ("providers.manage", Some("manage providers"))
    );
    assert_eq!(
        category.description.as_deref(),
        Some(
            "Add, change and remove your provider entries and their models, which every conversation of yours can use."
        )
    );

    // Allowed: the entry joins incomplete, the search fills a model, and
    // the key piped from a file makes it ready; a subscription's list and
    // a key of two lines are refused with the page's words.
    scripts.root(
        FIRST,
        vec![
            shell(
                "t2",
                &[
                    add,
                    "demi providers models search gemini flash",
                    "demi providers models add Google google/gemini-3.8-flash",
                    "demi providers edit Google --api-key-stdin < key.txt",
                    "demi providers show Google --json",
                    "demi providers list --json",
                    "demi providers models add 'Claude Code' google/gemini-3.8-flash",
                    "printf 'two\\nlines\\n' | demi providers edit Google --api-key-stdin",
                ]
                .join("\n"),
            ),
            say("added"),
        ],
    );
    let allowed = decide(&backend, &master, FIRST, request.id().as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    socket.until_idle().await;
    let asked = last_root(&scripts);
    for shown in [
        "state     Needs an API key\n",
        "google/gemini-3.8-flash  Gemini 3.8 Flash  1M  out 65.5K  low* medium high  png jpg jpeg gif webp pdf\n",
        "Google now keeps its own list of 2 models; it no longer follows models.dev.\nAdded to Google: gemini-3.8-flash  Gemini 3.8 Flash  1M  out 65.5K  low* medium high  png jpg jpeg gif webp pdf\n",
        "key       saved\nstate     Ready\n",
        "\"keySaved\":true",
        "demi providers models add: Claude Code lists its own models; its list cannot be edited.",
        "demi providers edit: apiKey: the credential contains a control character",
    ] {
        assert_shows(&asked, shown);
    }
    assert!(!asked.contains(KEY), "a key reached the transcript: {asked}");

    // The model holds every fact models.dev states, as the page saves one,
    // and the vendor's other model stays.
    let providers = backend
        .get("/api/providers", Some(&master))
        .await
        .json::<Providers>()
        .providers;
    let google = providers
        .iter()
        .find(|provider| provider.label == "Google")
        .expect("Google was added");
    assert_eq!(
        serde_json::to_value(&google.models).unwrap(),
        json!([
            {
                "id": "gemini-3.8-pro", "displayName": "Gemini 3.8 Pro", "contextWindow": 1_048_576,
                "outputLimit": null, "thinkingEfforts": [], "acceptedExtensions": null, "fastTier": null
            },
            {
                "id": "gemini-3.8-flash", "displayName": "Gemini 3.8 Flash", "contextWindow": 1_048_576,
                "outputLimit": 65_536, "thinkingEfforts": ["low", "medium", "high"],
                "acceptedExtensions": ["png", "jpg", "jpeg", "gif", "webp", "pdf"], "fastTier": null
            }
        ])
    );
    assert!(google.has_api_key);
    backend.close().await;
}
