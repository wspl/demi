//! The work panel (`web-api.md` § Work panel state): one document per
//! conversation, saved whole, read back as saved and never interpreted.

use demi_web_api::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::support::Harness;

#[tokio::test]
async fn a_work_panel_is_saved_whole_read_back_as_saved_and_never_interpreted() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let id = uuid::Uuid::new_v4().to_string();
    let created = backend.post("/api/conversations", Some(&master), json!({ "id": id })).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let path = format!("/api/conversations/{id}/panel");
    let read = || async { backend.get(&path, Some(&master)).await.json::<Value>() };
    // A conversation that never saved has the empty panel.
    assert_eq!(read().await, json!({ "selection": "change", "tabs": [] }));

    let panel = json!({
        "selection": "tab-1",
        "tabs": [
            { "id": "tab-1", "kind": "browser", "data": { "url": "about:blank" } },
            { "id": "tab-2", "kind": "a kind the backend never heard of", "data": [1, { "nested": true }] },
        ],
    });
    assert_eq!(backend.put(&path, &master, panel.clone()).await.status, StatusCode::NO_CONTENT);
    assert_eq!(read().await, panel);
    // The last save wins.
    let emptied = json!({ "selection": "file", "tabs": [] });
    assert_eq!(backend.put(&path, &master, emptied.clone()).await.status, StatusCode::NO_CONTENT);
    assert_eq!(read().await, emptied);

    let tabs = |count: usize| {
        let tabs: Vec<Value> = (0..count).map(|index| json!({ "id": format!("t{index}"), "kind": "page", "data": null })).collect();
        json!({ "selection": "change", "tabs": tabs })
    };
    let refusals = [
        (json!({ "selection": "change" }), StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        (
            json!({ "selection": "change", "tabs": [{ "id": "a", "kind": "page", "data": null, "status": "loading" }] }),
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidBody,
        ),
        (tabs(65), StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        (
            json!({ "selection": "change", "tabs": [{ "id": "a", "kind": "page", "data": "x".repeat(70_000) }] }),
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::TooLarge,
        ),
    ];
    for (body, status, code) in refusals {
        assert_eq!(backend.put(&path, &master, body).await.refusal(), (status, code));
    }
    assert_eq!(backend.put(&path, &master, tabs(64)).await.status, StatusCode::NO_CONTENT);
    assert_eq!(read().await, tabs(64));

    let unknown = backend
        .get(&format!("/api/conversations/{}/panel", uuid::Uuid::new_v4()), Some(&master))
        .await;
    assert_eq!(unknown.refusal(), (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound));
    // An archived conversation reads its panel and refuses a save.
    let archived = backend
        .patch(&format!("/api/conversations/{id}"), &master, json!({ "archived": true }))
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{}", String::from_utf8_lossy(&archived.body));
    assert_eq!(read().await, tabs(64));
    let refused = backend.put(&path, &master, panel).await;
    assert_eq!(refused.refusal(), (StatusCode::CONFLICT, ErrorCode::ConversationArchived));
    backend.close().await;
}
