//! The work panel (`web-api.md` § Work panel state): a conversation's tabs,
//! which the backend changes a request of changes at a time, all or none,
//! and never interprets, counted by a revision the summary carries, with
//! each tab's id used once.

use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::summaries;
use crate::support::{Harness, Session, TestBackend};

/// A new conversation of `master`'s.
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

/// The panel's routes of one conversation.
struct Panel<'a> {
    backend: &'a TestBackend,
    master: &'a Session,
    id: String,
}

impl Panel<'_> {
    async fn read(&self) -> Value {
        let path = format!("/api/conversations/{}/panel", self.id);
        let read = self.backend.get(&path, Some(self.master)).await;
        assert_eq!(read.status, StatusCode::OK);
        read.json()
    }

    /// The ids of the tabs, in order.
    async fn ids(&self) -> Vec<String> {
        let panel = self.read().await;
        panel["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tab| tab["id"].as_str().unwrap().to_owned())
            .collect()
    }

    /// Sends `changes` as one request.
    async fn send(&self, changes: Value) -> crate::support::Answer {
        let path = format!("/api/conversations/{}/panel/changes", self.id);
        let body = json!({ "changes": changes });
        self.backend.post(&path, Some(self.master), body).await
    }

    async fn create(&self, mut tab: Value) -> crate::support::Answer {
        tab["op"] = json!("create");
        self.send(json!([tab])).await
    }

    async fn update(&self, tab: &str, data: Value) -> crate::support::Answer {
        self.send(json!([{ "op": "update", "id": tab, "data": data }]))
            .await
    }

    async fn remove(&self, tab: &str) -> crate::support::Answer {
        self.send(json!([{ "op": "delete", "id": tab }])).await
    }

    async fn move_to(&self, tab: &str, index: usize) -> crate::support::Answer {
        self.send(json!([{ "op": "move", "id": tab, "index": index }]))
            .await
    }

    async fn summary_revision(&self) -> u64 {
        summaries(self.backend, self.master)
            .await
            .into_iter()
            .find(|summary| summary.id.as_str() == self.id)
            .expect("the conversation is listed")
            .panel_revision
    }
}

/// The revision a change answered.
fn revision(answer: &crate::support::Answer) -> u64 {
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    answer.json::<Value>()["revision"].as_u64().unwrap()
}

/// A tab of the panel fixture's `page` kind, whose plugin does nothing with
/// a tab.
fn page(id: &str, url: &str) -> Value {
    json!({ "id": id, "kind": "page", "data": { "url": url } })
}

#[tokio::test]
async fn a_work_panel_changes_one_request_at_a_time_and_uses_a_tab_id_once() {
    let harness = Harness::new().with_panel_fixture();
    let (backend, master) = harness.start_set_up().await;
    let panel = Panel {
        backend: &backend,
        master: &master,
        id: conversation(&backend, &master).await,
    };
    assert_eq!(panel.read().await, json!({ "revision": 0, "tabs": [] }));
    assert_eq!(panel.summary_revision().await, 0);

    let created = panel.create(page("p1", "https://a.test/")).await;
    assert_eq!(
        created.json::<Value>(),
        json!({ "revision": 1, "changed": true })
    );
    assert_eq!(
        panel.read().await,
        json!({ "revision": 1, "tabs": [page("p1", "https://a.test/")] })
    );
    assert_eq!(panel.summary_revision().await, 1);
    // Creating it again, as a page whose answer was lost does, creates nothing.
    let again = panel.create(page("p1", "https://other.test/")).await;
    assert_eq!(
        again.json::<Value>(),
        json!({ "revision": 1, "changed": false })
    );
    let first =
        json!({ "id": "p2", "kind": "page", "data": { "url": "https://b.test/" }, "index": 0 });
    assert_eq!(revision(&panel.create(first).await), 2);
    assert_eq!(panel.ids().await, ["p2", "p1"]);

    // An update sets the fields it names and removes the null ones.
    assert_eq!(
        revision(&panel.update("p1", json!({ "title": "A" })).await),
        3
    );
    assert_eq!(
        panel.read().await["tabs"][1]["data"],
        json!({ "url": "https://a.test/", "title": "A" })
    );
    assert_eq!(
        revision(&panel.update("p1", json!({ "title": null })).await),
        4
    );
    assert_eq!(
        revision(&panel.update("p1", json!({ "title": null })).await),
        4
    );
    assert_eq!(revision(&panel.move_to("p1", 0).await), 5);
    assert_eq!(revision(&panel.move_to("p1", 0).await), 5);
    assert_eq!(panel.ids().await, ["p1", "p2"]);

    // A removed tab's id is never used again, and its changes do nothing.
    assert_eq!(revision(&panel.remove("p2").await), 6);
    assert_eq!(revision(&panel.remove("p2").await), 6);
    assert_eq!(
        revision(&panel.create(page("p2", "https://b.test/")).await),
        6
    );
    assert_eq!(
        revision(&panel.update("p2", json!({ "url": "x" })).await),
        6
    );
    assert_eq!(panel.ids().await, ["p1"]);
    assert_eq!(panel.summary_revision().await, 6);

    // Several changes are one request and one revision, as Close Others
    // sends them; a request with a change that has nothing to do applies
    // the others.
    for tab in ["p3", "p4", "p5"] {
        panel.create(page(tab, "https://c.test/")).await;
    }
    assert_eq!(panel.ids().await, ["p1", "p3", "p4", "p5"]);
    let others = json!([
        { "op": "delete", "id": "p3" },
        { "op": "delete", "id": "gone" },
        { "op": "delete", "id": "p5" },
    ]);
    assert_eq!(revision(&panel.send(others).await), 10);
    assert_eq!(panel.ids().await, ["p1", "p4"]);
    assert_eq!(panel.summary_revision().await, 10);

    // The backend refuses a kind no plugin declares and a body that does not fit.
    let unknown = json!({ "id": "q", "kind": "a kind nobody declares", "data": {} });
    assert_eq!(
        panel.create(unknown).await.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::UnknownPanelKind)
    );
    for body in [
        json!({ "id": "", "kind": "page", "data": {} }),
        json!({ "id": "x".repeat(65), "kind": "page", "data": {} }),
        json!({ "id": "q", "kind": "page", "data": [1] }),
        json!({ "id": "q", "kind": "page" }),
    ] {
        assert_eq!(
            panel.create(body.clone()).await.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
            "{body}"
        );
    }
    backend.close().await;
}

#[tokio::test]
async fn a_work_panel_holds_64_tabs_and_64_kib_and_an_archived_conversation_takes_no_change() {
    let harness = Harness::new().with_panel_fixture();
    let (backend, master) = harness.start_set_up().await;
    let panel = Panel {
        backend: &backend,
        master: &master,
        id: conversation(&backend, &master).await,
    };
    for index in 0..64 {
        let created = panel
            .create(page(&format!("t{index}"), "https://a.test/"))
            .await;
        assert_eq!(created.status, StatusCode::OK, "{index}");
    }
    assert_eq!(
        panel.create(page("t64", "https://a.test/")).await.refusal(),
        (StatusCode::CONFLICT, ErrorCode::PanelFull)
    );
    let large = "x".repeat(70 * 1024);
    assert_eq!(
        panel
            .update("t0", json!({ "large": large }))
            .await
            .refusal(),
        (StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::TooLarge)
    );
    assert_eq!(panel.remove("t0").await.status, StatusCode::OK);
    let large = json!({ "id": "big", "kind": "page", "data": { "large": large } });
    assert_eq!(
        panel.create(large).await.refusal(),
        (StatusCode::CONFLICT, ErrorCode::PanelFull)
    );
    // A request whose last change is refused applies none of them.
    let before = panel.read().await;
    let refused = json!([
        { "op": "delete", "id": "t1" },
        { "op": "update", "id": "t2", "data": { "large": "x".repeat(70 * 1024) } },
    ]);
    assert_eq!(
        panel.send(refused).await.refusal(),
        (StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::TooLarge)
    );
    assert_eq!(panel.read().await, before);

    let archived = backend
        .patch(
            &format!("/api/conversations/{}", panel.id),
            &master,
            json!({ "archived": true }),
        )
        .await;
    assert_eq!(archived.status, StatusCode::OK);
    assert_eq!(panel.read().await["tabs"].as_array().unwrap().len(), 63);
    let conversation_archived = (StatusCode::CONFLICT, ErrorCode::ConversationArchived);
    assert_eq!(
        panel
            .create(page("late", "https://a.test/"))
            .await
            .refusal(),
        conversation_archived
    );
    assert_eq!(panel.remove("t1").await.refusal(), conversation_archived);

    let missing = uuid::Uuid::new_v4().to_string();
    let path = format!("/api/conversations/{missing}/panel");
    assert_eq!(
        backend.get(&path, Some(&master)).await.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound)
    );
    backend.close().await;
}
