//! The conversation browser's tab list and tab methods, read through the
//! conversation state route and called through the plugin call route against
//! the `demi.browser` package the workspace built.
//!
//! The scenarios run in three places:
//!
//! - a paired device's real runner;
//! - a Cloud that never started;
//! - a running Cloud, which listing its tabs does not keep awake
//!   (`crates/backend-user-shard/src/lifecycle/mod.rs`).
//!
//! No browser runs: the operations answer as a browser that does not run does,
//! so no Chrome is needed. Opening a tab starts one, which the browser's own
//! tests and the live view's acceptance cover.

use std::time::Duration;

use demi_shared_gates::Purpose;
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::time::Instant;

use demi_provider_common::testing::MockVendor;
use demi_web_api_protocol::conversations::PluginRevision;

use crate::cloud::{idle_after, the_cloud};
use crate::holding_edge::HoldingEdge;
use crate::conversations::{anthropic_at, create, on_device};
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

/// The conversation state of a browser that runs no tab.
fn no_tabs() -> Value {
    json!({ "tabs": [] })
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

/// A browser whose service does not start on the Host is unavailable, in the
/// browser's own reason, wherever the panel meets it:
///
/// - the tab list;
/// - a tab method;
/// - a panel tab the plugin could not open a browser tab for, whose content
///   says the browser couldn't start on the device.
///
/// Before, the list answered `plugin_failed` and the tab kept a reason the page
/// had no words for, so it said "The browser couldn't do that." On a production
/// server the first listing after an upgrade waited for the new release's
/// browser service, which did not start in time.
///
/// A paired device's real runner, about a second.
#[tokio::test]
async fn a_browser_whose_service_does_not_start_is_unavailable_to_the_list_the_methods_and_a_reopened_tab() {
    let harness = Harness::new().with_browser_package_that_does_not_start();
    let (backend, master) = harness.start_set_up().await;
    let laptop = backend.pair(&master, "laptop").await;
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
    let unavailable = (
        StatusCode::CONFLICT,
        ErrorCode::PluginRefused,
        Some("browser_unavailable".to_owned()),
    );

    let listed = tabs(&backend, &master, &id).await;
    assert_eq!(
        plugin_refusal(&listed),
        unavailable,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    let message = listed.error().message;
    assert!(
        message.contains("the browser service cannot start here"),
        "{message}"
    );

    // A panel tab whose browser tab the browser lost: Retry's `bind` cannot
    // open one, and the tab keeps why.
    let created = backend
        .post(
            &format!("/api/conversations/{id}/panel/changes"),
            Some(&master),
            json!({ "changes": [{ "op": "create", "id": "new", "kind": "browser", "data": { "url": "https://example.test/", "tab": "t9", "closed": true } }] }),
        )
        .await;
    assert!(
        created.status.is_success(),
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    let bound = call(&backend, &master, &id, "bind", json!({ "panelTab": "new" })).await;
    assert_eq!(bound.status, StatusCode::OK);
    assert_eq!(bound.json::<Value>(), json!({ "tab": null }));
    let panel = serde_json::to_value(crate::conversations::metadata(&backend, &master, &id).await.panel).unwrap();
    let failure = panel["tabs"]
        .as_array()
        .and_then(|tabs| tabs.iter().find(|tab| tab["id"] == "new"))
        .map(|tab| tab["data"]["failure"].clone());
    assert_eq!(
        failure.as_ref().map(|failure| &failure["code"]),
        Some(&json!("browser_unavailable")),
        "{panel}"
    );
    backend.close().await;
}

/// A tab call whose connection to the Host breaks partway says so, 409
/// `host_operation_failed`, which the page words as a broken connection to the
/// device, not 500 `plugin_failed`, the plugin's own defect.
///
/// The runner reaches the backend through an edge that drops every pipe WebSocket
/// once cut, while the runner's own connection stays.
///
/// A paired device's real runner, about a second.
#[tokio::test]
async fn a_tab_call_whose_connection_to_the_host_breaks_partway_says_the_connection_broke() {
    let harness = Harness::new().with_browser_package();
    let (backend, master) = harness.start_set_up().await;
    let edge = HoldingEdge::start(backend.address()).await;
    let laptop = backend.pair_through(&master, "laptop", &edge.url).await;
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
    let listed = tabs(&backend, &master, &id).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );

    edge.cut_pipe_sockets();
    let broken = tabs(&backend, &master, &id).await;
    assert_eq!(
        broken.refusal(),
        (StatusCode::CONFLICT, ErrorCode::HostOperationFailed),
        "{}",
        String::from_utf8_lossy(&broken.body)
    );
    assert!(backend.online(&master, laptop.id()).await);
    backend.close().await;
}

#[tokio::test]
async fn a_stopped_cloud_is_not_woken_to_list_or_move_its_tabs() {
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

/// The revisions the conversation `id`'s metadata carries of its plugins'
/// states: the browser's tab list's.
async fn revisions(backend: &TestBackend, master: &Session, id: &str) -> Vec<PluginRevision> {
    crate::conversations::metadata(backend, master, id).await.plugin_revisions
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
async fn a_job_that_touches_no_browser_leaves_the_tab_list_revision_the_summary_carries() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_browser_package();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    let id = uuid::Uuid::new_v4().to_string();
    create(&backend, &master, &id).await;
    // The paired device's runner lives as long as its handle.
    let (_device, _) = on_device(&harness, &backend, &master, &id).await;
    assert_eq!(revisions(&backend, &master, &id).await, browser_at(0));

    // The browser tells its plugin of its own changes; a job that runs no
    // browser command changes nothing the plugin shows, and its end reads
    // nothing (`crates/plugin-interface/src/request.rs`).
    let mut work = Driven::open(&backend, &master, &vendor, &id, &provider, "/work").await;
    work.turn(vec![shell("t1", "true", 30_000), say("done")])
        .await;
    assert_eq!(revisions(&backend, &master, &id).await, browser_at(0));
    backend.close().await;
}

/// While the device's runner is away after a drop, the device shows online,
/// and the work panel's tab list waits for the runner, rather than answering
/// that the device is offline, and is answered once it is back
/// (`crates/backend-runners/src/devices.rs`).
// About two seconds: a real device installs the builtin package, stops and
// starts again.
#[tokio::test]
async fn the_tab_list_waits_for_a_runner_that_dropped() {
    let mut harness = Harness::new().with_browser_package();
    // Longer than the test: the runner comes back first.
    harness.runners.returning = Duration::from_secs(10 * 60);
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let id = conversation(&backend, &master).await;
    let home = laptop.runner.home_dir().to_str().unwrap().to_owned();
    let target = json!({ "target": { "kind": "device", "deviceId": laptop.id(), "path": home } });
    let moved = backend.patch(&format!("/api/conversations/{id}"), &master, target).await;
    assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));
    let listed = tabs(&backend, &master, &id).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&listed.body));

    let before = backend.last_seen(&master, laptop.id()).await;
    harness.clock.advance(jiff::SignedDuration::from_secs(1));
    laptop.runner.stop().await;
    backend.until_seen_since(&master, laptop.id(), before).await;
    assert!(backend.online(&master, laptop.id()).await);

    let listed = {
        let mut listing = std::pin::pin!(tabs(&backend, &master, &id));
        tokio::select! {
            answered = &mut listing => panic!(
                "the tab list answered while the runner was away: {} {}",
                answered.status,
                String::from_utf8_lossy(&answered.body)
            ),
            () = backend.until_waiting_for_return(&master, 1) => {}
        }
        laptop.runner.start_again();
        listing.await
    };
    assert_eq!(listed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&listed.body));
    assert_eq!(listed.json::<Value>()["state"], no_tabs());
    backend.close().await;
}
