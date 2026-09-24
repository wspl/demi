//! The product snapshot the page loads and revalidates (`web-api.md` §
//! Sidebar mutations, read state and page synchronization).

use demi_core::AuthState;
use demi_provider::quota::ProbeCost;
use demi_web_api::auth::Role;
use demi_web_api::error::ErrorCode;
use demi_web_api::providers::{ProviderDetails, ProviderReading};
use demi_web_api::settings::{InstanceMode, Preferences, Theme};
use demi_web_api::state::ProductState;
use reqwest::StatusCode;
use serde_json::json;

use crate::accounts::{device_entry, scripts};
use crate::support::Harness;

#[tokio::test]
async fn the_state_is_the_users_snapshot_and_revalidates_by_its_etag() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    let first = backend.get("/api/state", Some(&master)).await;
    assert_eq!(first.status, StatusCode::OK);
    let state = first.json::<ProductState>();
    assert_eq!(
        state,
        ProductState {
            user: master.user.clone(),
            mode: InstanceMode::Shared,
            preferences: Preferences::default(),
            providers: Vec::new(),
            workspaces: Vec::new(),
            devices: Vec::new(),
            exposes: Vec::new(),
            expose_domain: None,
            conversations: Vec::new(),
        }
    );
    let etag = first.headers["etag"].to_str().unwrap().to_owned();
    assert!(etag.len() == 66 && etag.starts_with('"') && etag.ends_with('"'), "{etag}");
    assert_eq!(first.headers["cache-control"], "private, no-cache");
    assert_eq!(first.headers["content-type"], "application/json");
    let body = String::from_utf8(first.body.clone()).unwrap();
    assert!(!body.contains("passwordHash") && !body.contains("$argon2"), "{body}");

    let unchanged = backend.get_with("/api/state", &master, &[("if-none-match", &etag)]).await;
    assert_eq!(unchanged.status, StatusCode::NOT_MODIFIED);
    assert!(unchanged.body.is_empty());
    assert_eq!(unchanged.headers["etag"].to_str().unwrap(), etag);
    assert_eq!(unchanged.headers["cache-control"], "private, no-cache");

    let dark = backend
        .patch("/api/settings/preferences", &master, json!({ "appearance": { "theme": "dark" } }))
        .await;
    assert_eq!(dark.status, StatusCode::OK);
    let changed = backend.get_with("/api/state", &master, &[("if-none-match", &etag)]).await;
    assert_eq!(changed.status, StatusCode::OK);
    assert_eq!(changed.json::<ProductState>().preferences.appearance.theme, Some(Theme::Dark));
    let changed_etag = changed.headers["etag"].to_str().unwrap().to_owned();
    assert_ne!(changed_etag, etag);

    // An existing user's snapshot after a restart is the same body, so the
    // same ETag.
    backend.close().await;
    let backend = harness.start().await;
    let again = backend
        .get_with("/api/state", &master, &[("if-none-match", &changed_etag)])
        .await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);

    let anonymous = backend.get("/api/state", None).await;
    assert_eq!(anonymous.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated));
    backend.close().await;
}

fn details(reading: &ProviderReading) -> &ProviderDetails {
    let ProviderReading::Read(details) = reading else {
        panic!("the entry could not be read: {reading:?}");
    };
    details
}

#[tokio::test]
async fn the_state_carries_the_entries_the_user_infers_with_and_only_a_configuring_user_sees_their_accounts() {
    let scripts = scripts(Some(ProbeCost::Free));
    let harness = Harness::new().with_families(scripts.families.clone());
    let (backend, master) = harness.start_set_up().await;
    let empty = backend.get("/api/state", Some(&master)).await;
    let etag = empty.headers["etag"].to_str().unwrap().to_owned();
    let device = device_entry(&backend, &master, &scripts).await;
    let keyed = backend
        .post("/api/providers", Some(&master), json!({ "source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-state" }))
        .await;
    assert_eq!(keyed.status, StatusCode::CREATED);

    let changed = backend.get_with("/api/state", &master, &[("if-none-match", &etag)]).await;
    assert_eq!(changed.status, StatusCode::OK);
    assert!(!String::from_utf8_lossy(&changed.body).contains("sk-state"));
    let state = changed.json::<ProductState>();
    let labels: Vec<&str> = state.providers.iter().map(|entry| entry.provider.label.as_str()).collect();
    assert_eq!(labels, ["device subscription", "Work"]);
    let subscription = details(&state.providers[0].details);
    assert_eq!(subscription.accounts.len(), 1);
    assert_eq!(subscription.active.as_ref().map(|id| id.as_str()), Some(subscription.accounts[0].account.id.as_str()));
    assert!(matches!(details(&state.providers[1].details).auth, AuthState::Authenticated { .. }));
    assert_eq!(state.providers[0].provider.id, device.id);

    harness.add_user("reader@example.test", "reader-pass-1", Role::User);
    let reader = backend.login("reader@example.test", "reader-pass-1").await;
    let seen = backend.get("/api/state", Some(&reader)).await.json::<ProductState>();
    let subscription = details(&seen.providers[0].details);
    assert_eq!((subscription.accounts.len(), subscription.active.clone(), subscription.quota.clone()), (0, None, None));
    assert_eq!(subscription.auth, AuthState::Authenticated { account_label: None });
    backend.close().await;
}
