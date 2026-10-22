//! The work panel: a conversation's tabs.
//!
//! - The backend changes a request of changes at a time, all or none, and
//!   never interprets them.
//! - The conversation's entry in the list carries the tabs under the revision
//!   that counts the changes, and every page hears of each change.
//! - The conversation's metadata carries the states plugins hold for them.
//! - Each tab's id is used once.
//! - Turning a plugin off closes its kinds' tabs in every panel, an archived
//!   conversation's included.

use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::{Value, json};

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
    /// The panel as the conversation's entry in the list carries it.
    async fn read(&self) -> Value {
        let detail: Value = self
            .backend
            .get(&format!("/api/conversations/{}", self.id), Some(self.master))
            .await
            .json();
        detail["conversation"]["panel"].clone()
    }

    /// The states plugins hold for the panel, as the conversation's metadata carries them.
    async fn states(&self) -> Value {
        let detail: Value = self
            .backend
            .get(&format!("/api/conversations/{}", self.id), Some(self.master))
            .await
            .json();
        detail["metadata"]["panelStates"].clone()
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
        self.read().await["revision"].as_u64().unwrap()
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
    let mut channel = backend.sync(&master).await;
    channel.snapshot().await;

    let created = panel.create(page("p1", "https://a.test/")).await;
    assert_eq!(
        created.json::<Value>(),
        json!({ "revision": 1, "changed": true })
    );
    // Every page hears of the change with the entry, which carries the tabs.
    let heard = channel
        .until(|event| matches!(event, SyncEvent::Conversation { conversation } if conversation.id.as_str() == panel.id && conversation.panel.revision == 1))
        .await;
    let Some(SyncEvent::Conversation { conversation }) = heard.last() else { unreachable!() };
    assert_eq!(serde_json::to_value(&conversation.panel).unwrap(), json!({ "revision": 1, "tabs": [page("p1", "https://a.test/")] }));
    drop(channel);
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
    let path = format!("/api/conversations/{missing}/panel/changes");
    assert_eq!(
        backend
            .post(&path, Some(&master), json!({ "changes": [{ "op": "delete", "id": "t1" }] }))
            .await
            .refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound)
    );
    backend.close().await;
}

// An attachment block's card opens a tab of the file-browser plugin's `attachment`
// kind, which the backend keeps as any tab and lets go of when the user closes it.
//
// Planted defect this catches: the kind left undeclared, which refuses the tab, so
// a card's click opens nothing. One backend, a few requests.
#[tokio::test]
async fn an_attachments_tab_is_kept_until_the_user_closes_it() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let panel = Panel {
        backend: &backend,
        master: &master,
        id: conversation(&backend, &master).await,
    };
    let tab = json!({
        "id": "t1",
        "kind": "attachment",
        "data": { "id": "a3", "name": "report.pdf", "size": 2516582, "url": "/api/blobs/3?type=application%2Fpdf" },
    });
    let created = panel.create(tab.clone()).await;
    assert_eq!(created.json::<Value>(), json!({ "revision": 1, "changed": true }));
    assert_eq!(panel.read().await, json!({ "revision": 1, "tabs": [tab] }));
    assert_eq!(revision(&panel.remove("t1").await), 2);
    assert_eq!(panel.ids().await, Vec::<String>::new());
    backend.close().await;
}

// A picture or a video of the conversation opens the work panel's Media tab, of the
// file-browser plugin's `media` kind, which the backend keeps with the place of the
// medium it shows, so a reload shows it again, and lets go of when the user closes
// it.
//
// Planted defect this catches: the kind left undeclared, which refuses the tab, so
// a picture's click opens nothing. One backend, a few requests.
#[tokio::test]
async fn a_media_tab_is_kept_with_its_place_until_the_user_closes_it() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let panel = Panel {
        backend: &backend,
        master: &master,
        id: conversation(&backend, &master).await,
    };
    let tab = json!({
        "id": "t1",
        "kind": "media",
        "data": { "key": "/api/blobs/5?type=image%2Fpng", "row": "b7" },
    });
    let created = panel.create(tab.clone()).await;
    assert_eq!(created.json::<Value>(), json!({ "revision": 1, "changed": true }));
    assert_eq!(panel.read().await, json!({ "revision": 1, "tabs": [tab] }));
    assert_eq!(revision(&panel.remove("t1").await), 2);
    assert_eq!(panel.ids().await, Vec::<String>::new());
    backend.close().await;
}

// Planted defect this catches: the switch leaving the tabs of a plugin turned
// off, which the panel shows as "This tab cannot be shown", or leaving them in
// an archived conversation, which shows them so once it is unarchived. One
// backend, a few requests.
#[tokio::test]
async fn turning_a_plugin_off_closes_its_tabs_and_leaves_the_others() {
    let harness = Harness::new().with_panel_fixture();
    let (backend, master) = harness.start_set_up().await;
    let panel = Panel {
        backend: &backend,
        master: &master,
        id: conversation(&backend, &master).await,
    };
    let attachment = json!({
        "id": "t2",
        "kind": "attachment",
        "data": { "id": "a3", "name": "report.pdf", "size": 2516582, "url": "/api/blobs/3?type=application%2Fpdf" },
    });
    revision(&panel.create(page("t1", "https://a.example.test/")).await);
    revision(&panel.create(attachment).await);
    let archived = Panel {
        backend: &backend,
        master: &master,
        id: conversation(&backend, &master).await,
    };
    revision(&archived.create(page("t1", "https://b.example.test/")).await);
    let path = format!("/api/conversations/{}", archived.id);
    let archive = |value: bool| backend.patch(&path, &master, json!({ "archived": value }));
    assert_eq!(archive(true).await.status, StatusCode::OK);

    let switched = backend
        .put("/api/plugins/fixture-panel", &master, json!({ "enabled": false }))
        .await;
    assert_eq!(switched.status, StatusCode::NO_CONTENT);
    assert_eq!(panel.ids().await, vec!["t2".to_owned()]);
    assert_eq!(panel.summary_revision().await, 3);
    assert_eq!(archive(false).await.status, StatusCode::OK);
    assert_eq!(archived.ids().await, Vec::<String>::new());
    backend.close().await;
}

/// The conversation's metadata carries the conversation states plugins hold, with
/// the revision each was held at, and leaves out one that holds none, so a kind's
/// content has what it shows without a read of its own.
#[tokio::test]
async fn the_metadata_carries_the_conversation_states_that_plugins_hold() {
    let harness = Harness::new().with_panel_fixture();
    let (backend, master) = harness.start_set_up().await;
    let panel = Panel {
        backend: &backend,
        master: &master,
        id: conversation(&backend, &master).await,
    };
    assert_eq!(panel.states().await, json!([]));

    let held = backend
        .post(
            &format!("/api/conversations/{}/plugins/fixture-panel/calls/hold", panel.id),
            Some(&master),
            json!({ "tabs": 2 }),
        )
        .await;
    assert_eq!(held.status, StatusCode::OK, "{}", String::from_utf8_lossy(&held.body));
    let states = panel.states().await;
    let revision = states[0]["revision"].as_u64().unwrap();
    assert_eq!(
        states,
        json!([{ "plugin": "fixture-panel", "revision": revision, "state": { "tabs": 2 } }])
    );
    assert!(revision >= 1);
    backend.close().await;
}

// Planted defect this catches: a panel document that does not decode failing the
// whole list, or the entry of its conversation, which then no page can list. One
// backend, a few requests.
#[tokio::test]
async fn a_panel_that_cannot_be_read_costs_only_its_conversations_tabs() {
    let harness = Harness::new().with_panel_fixture();
    let (backend, master) = harness.start_set_up().await;
    let whole = Panel { backend: &backend, master: &master, id: conversation(&backend, &master).await };
    let broken = Panel { backend: &backend, master: &master, id: conversation(&backend, &master).await };
    revision(&whole.create(page("p1", "https://a.test/")).await);
    revision(&broken.create(page("p1", "https://b.test/")).await);
    let control = rusqlite::Connection::open(harness.data_dir().join("control.sqlite")).unwrap();
    control
        .execute(
            "UPDATE conversation_panels SET document = '{\"tabs\": 5}' WHERE conversation_id = ?1",
            [&broken.id],
        )
        .unwrap();
    drop(control);

    let listed = crate::conversations::summaries(&backend, &master).await;
    let tabs = |id: &str| {
        listed
            .iter()
            .find(|summary| summary.id.as_str() == id)
            .map(|summary| summary.panel.tabs.len())
    };
    assert_eq!((tabs(&whole.id), tabs(&broken.id)), (Some(1), Some(0)));
    let mut channel = backend.sync(&master).await;
    let state = channel.snapshot().await;
    assert_eq!(state.conversations.len(), 2);
    backend.close().await;
}
