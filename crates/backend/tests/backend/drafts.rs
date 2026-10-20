//! Conversation drafts: the backend keeps one draft per conversation, which every
//! session of its user follows through the draft revision its channel brings. A save
//! always takes effect; one built on an older revision keeps the version it replaced,
//! which a session restores or dismisses. An upload the draft names is readable from
//! every session, with the opening its record keeps.

use demi_web_api_protocol::attachments::AttachmentAnswer;
use demi_shared_types::{BlockId, MessageComment};
use demi_web_api_protocol::drafts::{ConversationDraft, DraftAnswer, DraftFile, ReplacedDraft};
use demi_web_api_protocol::saved::{FromDraftAnswer, SavedList};
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
    let body = json!({ "base": base, "text": text, "files": files, "comments": [] });
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
            comments: Vec::new(),
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
        comments: Vec::new(),
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
        comments: Vec::new(),
    };
    assert_eq!(
        restored,
        ConversationDraft {
            revision: 7,
            text: text(" test"),
            files: stored.clone(),
            comments: Vec::new(),
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
            json!({ "base": 11, "text": "one mark", "files": files, "comments": [] }),
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidBody,
        ),
        (
            json!({ "base": 9, "text": format!("{MARK}"), "files": [{ "type": "text", "text": "not a file" }], "comments": [] }),
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidBody,
        ),
        (
            json!({ "base": 9, "text": format!("{MARK}"), "files": [{ "type": "upload", "ref": "no-such-upload", "fileName": "a.txt" }], "comments": [] }),
            StatusCode::NOT_FOUND,
            ErrorCode::UploadNotFound,
        ),
        (
            json!({ "base": 9, "text": "x".repeat(256 * 1024), "files": [], "comments": [] }),
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

/// A comment as a draft holds it, on the reply's text block `text-1`.
fn comment(id: &str, start: u64, quote: &str, text: &str) -> MessageComment {
    MessageComment {
        id: id.into(),
        block_id: BlockId::try_from("text-1").unwrap(),
        start,
        at: start,
        end: start + quote.chars().count() as u64,
        quote: quote.into(),
        picture: None,
        text: text.into(),
    }
}

async fn save_comments(
    backend: &TestBackend,
    session: &Session,
    id: &str,
    base: u64,
    text: &str,
    comments: &[MessageComment],
) -> ConversationDraft {
    let body = json!({ "base": base, "text": text, "files": [], "comments": comments });
    let saved = backend
        .put(&format!("/api/conversations/{id}/draft"), session, body)
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&saved.body));
    saved.json::<DraftAnswer>().draft
}

/// Comments wait in the draft as its text does: every session reads the ones
/// another saved, a save built on an older revision keeps the comments it
/// replaced with their text, and Save for Later takes the text into the list
/// while the comments stay waiting, as Edit on a saved message leaves them too.
#[tokio::test]
async fn waiting_comments_reach_every_session_and_stay_in_the_draft_when_its_text_is_saved_for_later() {
    let harness = Harness::new();
    let (backend, laptop) = harness.start_set_up().await;
    let phone = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let id = uuid::Uuid::new_v4().to_string();
    let created = backend
        .post("/api/conversations", Some(&laptop), json!({ "id": id }))
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let mut phone_page = backend.sync(&phone).await;
    phone_page.snapshot().await;

    let chunks = comment("c1", 120, "Upload in chunks of 4 MB.", "4 MB feels large on 2G.");
    let timeout = comment("c2", 40, "the same 30-second timeout", "Should uploads get their own?");
    let first = save_comments(&backend, &laptop, &id, 0, "", &[chunks.clone()]).await;
    assert_eq!((first.revision, first.comments.clone()), (1, vec![chunks.clone()]));
    until_draft(&mut phone_page, &id, 1).await;
    assert_eq!(read(&backend, &phone, &id).await, first);

    // Both change the comments on revision 1: the laptop's save wins and
    // keeps the phone's version, comments included.
    let both = [chunks.clone(), timeout.clone()];
    let phone_saved = save_comments(&backend, &phone, &id, 1, "", &both).await;
    assert_eq!(phone_saved.comments, both);
    let edited = comment("c1", 120, "Upload in chunks of 4 MB.", "Start at 1 MB.");
    let laptop_saved = save_comments(&backend, &laptop, &id, 1, "Otherwise go ahead", &[edited.clone()]).await;
    assert_eq!(
        laptop_saved.replaced,
        Some(ReplacedDraft { revision: 2, text: String::new(), files: Vec::new(), comments: both.to_vec() })
    );
    until_draft(&mut phone_page, &id, 3).await;
    assert_eq!(read(&backend, &phone, &id).await, laptop_saved);

    // Save for Later takes the text; the comment stays waiting.
    let saved = backend
        .post(
            &format!("/api/conversations/{id}/saved/from-draft"),
            Some(&laptop),
            json!({ "id": uuid::Uuid::new_v4().to_string(), "base": 3, "text": "Otherwise go ahead", "files": [] }),
        )
        .await;
    assert_eq!(saved.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&saved.body));
    let FromDraftAnswer { message, draft, .. } = saved.json();
    assert_eq!((message.text.as_str(), draft.text.as_str()), ("Otherwise go ahead", ""));
    assert_eq!(draft.comments, vec![edited.clone()]);

    // Edit on it brings the text back beside the comment, which no saved
    // message takes.
    let typed = save_comments(&backend, &laptop, &id, draft.revision, "Also check retries", &[edited.clone()]).await;
    let opened = backend
        .post(
            &format!("/api/conversations/{id}/saved/{}/edit", message.id),
            Some(&laptop),
            json!({ "op": uuid::Uuid::new_v4().to_string(), "id": uuid::Uuid::new_v4().to_string() }),
        )
        .await;
    assert_eq!(opened.status, StatusCode::OK, "{}", String::from_utf8_lossy(&opened.body));
    let after = read(&backend, &laptop, &id).await;
    assert_eq!((after.text.as_str(), after.comments.clone()), ("Otherwise go ahead", vec![edited]));
    let list = backend
        .get(&format!("/api/conversations/{id}/saved"), Some(&laptop))
        .await
        .json::<SavedList>();
    assert_eq!(
        list.messages.iter().map(|each| each.text.as_str()).collect::<Vec<_>>(),
        [typed.text.as_str()]
    );
}
