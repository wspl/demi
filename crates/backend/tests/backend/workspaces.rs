//! Workspaces and the sidebar's order (`web-api.md` § Workspaces, devices,
//! and attached hosts, § Sidebar mutations, read state and page
//! synchronization): a workspace is a named pointer at a directory of one
//! of the user's devices, which stays while conversations target it; the
//! sidebar keeps the order the user gave it, within each partition, however
//! much the rows change.

use demi_web_api::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::support::{Harness, Session, TestBackend};

async fn create_workspace(backend: &TestBackend, session: &Session, body: Value) -> String {
    let created = backend.post("/api/workspaces", Some(session), body).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    created.json::<Value>()["workspace"]["id"].as_str().unwrap().to_owned()
}

async fn workspace_names(backend: &TestBackend, session: &Session) -> Vec<String> {
    let listed: Value = backend.get("/api/workspaces", Some(session)).await.json();
    listed["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|workspace| workspace["name"].as_str().unwrap().to_owned())
        .collect()
}

async fn create_conversation(backend: &TestBackend, session: &Session) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let created = backend.post("/api/conversations", Some(session), json!({ "id": id })).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    id
}

async fn conversation_order(backend: &TestBackend, session: &Session) -> Vec<String> {
    let listed: Value = backend.get("/api/conversations", Some(session)).await.json();
    listed["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|conversation| conversation["id"].as_str().unwrap().to_owned())
        .collect()
}

async fn reorder(backend: &TestBackend, session: &Session, kind: &str, id: &str, before: Option<&str>) -> StatusCode {
    let body = json!({ "kind": kind, "id": id, "beforeId": before });
    backend.post("/api/sidebar/reorder", Some(session), body).await.status
}

#[tokio::test]
async fn a_workspace_points_at_a_directory_of_the_users_device_and_stays_while_conversations_target_it() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let laptop = backend.pair(&master, "laptop").await;
    let home = laptop.runner.home_dir().to_str().unwrap().to_owned();
    let device = json!({ "kind": "device", "deviceId": laptop.id(), "path": home, "name": "  notes  " });
    let notes = create_workspace(&backend, &master, device).await;
    let body = json!({ "kind": "device", "deviceId": laptop.id(), "path": format!("{home}/site"), "name": "site" });
    create_workspace(&backend, &master, body).await;
    assert_eq!(workspace_names(&backend, &master).await, ["notes", "site"]);
    // The state carries them, in the same order.
    let state: Value = backend.get("/api/state", Some(&master)).await.json();
    assert_eq!(state["workspaces"][0]["id"], json!(notes));
    assert_eq!(state["workspaces"][0]["path"], json!(home));

    // What a creation names must be there and the user's.
    let refusals = [
        (json!({ "kind": "device", "deviceId": "nothing", "path": home, "name": "x" }), StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound),
        (json!({ "kind": "device", "deviceId": laptop.id(), "path": "relative", "name": "x" }), StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        (json!({ "kind": "device", "deviceId": laptop.id(), "path": home, "name": "   " }), StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        (json!({ "deviceId": laptop.id(), "path": home, "name": "untagged" }), StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
    ];
    for (body, status, code) in refusals {
        let refused = backend.post("/api/workspaces", Some(&master), body.clone()).await;
        assert_eq!(refused.refusal(), (status, code), "{body}");
    }

    let renamed = backend
        .patch(&format!("/api/workspaces/{notes}"), &master, json!({ "name": "journal" }))
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&renamed.body));
    assert_eq!(renamed.json::<Value>()["workspace"]["name"], json!("journal"));
    let missing = backend
        .patch("/api/workspaces/nothing", &master, json!({ "name": "x" }))
        .await;
    assert_eq!(missing.refusal(), (StatusCode::NOT_FOUND, ErrorCode::WorkspaceNotFound));

    // A conversation targets it: neither it nor its device goes.
    let conversation = create_conversation(&backend, &master).await;
    let target = json!({ "target": { "kind": "workspace", "workspaceId": notes } });
    let moved = backend.patch(&format!("/api/conversations/{conversation}"), &master, target).await;
    assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));
    let listed: Value = backend.get("/api/conversations", Some(&master)).await.json();
    assert_eq!(listed["conversations"][0]["cwd"], json!(home));
    let in_use = backend.delete(&format!("/api/workspaces/{notes}"), &master).await;
    assert_eq!(in_use.refusal(), (StatusCode::CONFLICT, ErrorCode::WorkspaceInUse));
    let revoke = backend.delete(&format!("/api/devices/{}", laptop.id()), &master).await;
    assert_eq!(revoke.refusal(), (StatusCode::CONFLICT, ErrorCode::DeviceInUse));

    // Once nothing targets it, it goes, and its files stay.
    let away = json!({ "target": { "kind": "cloud" } });
    let moved = backend.patch(&format!("/api/conversations/{conversation}"), &master, away).await;
    assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));
    assert_eq!(backend.delete(&format!("/api/workspaces/{notes}"), &master).await.status, StatusCode::NO_CONTENT);
    assert_eq!(workspace_names(&backend, &master).await, ["site"]);
    assert!(std::path::Path::new(&home).is_dir());
    let gone = backend.delete(&format!("/api/workspaces/{notes}"), &master).await;
    assert_eq!(gone.refusal(), (StatusCode::NOT_FOUND, ErrorCode::WorkspaceNotFound));
    backend.close().await;
}

#[tokio::test]
async fn the_sidebar_keeps_the_users_order_within_each_partition_through_patches_and_a_restart() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    // New conversations enter at the front.
    let a = create_conversation(&backend, &master).await;
    let b = create_conversation(&backend, &master).await;
    let c = create_conversation(&backend, &master).await;
    assert_eq!(conversation_order(&backend, &master).await, [c.clone(), b.clone(), a.clone()]);
    assert_eq!(reorder(&backend, &master, "conversation", &a, Some(&c)).await, StatusCode::NO_CONTENT);
    assert_eq!(conversation_order(&backend, &master).await, [a.clone(), c.clone(), b.clone()]);
    // A rename changes no position, and null moves a row to the end.
    let renamed = backend
        .patch(&format!("/api/conversations/{a}"), &master, json!({ "title": "kept title" }))
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&renamed.body));
    assert_eq!(reorder(&backend, &master, "conversation", &c, None).await, StatusCode::NO_CONTENT);
    assert_eq!(conversation_order(&backend, &master).await, [a.clone(), b.clone(), c.clone()]);

    // A pinned row is another partition: nothing moves across it.
    let pinned = backend
        .patch(&format!("/api/conversations/{b}"), &master, json!({ "pinned": true }))
        .await;
    assert_eq!(pinned.status, StatusCode::OK, "{}", String::from_utf8_lossy(&pinned.body));
    assert_eq!(conversation_order(&backend, &master).await, [b.clone(), a.clone(), c.clone()]);
    assert_eq!(reorder(&backend, &master, "conversation", &a, Some(&b)).await, StatusCode::CONFLICT);
    // So is a workspace, and an archived row is in none.
    let laptop = backend.pair(&master, "laptop").await;
    let home = laptop.runner.home_dir().to_str().unwrap().to_owned();
    let notes = create_workspace(
        &backend,
        &master,
        json!({ "kind": "device", "deviceId": laptop.id(), "path": home, "name": "notes" }),
    )
    .await;
    let target = json!({ "target": { "kind": "workspace", "workspaceId": notes } });
    let moved = backend.patch(&format!("/api/conversations/{c}"), &master, target).await;
    assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));
    assert_eq!(reorder(&backend, &master, "conversation", &a, Some(&c)).await, StatusCode::CONFLICT);
    let archived = backend
        .patch(&format!("/api/conversations/{a}"), &master, json!({ "archived": true }))
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{}", String::from_utf8_lossy(&archived.body));
    let refused = backend
        .post("/api/sidebar/reorder", Some(&master), json!({ "kind": "conversation", "id": a, "beforeId": null }))
        .await;
    assert_eq!(refused.refusal(), (StatusCode::CONFLICT, ErrorCode::InvalidOrder));
    let unknown = json!({ "kind": "project", "id": a, "beforeId": null });
    let unknown = backend.post("/api/sidebar/reorder", Some(&master), unknown).await;
    assert_eq!(unknown.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));

    // Workspaces keep their order too: a new one appends.
    let site = create_workspace(
        &backend,
        &master,
        json!({ "kind": "device", "deviceId": laptop.id(), "path": format!("{home}/site"), "name": "site" }),
    )
    .await;
    assert_eq!(workspace_names(&backend, &master).await, ["notes", "site"]);
    assert_eq!(reorder(&backend, &master, "workspace", &site, Some(&notes)).await, StatusCode::NO_CONTENT);
    assert_eq!(workspace_names(&backend, &master).await, ["site", "notes"]);

    // The order is a record: it outlives the process.
    let address = backend.address();
    backend.close().await;
    let backend = harness.start_at(address).await;
    let unarchived: Value = backend.get("/api/conversations?archived=false", Some(&master)).await.json();
    let order: Vec<&str> = unarchived["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|conversation| conversation["id"].as_str().unwrap())
        .collect();
    assert_eq!(order, [b.as_str(), c.as_str()]);
    assert_eq!(workspace_names(&backend, &master).await, ["site", "notes"]);
    backend.close().await;
}
