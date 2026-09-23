//! The product snapshot the page loads and revalidates (`web-api.md` §
//! Sidebar mutations, read state and page synchronization).

use demi_web_api::error::ErrorCode;
use demi_web_api::settings::{InstanceMode, Preferences, Theme};
use demi_web_api::state::ProductState;
use reqwest::StatusCode;
use serde_json::json;

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
