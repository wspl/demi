//! The conversation browser's tab methods (`live-view.md` § The tab
//! methods), called through the plugin call route (`web-api.md` § Plugin
//! calls) against the `demi.browser` package the workspace built,
//! on a paired device's real runner, on a Cloud that never started, and on
//! a running Cloud, which listing its tabs does not keep awake
//! (`resource-lifecycle.md` § Activity). No browser runs: the operations
//! answer as a browser that does not run does, so no Chrome is needed.
//! Opening a tab starts one, which the browser's own tests and the live
//! view's acceptance cover.

use std::time::Duration;

use demi_shared_gates::Purpose;
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::cloud::{idle_after, the_cloud};
use crate::support::{Harness, PATIENCE, Session, TestBackend};

/// A tab id the browser never gave out.
const ABSENT: &str = "t999999";

async fn conversation(backend: &TestBackend, master: &Session) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let created = backend
        .post("/api/conversations", Some(master), json!({ "id": id }))
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    id
}

/// The route of the browser plugin's method `method` for conversation `id`.
fn method(id: &str, method: &str) -> String {
    format!("/api/conversations/{id}/plugins/browser/calls/{method}")
}

/// The tab method `name` with `body`, as the work panel calls it.
async fn call(
    backend: &TestBackend,
    master: &Session,
    id: &str,
    name: &str,
    body: Value,
) -> crate::support::Answer {
    backend.post(&method(id, name), Some(master), body).await
}

/// The plugin's refusal: `plugin_refused` with its own reason.
fn plugin_refusal(answer: &crate::support::Answer) -> (StatusCode, ErrorCode, Option<String>) {
    let error = answer.error();
    (answer.status, error.code, error.reason)
}

// Over a second: a real device installs the builtin package, whose browser
// service answers the methods.
#[tokio::test]
async fn the_tab_methods_run_the_browsers_operations_as_the_user_on_the_conversations_host() {
    let harness = Harness::new().with_browser_package();
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let id = conversation(&backend, &master).await;
    let home = laptop.runner.home_dir().to_str().unwrap().to_owned();
    let target = json!({ "target": { "kind": "device", "deviceId": laptop.id(), "path": home } });
    let moved = backend
        .patch(&format!("/api/conversations/{id}"), &master, target)
        .await;
    assert_eq!(
        moved.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&moved.body)
    );

    // A browser that does not run has no tabs, and nothing to close.
    let listed = call(&backend, &master, &id, "tabs", json!({})).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    assert_eq!(listed.json::<Value>(), json!({ "tabs": [] }));
    for tab in [ABSENT, "not-a-tab"] {
        let closed = call(&backend, &master, &id, "close", json!({ "tab": tab })).await;
        assert_eq!(closed.status, StatusCode::OK, "{tab}");
        assert_eq!(closed.json::<Value>(), Value::Null);
    }
    // A tab the browser does not have is the browser's answer.
    let tab_not_found = (
        StatusCode::CONFLICT,
        ErrorCode::PluginRefused,
        Some("tab_not_found".to_owned()),
    );
    let navigate = json!({ "tab": ABSENT, "url": "https://example.test/" });
    let missing = call(&backend, &master, &id, "navigate", navigate).await;
    assert_eq!(plugin_refusal(&missing), tab_not_found);
    for tab in [ABSENT, "not-a-tab"] {
        let reload = json!({ "tab": tab, "action": "reload" });
        let moved = call(&backend, &master, &id, "history", reload).await;
        assert_eq!(plugin_refusal(&moved), tab_not_found, "{tab}");
    }
    for (name, body) in [
        ("navigate", json!({ "tab": ABSENT })),
        ("history", json!({ "tab": ABSENT, "action": "sideways" })),
        ("open", json!({ "url": "" })),
    ] {
        let refused = call(&backend, &master, &id, name, body.clone()).await;
        assert_eq!(
            refused.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
            "{name} {body}"
        );
    }

    // Without its runner the device answers nothing.
    laptop.runner.kill().await;
    backend.until_online(&master, laptop.id(), false).await;
    let offline = call(&backend, &master, &id, "tabs", json!({})).await;
    assert_eq!(
        offline.refusal(),
        (StatusCode::CONFLICT, ErrorCode::DeviceOffline)
    );

    // An archived conversation's browser is not operated.
    let archived = backend
        .patch(
            &format!("/api/conversations/{id}"),
            &master,
            json!({ "archived": true }),
        )
        .await;
    assert_eq!(
        archived.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&archived.body)
    );
    let listed = call(&backend, &master, &id, "tabs", json!({})).await;
    assert_eq!(
        listed.refusal(),
        (StatusCode::CONFLICT, ErrorCode::ConversationArchived)
    );
    let opened = call(&backend, &master, &id, "open", json!({})).await;
    assert_eq!(
        opened.refusal(),
        (StatusCode::CONFLICT, ErrorCode::ConversationArchived)
    );
    let elsewhere = uuid::Uuid::new_v4().to_string();
    let unknown = call(&backend, &master, &elsewhere, "tabs", json!({})).await;
    assert_eq!(
        unknown.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound)
    );
    backend.close().await;
}

#[tokio::test]
async fn a_stopped_cloud_is_not_woken_to_list_close_or_move_its_tabs() {
    let harness = Harness::new().with_browser_package();
    let (backend, master) = harness.start_set_up().await;
    // The pages see the tab methods and the stream the catalog serves.
    let state = backend.sync(&master).await.snapshot().await;
    let browser = state
        .plugins
        .iter()
        .find(|plugin| plugin.id == "browser")
        .unwrap();
    assert_eq!(
        browser.methods,
        ["tabs", "open", "close", "navigate", "history"]
    );
    assert_eq!(browser.streams, ["browser"]);
    // A new conversation works on the Cloud, which never started.
    let id = conversation(&backend, &master).await;
    let listed = call(&backend, &master, &id, "tabs", json!({})).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    assert_eq!(listed.json::<Value>(), json!({ "tabs": [] }));
    let closed = call(&backend, &master, &id, "close", json!({ "tab": ABSENT })).await;
    assert_eq!(closed.status, StatusCode::OK);
    let navigate = json!({ "tab": ABSENT, "url": "https://example.test/" });
    let stopped = call(&backend, &master, &id, "navigate", navigate).await;
    assert_eq!(
        stopped.refusal(),
        (StatusCode::CONFLICT, ErrorCode::HostStopped)
    );
    let forward = json!({ "tab": ABSENT, "action": "forward" });
    let stopped = call(&backend, &master, &id, "history", forward).await;
    assert_eq!(
        stopped.refusal(),
        (StatusCode::CONFLICT, ErrorCode::HostStopped)
    );
    // Looking made no Cloud.
    let devices: Value = backend.get("/api/devices", Some(&master)).await.json();
    assert_eq!(devices["devices"], json!([]));
    backend.close().await;
}

// Several seconds: the Cloud boots, the tab close installs the builtin
// package on its runner, and the Cloud then idles for a window.
#[tokio::test]
async fn listing_a_running_clouds_tabs_does_not_keep_it_awake() {
    // Far longer than the time between two listings, so that listings counted
    // as activity would keep the Cloud up.
    let window = Duration::from_millis(800);
    let mut harness = Harness::new().with_browser_package();
    harness.lifecycle = idle_after(window);
    harness.cloud.sweep = Duration::from_millis(50);
    let (backend, master) = harness.start_set_up().await;
    let id = conversation(&backend, &master).await;
    // Reading the conversation's files wakes the Cloud it works on.
    let listed = backend
        .get(&format!("/api/conversations/{id}/fs"), Some(&master))
        .await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    let device = the_cloud(&harness);
    // Closing a tab starts the browser's service, which the Cloud's runner
    // installs first. The close restarts the window as it is admitted and is
    // no activity after that, so a lease of the conversation's file gate,
    // which is its work, keeps the Cloud up meanwhile.
    let working = backend
        .file_gate(&master, &id)
        .await
        .enter(Purpose::Demand)
        .await;
    let closed = call(&backend, &master, &id, "close", json!({ "tab": ABSENT })).await;
    assert_eq!(
        closed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&closed.body)
    );
    let rested = Instant::now();
    drop(working);
    // The page lists the tabs again and again, and the Cloud idles and
    // stops all the same, a window after the last activity. A listing the
    // stop overtakes finds the runner gone.
    let listing = async {
        loop {
            let listed = call(&backend, &master, &id, "tabs", json!({})).await;
            let answered = match listed.status {
                StatusCode::OK => true,
                StatusCode::CONFLICT => listed.refusal().1 == ErrorCode::DeviceOffline,
                _ => false,
            };
            assert!(answered, "{}", String::from_utf8_lossy(&listed.body));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let hibernate = format!("hibernate:{device}");
    let stopped = tokio::select! {
        stopped = harness.manager.arrival(&hibernate, PATIENCE) => stopped,
        _ = listing => unreachable!("the page lists until the Cloud stops"),
    };
    assert!(
        stopped >= rested + window,
        "the Cloud stopped {:?} after the last activity",
        stopped.saturating_duration_since(rested)
    );
    backend.close().await;
}

#[tokio::test]
async fn a_backend_whose_catalog_serves_no_browser_has_no_tab_methods() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let id = conversation(&backend, &master).await;
    let listed = call(&backend, &master, &id, "tabs", json!({})).await;
    assert_eq!(
        listed.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::UnknownPluginMethod)
    );
    // The pages see none, so they offer no browser tab.
    let state = backend.sync(&master).await.snapshot().await;
    let browser = state
        .plugins
        .iter()
        .find(|plugin| plugin.id == "browser")
        .unwrap();
    assert!(
        browser.methods.is_empty() && browser.streams.is_empty(),
        "{browser:?}"
    );
    backend.close().await;
}
