//! Conversation drafts (`web-api.md` § Conversation drafts): the backend
//! keeps one draft per conversation, which every session of its user
//! follows through the draft revision its channel brings. A save always
//! takes effect; one built on an older revision keeps the version it
//! replaced, which a session restores or dismisses. An upload the draft
//! names is readable from every session, with the opening its record keeps.

use demi_web_api_protocol::attachments::AttachmentAnswer;
use demi_web_api_protocol::drafts::{ConversationDraft, DraftAnswer, DraftFile, ReplacedDraft};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::state::SyncEvent;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::{
    Answer, Harness, MASTER_EMAIL, MASTER_PASSWORD, Session, SyncChannel, TestBackend, answer,
};

/// The mark a draft's text holds where a file's capsule stands.
const MARK: char = '\u{FFFC}';

/// Waits until the page's channel brings the conversation's summary with
/// the draft revision `revision`.
async fn until_draft(page: &mut SyncChannel, id: &str, revision: u64) {
    page.until(|event| {
        matches!(event, SyncEvent::Conversation { conversation }
            if conversation.id.as_str() == id && conversation.draft_revision == revision)
    })
    .await;
}

async fn read(backend: &TestBackend, session: &Session, id: &str) -> ConversationDraft {
    let read = backend
        .get(&format!("/api/conversations/{id}/draft"), Some(session))
        .await;
    assert_eq!(
        read.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&read.body)
    );
    read.json::<DraftAnswer>().draft
}

async fn put(
    backend: &TestBackend,
    session: &Session,
    id: &str,
    base: u64,
    text: &str,
    files: &Value,
) -> Answer {
    let body = json!({ "base": base, "text": text, "files": files });
    backend
        .put(&format!("/api/conversations/{id}/draft"), session, body)
        .await
}

async fn save(
    backend: &TestBackend,
    session: &Session,
    id: &str,
    base: u64,
    text: &str,
    files: &Value,
) -> ConversationDraft {
    let saved = put(backend, session, id, base, text, files).await;
    assert_eq!(
        saved.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&saved.body)
    );
    saved.json::<DraftAnswer>().draft
}

async fn act(
    backend: &TestBackend,
    session: &Session,
    id: &str,
    action: &str,
    revision: u64,
) -> Answer {
    let body = json!({ "action": action, "revision": revision });
    backend
        .post(
            &format!("/api/conversations/{id}/draft/replaced"),
            Some(session),
            body,
        )
        .await
}

#[tokio::test]
async fn a_draft_reaches_every_session_and_a_save_built_on_an_older_revision_keeps_what_it_replaced()
 {
    let harness = Harness::new();
    let (backend, laptop) = harness.start_set_up().await;
    let phone = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let id = uuid::Uuid::new_v4().to_string();
    let created = backend
        .post("/api/conversations", Some(&laptop), json!({ "id": id }))
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    assert_eq!(
        read(&backend, &phone, &id).await,
        ConversationDraft::empty()
    );
    let mut phone_page = backend.sync(&phone).await;
    assert_eq!(
        phone_page.snapshot().await.conversations[0].draft_revision,
        0
    );

    // The laptop stages a text file, and a file on a device, and saves the
    // draft that names them.
    let notes = b"\n\n  first line of the notes\r\nsecond line";
    let headers = [("content-type", "text/plain")];
    let uploaded = backend
        .response(
            Method::POST,
            "/api/attachments?name=notes.txt",
            &laptop,
            &headers,
            Some(notes.to_vec().into()),
        )
        .await;
    let upload = answer(uploaded).await.json::<AttachmentAnswer>().attachment;
    let files = json!([
        { "type": "upload", "ref": upload.id, "fileName": "notes.txt" },
        { "type": "remote_file", "deviceId": "a-device", "path": "/var/log/app.log" },
    ]);
    let stored = vec![
        DraftFile::Upload {
            r#ref: upload.id.clone(),
            file_name: "notes.txt".into(),
            media_type: upload.media_type.clone(),
            sha256: upload.sha256.clone(),
            snippet: Some("first line of the notes\nsecond line".into()),
        },
        DraftFile::RemoteFile {
            device_id: "a-device".into(),
            path: "/var/log/app.log".into(),
        },
    ];
    let text = |words: &str| format!("Fix the login{words} {MARK} after {MARK}");
    let first = save(&backend, &laptop, &id, 0, &text(""), &files).await;
    assert_eq!(
        first,
        ConversationDraft {
            revision: 1,
            text: text(""),
            files: stored.clone(),
            replaced: None,
        }
    );

    // The phone learns of it from its channel and reads it: the text, the
    // file with what its record holds, and the file's bytes.
    until_draft(&mut phone_page, &id, 1).await;
    assert_eq!(read(&backend, &phone, &id).await, first);
    let bytes = backend
        .get(
            &format!("/api/blobs/{}", upload.sha256.as_str()),
            Some(&phone),
        )
        .await;
    assert_eq!(bytes.body, notes);

    // Both type on revision 1 at once. The phone's save lands first; the
    // laptop's, built on revision 1 as well, wins and keeps the phone's.
    let test = save(&backend, &phone, &id, 1, &text(" test"), &files).await;
    assert_eq!((test.revision, test.replaced), (2, None));
    let bug = save(&backend, &laptop, &id, 1, &text(" bug"), &files).await;
    let tested = ReplacedDraft {
        revision: 2,
        text: text(" test"),
        files: stored.clone(),
    };
    assert_eq!(
        (bug.revision, bug.text.as_str(), &bug.replaced),
        (3, text(" bug").as_str(), &Some(tested.clone()))
    );
    until_draft(&mut phone_page, &id, 3).await;
    assert_eq!(read(&backend, &phone, &id).await, bug);
    // A save on the current revision keeps the replaced version, and so
    // does one built on an older revision that saves what the draft holds.
    let typed = save(&backend, &laptop, &id, 3, &text(" bug now"), &files).await;
    assert_eq!(
        (typed.revision, &typed.replaced),
        (4, &Some(tested.clone()))
    );
    let again = save(&backend, &phone, &id, 2, &text(" bug now"), &files).await;
    assert_eq!(
        (again.revision, &again.replaced),
        (5, &Some(tested.clone()))
    );
    // That save wrote no new text: one built on the revision before it
    // replaces nothing its page did not show.
    let more = save(&backend, &laptop, &id, 4, &text(" bug now, more"), &files).await;
    assert_eq!((more.revision, &more.replaced), (6, &Some(tested)));

    // A restore exchanges the replaced version with the draft, so nothing
    // is lost; the version it restored is not the replaced one any more.
    let restored = act(&backend, &phone, &id, "restore", 2)
        .await
        .json::<DraftAnswer>()
        .draft;
    let displaced = ReplacedDraft {
        revision: 6,
        text: text(" bug now, more"),
        files: stored.clone(),
    };
    assert_eq!(
        restored,
        ConversationDraft {
            revision: 7,
            text: text(" test"),
            files: stored.clone(),
            replaced: Some(displaced),
        }
    );
    let stale = act(&backend, &laptop, &id, "restore", 2).await;
    assert_eq!(
        stale.refusal(),
        (StatusCode::CONFLICT, ErrorCode::DraftChanged)
    );
    let dismissed = act(&backend, &laptop, &id, "dismiss", 6)
        .await
        .json::<DraftAnswer>()
        .draft;
    assert_eq!(
        (dismissed.revision, dismissed.text, dismissed.replaced),
        (8, text(" test"), None)
    );
    let none = act(&backend, &laptop, &id, "dismiss", 6).await;
    assert_eq!(
        none.refusal(),
        (StatusCode::CONFLICT, ErrorCode::DraftChanged)
    );
    // A dismissal changes no text: a save built on the revision before it
    // replaces nothing its page did not show.
    let restated = save(&backend, &phone, &id, 7, &text(" test more"), &files).await;
    assert_eq!((restated.revision, &restated.replaced), (9, &None));

    // An empty draft that a stale save replaces is nothing to restore.
    let emptied = save(&backend, &laptop, &id, 9, "", &json!([])).await;
    let late = save(&backend, &phone, &id, 8, &text(" late"), &files).await;
    assert_eq!(
        (emptied.revision, late.revision, &late.replaced),
        (10, 11, &None)
    );

    let refusals = [
        (
            json!({ "base": 11, "text": "one mark", "files": files }),
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidBody,
        ),
        (
            json!({ "base": 9, "text": format!("{MARK}"), "files": [{ "type": "text", "text": "not a file" }] }),
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidBody,
        ),
        (
            json!({ "base": 9, "text": format!("{MARK}"), "files": [{ "type": "upload", "ref": "no-such-upload", "fileName": "a.txt" }] }),
            StatusCode::NOT_FOUND,
            ErrorCode::UploadNotFound,
        ),
        (
            json!({ "base": 9, "text": "x".repeat(256 * 1024), "files": [] }),
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::TooLarge,
        ),
    ];
    let path = format!("/api/conversations/{id}/draft");
    for (body, status, code) in refusals {
        assert_eq!(
            backend.put(&path, &laptop, body).await.refusal(),
            (status, code)
        );
    }
    assert_eq!(read(&backend, &laptop, &id).await, late);

    // An archived conversation reads its draft and refuses the rest.
    let archived = backend
        .patch(
            &format!("/api/conversations/{id}"),
            &laptop,
            json!({ "archived": true }),
        )
        .await;
    assert_eq!(
        archived.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&archived.body)
    );
    assert_eq!(read(&backend, &phone, &id).await, late);
    let refused = put(&backend, &phone, &id, 9, "", &json!([])).await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::CONFLICT, ErrorCode::ConversationArchived)
    );
    let refused = act(&backend, &phone, &id, "dismiss", 9).await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::CONFLICT, ErrorCode::ConversationArchived)
    );
    backend.close().await;
}
