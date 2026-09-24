//! Account administration (`web-api.md` § Account API, `product.md` § User
//! system): the master creates admins and users, an admin creates users
//! only, and a password is reset only down the ranks.

use demi_web_api::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::support::{Answer, Harness, Session, TestBackend};

async fn create(backend: &TestBackend, actor: &Session, email: &str, role: &str) -> Answer {
    let password = format!("{}-pass-1", email.split('@').next().unwrap());
    let body = json!({ "email": email, "password": password, "role": role });
    backend.post("/api/users", Some(actor), body).await
}

fn id(answer: &Answer) -> String {
    assert_eq!(answer.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&answer.body));
    answer.json::<Value>()["user"]["id"].as_str().unwrap().to_owned()
}

async fn reset(backend: &TestBackend, actor: &Session, id: &str, password: &str) -> Answer {
    backend
        .patch(&format!("/api/users/{id}"), actor, json!({ "password": password }))
        .await
}

#[tokio::test]
async fn the_master_creates_admins_and_users_an_admin_users_only_and_nobody_outranks_the_master() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let admin = create(&backend, &master, "alice@example.test", "admin").await;
    let admin_id = id(&admin);
    assert_eq!(admin.json::<Value>()["user"]["role"], json!("admin"));
    let user_id = id(&create(&backend, &master, "bob@example.test", "user").await);
    let taken = create(&backend, &master, "Bob@Example.test", "user").await;
    assert_eq!(taken.refusal(), (StatusCode::CONFLICT, ErrorCode::EmailTaken));
    let short = backend
        .post("/api/users", Some(&master), json!({ "email": "x@example.test", "password": "short", "role": "user" }))
        .await;
    assert_eq!(short.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
    let master_role = backend
        .post("/api/users", Some(&master), json!({ "email": "x@example.test", "password": "long-enough", "role": "master" }))
        .await;
    assert_eq!(master_role.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));

    // Administrators see every account; a user sees nothing of it.
    let alice = backend.login("alice@example.test", "alice-pass-1").await;
    let bob = backend.login("bob@example.test", "bob-pass-1").await;
    let listed: Value = backend.get("/api/users", Some(&alice)).await.json();
    let accounts: Vec<String> = listed["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|user| format!("{}:{}", user["email"].as_str().unwrap(), user["role"].as_str().unwrap()))
        .collect();
    assert_eq!(
        accounts,
        ["master@example.test:master", "alice@example.test:admin", "bob@example.test:user"]
    );
    assert!(listed["users"].as_array().unwrap().iter().all(|user| user.get("passwordHash").is_none()));
    assert_eq!(backend.get("/api/users", Some(&bob)).await.refusal(), (StatusCode::FORBIDDEN, ErrorCode::Forbidden));
    let by_user = create(&backend, &bob, "carol@example.test", "user").await;
    assert_eq!(by_user.refusal(), (StatusCode::FORBIDDEN, ErrorCode::Forbidden));

    // An admin creates users, not admins.
    id(&create(&backend, &alice, "carol@example.test", "user").await);
    let by_admin = create(&backend, &alice, "dave@example.test", "admin").await;
    assert_eq!(by_admin.refusal(), (StatusCode::FORBIDDEN, ErrorCode::Forbidden));

    // Resets go down the ranks, never up or sideways.
    assert_eq!(reset(&backend, &alice, &user_id, "bob-pass-2").await.status, StatusCode::NO_CONTENT);
    assert_eq!(backend.login("bob@example.test", "bob-pass-2").await.user.id.as_str(), user_id);
    let sideways = reset(&backend, &alice, &admin_id, "alice-pass-2").await;
    assert_eq!(sideways.refusal(), (StatusCode::FORBIDDEN, ErrorCode::Forbidden));
    let up = reset(&backend, &alice, master.user.id.as_str(), "master-pass-2").await;
    assert_eq!(up.refusal(), (StatusCode::FORBIDDEN, ErrorCode::Forbidden));
    // Whom the reset names is answered before its body.
    let unchecked = backend
        .patch(&format!("/api/users/{}", master.user.id), &alice, json!({ "password": "short" }))
        .await;
    assert_eq!(unchecked.refusal(), (StatusCode::FORBIDDEN, ErrorCode::Forbidden));
    assert_eq!(reset(&backend, &master, &admin_id, "alice-pass-2").await.status, StatusCode::NO_CONTENT);
    assert_eq!(backend.login("alice@example.test", "alice-pass-2").await.user.role.to_string(), "admin");
    let nobody = reset(&backend, &master, "nobody", "whatever-1").await;
    assert_eq!(nobody.refusal(), (StatusCode::NOT_FOUND, ErrorCode::UserNotFound));
    let short = reset(&backend, &master, &user_id, "short").await;
    assert_eq!(short.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
    backend.close().await;
}
