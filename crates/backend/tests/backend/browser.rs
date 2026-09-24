//! The conversation browser's tab routes (`web-api.md` § Conversation
//! browser tabs), against the `demi.builtin` package the workspace built,
//! on a paired device's real runner and on a Cloud that never started. No
//! browser runs: the operations answer as a browser that does not run
//! does, so no Chrome is needed. Opening a tab starts one, which the
//! browser's own tests and the live view's acceptance cover.

use demi_web_api::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::support::{Harness, Session, TestBackend};

/// A tab id the browser never gave out.
const ABSENT: &str = "t_nosuchtabnosuchtabnosu";

async fn conversation(backend: &TestBackend, master: &Session) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let created = backend.post("/api/conversations", Some(master), json!({ "id": id })).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    id
}

fn tabs(id: &str) -> String {
    format!("/api/conversations/{id}/browser/tabs")
}

#[tokio::test]
async fn the_tab_routes_run_the_browsers_operations_as_the_user_on_the_conversations_host() {
    let harness = Harness::new().with_builtin_package();
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let id = conversation(&backend, &master).await;
    let home = laptop.runner.home_dir().to_str().unwrap().to_owned();
    let target = json!({ "target": { "kind": "device", "deviceId": laptop.id(), "path": home } });
    let moved = backend.patch(&format!("/api/conversations/{id}"), &master, target).await;
    assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));

    // A browser that does not run has no tabs, and nothing to close.
    let listed = backend.get(&tabs(&id), Some(&master)).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&listed.body));
    assert_eq!(listed.json::<Value>(), json!({ "tabs": [] }));
    assert_eq!(backend.delete(&format!("{}/{ABSENT}", tabs(&id)), &master).await.status, StatusCode::NO_CONTENT);
    assert_eq!(backend.delete(&format!("{}/not-a-tab", tabs(&id)), &master).await.status, StatusCode::NO_CONTENT);
    // A tab the browser does not have is the browser's answer.
    let navigate = format!("{}/{ABSENT}/navigate", tabs(&id));
    let missing = backend.post(&navigate, Some(&master), json!({ "url": "https://example.test/" })).await;
    assert_eq!(missing.refusal(), (StatusCode::NOT_FOUND, ErrorCode::TabNotFound));
    let history = format!("{}/{ABSENT}/history", tabs(&id));
    let reload = backend.post(&history, Some(&master), json!({ "action": "reload" })).await;
    assert_eq!(reload.refusal(), (StatusCode::NOT_FOUND, ErrorCode::TabNotFound));
    let malformed = backend
        .post(&format!("{}/not-a-tab/history", tabs(&id)), Some(&master), json!({ "action": "back" }))
        .await;
    assert_eq!(malformed.refusal(), (StatusCode::NOT_FOUND, ErrorCode::TabNotFound));
    for (path, body) in [
        (&navigate, json!({})),
        (&history, json!({ "action": "sideways" })),
        (&tabs(&id), json!({ "url": "" })),
    ] {
        let refused = backend.post(path, Some(&master), body.clone()).await;
        assert_eq!(refused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody), "{path} {body}");
    }

    // Without its runner the device answers nothing.
    laptop.runner.kill().await;
    backend.until_online(&master, laptop.id(), false).await;
    let offline = backend.get(&tabs(&id), Some(&master)).await;
    assert_eq!(offline.refusal(), (StatusCode::CONFLICT, ErrorCode::DeviceOffline));

    // An archived conversation's browser is not operated.
    let archived = backend
        .patch(&format!("/api/conversations/{id}"), &master, json!({ "archived": true }))
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{}", String::from_utf8_lossy(&archived.body));
    let listed = backend.get(&tabs(&id), Some(&master)).await;
    assert_eq!(listed.refusal(), (StatusCode::CONFLICT, ErrorCode::ConversationArchived));
    let opened = backend.post(&tabs(&id), Some(&master), json!({})).await;
    assert_eq!(opened.refusal(), (StatusCode::CONFLICT, ErrorCode::ConversationArchived));
    let unknown = backend.get(&tabs(&uuid::Uuid::new_v4().to_string()), Some(&master)).await;
    assert_eq!(unknown.refusal(), (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound));
    backend.close().await;
}

#[tokio::test]
async fn a_stopped_cloud_is_not_woken_to_list_close_or_move_its_tabs() {
    let harness = Harness::new().with_builtin_package();
    let (backend, master) = harness.start_set_up().await;
    // A new conversation works on the Cloud, which never started.
    let id = conversation(&backend, &master).await;
    let listed = backend.get(&tabs(&id), Some(&master)).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&listed.body));
    assert_eq!(listed.json::<Value>(), json!({ "tabs": [] }));
    assert_eq!(backend.delete(&format!("{}/{ABSENT}", tabs(&id)), &master).await.status, StatusCode::NO_CONTENT);
    let navigate = format!("{}/{ABSENT}/navigate", tabs(&id));
    let stopped = backend.post(&navigate, Some(&master), json!({ "url": "https://example.test/" })).await;
    assert_eq!(stopped.refusal(), (StatusCode::CONFLICT, ErrorCode::HostStopped));
    let history = format!("{}/{ABSENT}/history", tabs(&id));
    let stopped = backend.post(&history, Some(&master), json!({ "action": "forward" })).await;
    assert_eq!(stopped.refusal(), (StatusCode::CONFLICT, ErrorCode::HostStopped));
    // Looking made no Cloud.
    let devices: Value = backend.get("/api/devices", Some(&master)).await.json();
    assert_eq!(devices["devices"], json!([]));
    backend.close().await;
}

#[tokio::test]
async fn a_backend_whose_catalog_serves_no_browser_has_no_tab_routes() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let id = conversation(&backend, &master).await;
    let listed = backend.get(&tabs(&id), Some(&master)).await;
    assert_eq!(listed.refusal(), (StatusCode::NOT_FOUND, ErrorCode::NotFound));
    backend.close().await;
}
