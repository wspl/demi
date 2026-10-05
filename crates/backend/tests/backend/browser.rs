//! The conversation browser's tab list and tab methods (`live-view.md` § The
//! tab methods), read through the conversation state route and called
//! through the plugin call route (`web-api.md` § Conversation state of
//! plugins, § Plugin calls) against the `demi.browser` package the workspace
//! built,
//! on a paired device's real runner, on a Cloud that never started, and on
//! a running Cloud, which listing its tabs does not keep awake
//! (`resource-lifecycle.md` § Activity). No browser runs: the operations
//! answer as a browser that does not run does, so no Chrome is needed.
//! Opening a tab starts one, which the browser's own tests and the live
//! view's acceptance cover.

use std::time::Duration;

use demi_command_package_browser_protocol::release::{ARTIFACT, BrowserRelease};
use demi_shared_gates::Purpose;
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::time::Instant;

use demi_provider_common::testing::MockVendor;
use demi_web_api_protocol::conversations::{ConversationSummary, PluginRevision};

use crate::cloud::{idle_after, the_cloud};
use crate::conversations::{anthropic_at, create, on_device, summaries};
use crate::support::{Harness, PATIENCE, Session, TestBackend};
use crate::work::{Driven, say, shell};

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

/// The browser plugin's conversation state, the tab list, as the work panel reads it.
async fn tabs(backend: &TestBackend, master: &Session, id: &str) -> crate::support::Answer {
    let path = format!("/api/conversations/{id}/plugins/browser/state");
    backend.get(&path, Some(master)).await
}

/// The conversation state of a browser that runs no tab: none, and the
/// pinned Chrome for Testing, which the page looks for among the Host's
/// installed artifacts.
fn no_tabs() -> Value {
    let pinned = BrowserRelease::pinned().unwrap();
    json!({ "tabs": [], "browser": { "name": ARTIFACT, "version": pinned.version } })
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
    let listed = tabs(&backend, &master, &id).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    assert_eq!(
        listed.json::<Value>(),
        json!({ "revision": 0, "state": no_tabs() })
    );
    let synced = call(&backend, &master, &id, "sync", json!({})).await;
    assert_eq!(synced.status, StatusCode::OK);
    assert_eq!(synced.json::<Value>(), Value::Null);
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
        ("bind", json!({})),
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
    let offline = tabs(&backend, &master, &id).await;
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
    let listed = tabs(&backend, &master, &id).await;
    assert_eq!(
        listed.refusal(),
        (StatusCode::CONFLICT, ErrorCode::ConversationArchived)
    );
    let navigate = json!({ "tab": ABSENT, "url": "https://example.test/" });
    let navigated = call(&backend, &master, &id, "navigate", navigate).await;
    assert_eq!(
        navigated.refusal(),
        (StatusCode::CONFLICT, ErrorCode::ConversationArchived)
    );
    let elsewhere = uuid::Uuid::new_v4().to_string();
    let unknown = tabs(&backend, &master, &elsewhere).await;
    assert_eq!(
        unknown.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound)
    );
    backend.close().await;
}

#[tokio::test]
async fn a_stopped_cloud_is_not_woken_to_list_sync_or_move_its_tabs() {
    let harness = Harness::new().with_browser_package();
    let (backend, master) = harness.start_set_up().await;
    // A new conversation works on the Cloud, which never started.
    let id = conversation(&backend, &master).await;
    let listed = tabs(&backend, &master, &id).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    assert_eq!(listed.json::<Value>()["state"], no_tabs());
    let synced = call(&backend, &master, &id, "sync", json!({})).await;
    assert_eq!(synced.status, StatusCode::OK);
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

// Several seconds: the Cloud boots, the first listing installs the builtin
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
    // The first listing starts the browser's service, which the Cloud's
    // runner installs first. A listing is no activity, so a lease of the
    // conversation's file gate, which is its work, keeps the Cloud up
    // meanwhile.
    let working = backend
        .file_gate(&master, &id)
        .await
        .enter(Purpose::Demand)
        .await;
    let first = tabs(&backend, &master, &id).await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    let rested = Instant::now();
    drop(working);
    // The page lists the tabs again and again, and the Cloud idles and
    // stops all the same, a window after the last activity. A listing the
    // stop overtakes finds the runner gone.
    let listing = async {
        loop {
            let listed = tabs(&backend, &master, &id).await;
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
async fn a_backend_whose_catalog_serves_no_browser_has_no_tab_list_and_no_tab_methods() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let id = conversation(&backend, &master).await;
    let listed = tabs(&backend, &master, &id).await;
    assert_eq!(
        listed.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::UnknownPlugin)
    );
    let bound = call(&backend, &master, &id, "bind", json!({ "panelTab": "a" })).await;
    assert_eq!(
        bound.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::UnknownPluginMethod)
    );
    backend.close().await;
}

/// The conversation `id`'s summary as the conversation list carries it.
async fn summary(backend: &TestBackend, master: &Session, id: &str) -> ConversationSummary {
    summaries(backend, master)
        .await
        .into_iter()
        .find(|summary| summary.id.as_str() == id)
        .expect("the conversation is listed")
}

/// The revisions the summary carries: its working tree's and its tab list's.
fn revisions(summary: &ConversationSummary) -> (u64, Vec<PluginRevision>) {
    (
        summary.working_tree_revision,
        summary.plugin_revisions.clone(),
    )
}

fn browser_at(revision: u64) -> Vec<PluginRevision> {
    vec![PluginRevision {
        plugin: "browser".into(),
        revision,
    }]
}

// Several seconds: a real device installs the builtin package, and one turn
// runs a shell job.
#[tokio::test]
async fn a_job_that_ends_and_a_tab_method_raise_the_revisions_the_summary_carries() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_browser_package();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    let id = uuid::Uuid::new_v4().to_string();
    create(&backend, &master, &id).await;
    // The paired device's runner lives as long as its handle.
    let (_device, _) = on_device(&harness, &backend, &master, &id).await;
    let fresh = summary(&backend, &master, &id).await;
    assert_eq!(revisions(&fresh), (0, browser_at(0)));

    // A job may have changed the working tree and opened or closed tabs.
    let mut work = Driven::open(&backend, &master, &vendor, &id, &provider, "/work").await;
    work.turn(vec![shell("t1", "true", 30_000), say("done")])
        .await;
    let ended = summary(&backend, &master, &id).await;
    assert_eq!(revisions(&ended), (1, browser_at(1)));
    let listed = tabs(&backend, &master, &id).await;
    assert_eq!(
        listed.json::<Value>(),
        json!({ "revision": 1, "state": no_tabs() })
    );

    // The user's own tab methods change the list too; the working tree is the jobs'.
    let synced = call(&backend, &master, &id, "sync", json!({})).await;
    assert_eq!(synced.status, StatusCode::OK);
    let operated = summary(&backend, &master, &id).await;
    assert_eq!(revisions(&operated), (1, browser_at(2)));
    backend.close().await;
}
