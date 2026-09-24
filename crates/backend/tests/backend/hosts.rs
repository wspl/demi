//! Where a conversation's work runs, changed through the API
//! (`sessions-and-targets.md` § Switch the main target, § Attached hosts;
//! `web-api.md` § Workspaces, devices, and attached hosts): a target switch
//! moves the work and attaches the device it leaves, a switch ends the
//! conversation's open transfers instead of waiting for them, and attached
//! hosts are attached, renamed and detached. The devices are real runners.

use std::path::PathBuf;

use demi_web_api::auth::Role;
use demi_web_api::error::ErrorCode;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::{Answer, Harness, Paired, Session, TestBackend};

const CONVERSATION: &str = "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a01";

/// A signed-in master with a conversation on the Cloud.
async fn conversation(harness: &Harness) -> (TestBackend, Session) {
    let (backend, master) = harness.start_set_up().await;
    let created = backend
        .post("/api/conversations", Some(&master), json!({ "id": CONVERSATION }))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    (backend, master)
}

/// A directory in the device's home.
fn directory(device: &Paired, name: &str) -> PathBuf {
    let path = device.runner.home_dir().join(name);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn target(device: &Paired, path: &std::path::Path) -> Value {
    json!({ "kind": "device", "deviceId": device.id(), "path": path.to_str().unwrap() })
}

async fn switch(backend: &TestBackend, session: &Session, target: Value) -> Answer {
    backend
        .patch(&format!("/api/conversations/{CONVERSATION}"), session, json!({ "target": target }))
        .await
}

async fn hosts(backend: &TestBackend, session: &Session) -> Vec<Value> {
    let listed = backend
        .get(&format!("/api/conversations/{CONVERSATION}/hosts"), Some(session))
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&listed.body));
    listed.json::<Value>()["hosts"].as_array().unwrap().clone()
}

/// The conversation as the list shows it.
async fn summary(backend: &TestBackend, session: &Session) -> Value {
    let listed: Value = backend.get("/api/conversations?archived=false", Some(session)).await.json();
    listed["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|conversation| conversation["id"] == CONVERSATION)
        .unwrap()
        .clone()
}

#[tokio::test]
async fn a_switch_moves_the_work_and_attaches_the_device_it_leaves() {
    let harness = Harness::new();
    let (backend, master) = conversation(&harness).await;
    let laptop = backend.pair(&master, "laptop").await;
    let ci = backend.pair(&master, "ci").await;
    let on_laptop = directory(&laptop, "work");
    let on_ci = directory(&ci, "build");

    let moved = switch(&backend, &master, target(&laptop, &on_laptop)).await;
    assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));
    assert_eq!(moved.json::<Value>()["results"], json!([{ "field": "target", "status": "applied" }]));
    let before = summary(&backend, &master).await;
    assert_eq!(before["cwd"], json!(on_laptop.to_str().unwrap()));
    // The Cloud was never used, so no device is left behind.
    assert!(hosts(&backend, &master).await.is_empty());

    // The files follow the target; what stayed on the laptop stays there.
    std::fs::write(on_laptop.join("report.txt"), "on the laptop").unwrap();
    let moved = switch(&backend, &master, target(&ci, &on_ci)).await;
    assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));
    let after = summary(&backend, &master).await;
    assert_eq!(after["cwd"], json!(on_ci.to_str().unwrap()));
    assert!(after["contextVersion"].as_u64() > before["contextVersion"].as_u64());
    let listing: Value = backend
        .get(&format!("/api/conversations/{CONVERSATION}/fs"), Some(&master))
        .await
        .json();
    assert_eq!(listing["path"], json!(on_ci.to_str().unwrap()));
    let attached = hosts(&backend, &master).await;
    assert_eq!(attached.len(), 1);
    assert_eq!(
        (&attached[0]["deviceId"], &attached[0]["name"], &attached[0]["cwd"], &attached[0]["online"]),
        (&json!(laptop.id()), &json!("laptop"), &json!(on_laptop.to_str().unwrap()), &json!(true))
    );
    let left: Value = backend
        .get(&format!("/api/conversations/{CONVERSATION}/hosts/{}/fs", laptop.id()), Some(&master))
        .await
        .json();
    let names: Vec<&str> = left["entries"].as_array().unwrap().iter().map(|entry| entry["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["report.txt"]);

    // Back to the laptop: it is main alone, and the device left is attached.
    let back = switch(&backend, &master, target(&laptop, &on_laptop)).await;
    assert_eq!(back.status, StatusCode::OK, "{}", String::from_utf8_lossy(&back.body));
    let attached = hosts(&backend, &master).await;
    let devices: Vec<&Value> = attached.iter().map(|host| &host["deviceId"]).collect();
    assert_eq!(devices, [&json!(ci.id())]);
    // The conversation's own target is no change.
    let unchanged = summary(&backend, &master).await;
    let same = switch(&backend, &master, target(&laptop, &on_laptop)).await;
    assert_eq!(same.status, StatusCode::OK);
    assert_eq!(summary(&backend, &master).await["contextVersion"], unchanged["contextVersion"]);

    // What the destination names must be the user's.
    let missing = switch(&backend, &master, json!({ "kind": "workspace", "workspaceId": "nowhere" })).await;
    assert_eq!(missing.refusal(), (StatusCode::NOT_FOUND, ErrorCode::WorkspaceNotFound));
    let unknown = switch(&backend, &master, json!({ "kind": "device", "deviceId": "nothing", "path": "/" })).await;
    assert_eq!(unknown.refusal(), (StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound));
    harness.add_user("user@example.test", "user-pass-1", Role::User);
    let other = backend.login("user@example.test", "user-pass-1").await;
    let theirs = backend.pair(&other, "theirs").await;
    let foreign = switch(&backend, &master, target(&theirs, &directory(&theirs, "x"))).await;
    assert_eq!(foreign.refusal(), (StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound));
    // An archived conversation takes no switch.
    let archived = backend
        .patch(&format!("/api/conversations/{CONVERSATION}"), &master, json!({ "archived": true }))
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{}", String::from_utf8_lossy(&archived.body));
    let refused = switch(&backend, &master, target(&ci, &on_ci)).await;
    assert_eq!(refused.refusal(), (StatusCode::CONFLICT, ErrorCode::ConversationArchived));
    backend.close().await;
}

#[tokio::test]
async fn a_switch_ends_the_open_download_instead_of_waiting_for_it() {
    let harness = Harness::new();
    let (backend, master) = conversation(&harness).await;
    let laptop = backend.pair(&master, "laptop").await;
    let on_laptop = directory(&laptop, "work");
    assert_eq!(switch(&backend, &master, target(&laptop, &on_laptop)).await.status, StatusCode::OK);
    let video: Vec<u8> = (0..64 * 1024 * 1024).map(|index: usize| (index % 251) as u8).collect();
    std::fs::write(on_laptop.join("long.mp4"), &video).unwrap();
    let path = on_laptop.join("long.mp4");
    let route = format!(
        "/api/conversations/{CONVERSATION}/fs/raw?{}",
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("path", path.to_str().unwrap())
            .finish()
    );
    let mut playing = backend.response(Method::GET, &route, &master, &[], None).await;
    assert_eq!(playing.status(), StatusCode::OK);
    assert!(playing.chunk().await.unwrap().is_some());

    // The page reads nothing more; the switch ends the download and goes on.
    let moved = switch(&backend, &master, json!({ "kind": "cloud" })).await;
    assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));
    let ended = loop {
        match playing.chunk().await {
            Ok(Some(_)) => {}
            Ok(None) => break "complete",
            Err(_) => break "cut",
        }
    };
    assert_eq!(ended, "cut");
    backend.close().await;
}

#[tokio::test]
async fn an_attached_host_is_attached_once_named_uniquely_and_detached() {
    let harness = Harness::new();
    let (backend, master) = conversation(&harness).await;
    let laptop = backend.pair(&master, "laptop").await;
    let ci = backend.pair(&master, "ci").await;
    let spare = backend.pair(&master, "ci").await;
    assert_eq!(
        switch(&backend, &master, target(&laptop, &directory(&laptop, "work"))).await.status,
        StatusCode::OK
    );
    let route = format!("/api/conversations/{CONVERSATION}/hosts");
    let attach = |device: &str| backend.post(&route, Some(&master), json!({ "deviceId": device }));

    let attached = attach(ci.id()).await;
    assert_eq!(attached.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&attached.body));
    let first = attached.json::<Value>()["hosts"].clone();
    assert_eq!((&first[0]["name"], &first[0]["cwd"], &first[0]["online"]), (&json!("ci"), &Value::Null, &json!(true)));
    let context = summary(&backend, &master).await["contextVersion"].clone();
    // Attached already, it stays as it is.
    assert_eq!(attach(ci.id()).await.json::<Value>()["hosts"], first);
    assert_eq!(summary(&backend, &master).await["contextVersion"], context);
    assert_eq!(attach(laptop.id()).await.refusal(), (StatusCode::CONFLICT, ErrorCode::HostIsMain));
    assert_eq!(attach("nothing").await.refusal(), (StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound));
    // A second device of the same name gets a free one.
    let both = attach(spare.id()).await.json::<Value>()["hosts"].clone();
    let names: Vec<&Value> = both.as_array().unwrap().iter().map(|host| &host["name"]).collect();
    assert_eq!(names, [&json!("ci"), &json!("ci-2")]);

    let rename = |device: &str, name: &str| {
        let path = format!("{route}/{device}");
        let body = json!({ "name": name });
        let (backend, master) = (&backend, &master);
        async move { backend.patch(&path, master, body).await }
    };
    let renamed = rename(ci.id(), "  builder ").await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&renamed.body));
    assert_eq!(renamed.json::<Value>()["hosts"][0]["name"], json!("builder"));
    assert_eq!(rename(spare.id(), "builder").await.refusal(), (StatusCode::CONFLICT, ErrorCode::NameTaken));
    assert_eq!(rename(laptop.id(), "main").await.refusal(), (StatusCode::NOT_FOUND, ErrorCode::HostNotAttached));
    assert_eq!(rename(ci.id(), "   ").await.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));

    // A detach is a transition; a device that is not attached detaches as
    // nothing.
    let detached = backend.delete(&format!("{route}/{}", ci.id()), &master).await;
    assert_eq!(detached.status, StatusCode::NO_CONTENT);
    let left: Vec<Value> = hosts(&backend, &master).await.iter().map(|host| host["deviceId"].clone()).collect();
    assert_eq!(left, [json!(spare.id())]);
    assert_eq!(backend.delete(&format!("{route}/{}", ci.id()), &master).await.status, StatusCode::NO_CONTENT);
    // A detached device is no Host of the conversation.
    let gone = backend.get(&format!("{route}/{}/fs", ci.id()), Some(&master)).await;
    assert_eq!(gone.refusal(), (StatusCode::NOT_FOUND, ErrorCode::HostNotAttached));
    backend.close().await;
}
