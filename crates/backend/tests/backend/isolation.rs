//! Tenant isolation (`backend.md` § Authentication and ownership): every
//! route that names a user's object answers another user, an administrator
//! included, as if the object did not exist, and the lists hold nothing of
//! it. A revoked device is gone for good; pairing the machine again is a
//! new device.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::conversations::Socket;
use crate::support::{Session, TestBackend, answer, eventually};

/// A PNG image's first bytes, from which the backend reads its type.
const PNG: [u8; 12] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01];

async fn user(backend: &TestBackend, master: &Session, email: &str) -> Session {
    let password = format!("{}-pass-1", email.split('@').next().unwrap());
    let body = json!({ "email": email, "password": password, "role": "user" });
    let created = backend.post("/api/users", Some(master), body).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    backend.login(email, &password).await
}

async fn upload(backend: &TestBackend, session: &Session, media_type: &str, bytes: &[u8]) -> String {
    let headers = [("content-type", media_type)];
    let sent = backend
        .response(Method::POST, "/api/attachments?name=file", session, &headers, Some(bytes.to_vec().into()))
        .await;
    let uploaded = answer(sent).await;
    assert_eq!(uploaded.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&uploaded.body));
    uploaded.json::<Value>()["attachment"]["sha256"].as_str().unwrap().to_owned()
}

async fn created(backend: &TestBackend, session: &Session, path: &str, body: Value) -> Value {
    let answered = backend.post(path, Some(session), body).await;
    assert_eq!(answered.status, StatusCode::CREATED, "{path}: {}", String::from_utf8_lossy(&answered.body));
    answered.json()
}

/// How many entries each list holds for `session`.
async fn list_lengths(backend: &TestBackend, session: &Session) -> Vec<usize> {
    let mut lengths = Vec::new();
    for (path, field) in [
        ("/api/conversations", "conversations"),
        ("/api/devices", "devices"),
        ("/api/workspaces", "workspaces"),
    ] {
        let listed: Value = backend.get(path, Some(session)).await.json();
        lengths.push(listed[field].as_array().unwrap().len());
    }
    lengths
}

#[tokio::test]
async fn another_users_objects_answer_404_on_every_route_to_users_and_admins_alike_and_a_revoked_device_is_gone() {
    let harness = crate::support::Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alice = user(&backend, &master, "alice@example.test").await;
    let bob = user(&backend, &master, "bob@example.test").await;

    // Alice's world: a device, a workspace on it, a conversation with the
    // device attached, and two uploads.
    let mut laptop = backend.pair(&alice, "alice-laptop").await;
    let device = laptop.id().to_owned();
    let home = laptop.runner.home_dir().to_str().unwrap().to_owned();
    let workspace = created(
        &backend,
        &alice,
        "/api/workspaces",
        json!({ "kind": "device", "deviceId": device, "path": home, "name": "proj" }),
    )
    .await["workspace"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let conversation = uuid::Uuid::new_v4().to_string();
    created(&backend, &alice, "/api/conversations", json!({ "id": conversation })).await;
    created(&backend, &alice, &format!("/api/conversations/{conversation}/hosts"), json!({ "deviceId": device })).await;
    let image = upload(&backend, &alice, "image/png", &PNG).await;
    let private = b"alice private file";
    let private_hash = upload(&backend, &alice, "text/plain", private).await;
    assert_eq!(private_hash, hex::encode(Sha256::digest(private)));
    Socket::connect(&backend, &alice, &conversation).await;
    let bobs = uuid::Uuid::new_v4().to_string();
    created(&backend, &bob, "/api/conversations", json!({ "id": bobs })).await;

    let c = &conversation;
    let made = format!("{home}/made");
    let file = url::form_urlencoded::byte_serialize(home.as_bytes()).collect::<String>();
    let denied: Vec<(Method, String, Option<Value>)> = vec![
        (Method::GET, format!("/api/blobs/{image}"), None),
        (Method::GET, format!("/api/blobs/{private_hash}"), None),
        (Method::GET, format!("/api/conversations/{c}/transcript"), None),
        (Method::PATCH, format!("/api/conversations/{c}"), Some(json!({ "title": "taken" }))),
        (Method::PATCH, format!("/api/conversations/{c}"), Some(json!({ "archived": true }))),
        (Method::GET, format!("/api/conversations/{c}/hosts"), None),
        (Method::GET, format!("/api/conversations/{c}/fs"), None),
        (Method::POST, format!("/api/conversations/{c}/fs"), Some(json!({ "path": made }))),
        (Method::GET, format!("/api/conversations/{c}/fs/file?path={file}"), None),
        (Method::GET, format!("/api/conversations/{c}/fs/raw?path={file}"), None),
        (Method::GET, format!("/api/conversations/{c}/changes"), None),
        (Method::GET, format!("/api/conversations/{c}/panel"), None),
        (Method::PUT, format!("/api/conversations/{c}/panel"), Some(json!({ "selection": "change", "tabs": [] }))),
        (Method::GET, format!("/api/conversations/{c}/browser/tabs"), None),
        (Method::POST, format!("/api/conversations/{c}/activity"), None),
        (Method::GET, format!("/api/conversations/{c}/hosts/{device}/fs"), None),
        (Method::POST, format!("/api/conversations/{c}/hosts/{device}/fs"), Some(json!({ "path": made }))),
        (Method::GET, format!("/api/conversations/{bobs}/hosts/{device}/fs"), None),
        (Method::POST, format!("/api/conversations/{c}/hosts"), Some(json!({ "deviceId": device }))),
        (Method::PATCH, format!("/api/conversations/{c}/hosts/{device}"), Some(json!({ "name": "taken" }))),
        (Method::DELETE, format!("/api/conversations/{c}/hosts/{device}"), None),
        (
            Method::PATCH,
            format!("/api/conversations/{bobs}"),
            Some(json!({ "target": { "kind": "workspace", "workspaceId": workspace } })),
        ),
        (Method::POST, format!("/api/conversations/{bobs}/hosts"), Some(json!({ "deviceId": device }))),
        (Method::DELETE, format!("/api/devices/{device}"), None),
        (Method::GET, format!("/api/devices/{device}/fs?path={file}"), None),
        (Method::POST, format!("/api/devices/{device}/fs"), Some(json!({ "path": made }))),
        (Method::PATCH, format!("/api/workspaces/{workspace}"), Some(json!({ "name": "taken" }))),
        (Method::DELETE, format!("/api/workspaces/{workspace}"), None),
        (
            Method::POST,
            "/api/workspaces".to_owned(),
            Some(json!({ "kind": "device", "deviceId": device, "path": home, "name": "squat" })),
        ),
    ];
    for (actor, lists) in [(&bob, [1, 0, 0]), (&master, [0, 0, 0])] {
        for (method, path, body) in &denied {
            let answered = backend
                .send(method.clone(), path, Some(&actor.cookie), body.clone())
                .await;
            assert_eq!(
                answered.status,
                StatusCode::NOT_FOUND,
                "{} {method} {path}: {}",
                actor.user.email.as_str(),
                String::from_utf8_lossy(&answered.body)
            );
        }
        assert_eq!(Socket::try_connect(&backend, actor, &conversation).await.err(), Some(404));
        assert_eq!(list_lengths(&backend, actor).await, lists, "{}", actor.user.email.as_str());
    }

    // Alice still has everything.
    let hosts: Value = backend.get(&format!("/api/conversations/{c}/hosts"), Some(&alice)).await.json();
    assert_eq!(hosts["hosts"].as_array().unwrap().len(), 1);
    assert_eq!(list_lengths(&backend, &alice).await, [1, 1, 1]);
    assert_eq!(backend.get(&format!("/api/blobs/{image}"), Some(&alice)).await.body, PNG);
    assert_eq!(backend.get(&format!("/api/blobs/{private_hash}"), Some(&alice)).await.body, private);
    // The same bytes are Bob's only once he stores his own copy.
    assert_eq!(upload(&backend, &bob, "image/png", &PNG).await, image);
    assert_eq!(backend.get(&format!("/api/blobs/{image}"), Some(&bob)).await.body, PNG);
    let still = backend.get(&format!("/api/blobs/{private_hash}"), Some(&bob)).await;
    assert_eq!(still.status, StatusCode::NOT_FOUND);

    // A revoke waits for the workspace to go, then the runner is refused
    // for good and the attachment goes with the device.
    let in_use = backend.delete(&format!("/api/devices/{device}"), &alice).await;
    assert_eq!(in_use.status, StatusCode::CONFLICT);
    assert_eq!(backend.delete(&format!("/api/workspaces/{workspace}"), &alice).await.status, StatusCode::NO_CONTENT);
    assert_eq!(backend.delete(&format!("/api/devices/{device}"), &alice).await.status, StatusCode::NO_CONTENT);
    let hosts: Value = backend.get(&format!("/api/conversations/{c}/hosts"), Some(&alice)).await.json();
    assert_eq!(hosts["hosts"], json!([]));
    eventually("the revoked runner stops", || {
        let running = laptop.runner.running();
        async move { !running }
    })
    .await;
    // Pairing the machine again is a new device, here Bob's.
    let again = backend.pair(&bob, "alice-laptop").await;
    assert_ne!(again.id(), device);
    let devices: Value = backend.get("/api/devices", Some(&bob)).await.json();
    let ids: Vec<&str> = devices["devices"].as_array().unwrap().iter().map(|entry| entry["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [again.id()]);
    assert_eq!(list_lengths(&backend, &alice).await, [1, 0, 0]);
    let theirs = backend.delete(&format!("/api/devices/{}", again.id()), &alice).await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND);
    backend.close().await;
}
