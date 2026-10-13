//! Messages saved for later (`web-api.md` § Messages saved for later,
//! `runtime.md` § Client frames): a save of the composer's message empties
//! the conversation's draft, keeping one its page never showed to restore,
//! an edit exchanges a saved message with the draft in one go, Undo's save
//! leaves the draft alone, a removal and a send take one off the list, and the conversation's summary counts
//! them with the list's revision. A saved message leaves the list once the
//! session takes it, and stays when the session fails its send; it is sent
//! once: a second `send_saved` of it is rejected. No test calls a real model.

use demi_conversation_socket_protocol::{
    ClientFrame, ClientFrameKind, SavedDelivery, ServerFrame, SteerOutcome,
};
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_shared_types::{Block, BlockId, MediaSource, UserContentBlock};
use demi_web_api_protocol::drafts::{ConversationDraft, DraftAnswer, DraftFile};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::saved::{FromDraftAnswer, SavedAnswer, SavedList, SavedMessage};
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::{FIRST, Socket, anthropic, answer, choose, create, send, transcript};
use crate::support::{
    Answer, Harness, MASTER_EMAIL, MASTER_PASSWORD, Session, SyncChannel, TestBackend,
};
use crate::uploads::{png, upload};

/// The mark a message's text holds where a file's capsule stands.
const MARK: char = '\u{FFFC}';

async fn list(backend: &TestBackend, session: &Session) -> SavedList {
    let read = backend
        .get(&format!("/api/conversations/{FIRST}/saved"), Some(session))
        .await;
    assert_eq!(read.status, StatusCode::OK, "{}", String::from_utf8_lossy(&read.body));
    read.json()
}

/// Adds a message for later, as Undo does.
async fn add(backend: &TestBackend, session: &Session, text: &str, files: &Value) -> Answer {
    backend
        .post(
            &format!("/api/conversations/{FIRST}/saved"),
            Some(session),
            json!({ "text": text, "files": files }),
        )
        .await
}

async fn save(backend: &TestBackend, session: &Session, text: &str, files: &Value) -> SavedMessage {
    let saved = add(backend, session, text, files).await;
    assert_eq!(saved.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&saved.body));
    saved.json::<SavedAnswer>().message
}

/// Saves the composer's message, built on the draft's revision `base`.
async fn from_draft(
    backend: &TestBackend,
    session: &Session,
    base: u64,
    text: &str,
    files: &Value,
) -> (SavedMessage, ConversationDraft) {
    let saved = backend
        .post(
            &format!("/api/conversations/{FIRST}/saved/from-draft"),
            Some(session),
            json!({ "base": base, "text": text, "files": files }),
        )
        .await;
    assert_eq!(saved.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&saved.body));
    let FromDraftAnswer { message, draft } = saved.json();
    (message, draft)
}

async fn edit(backend: &TestBackend, session: &Session, message: &str) -> Answer {
    backend
        .post(
            &format!("/api/conversations/{FIRST}/saved/{message}/edit"),
            Some(session),
            json!({}),
        )
        .await
}

async fn put_draft(backend: &TestBackend, session: &Session, base: u64, text: &str, files: &Value) {
    let saved = backend
        .put(
            &format!("/api/conversations/{FIRST}/draft"),
            session,
            json!({ "base": base, "text": text, "files": files }),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&saved.body));
}

async fn draft(backend: &TestBackend, session: &Session) -> ConversationDraft {
    backend
        .get(&format!("/api/conversations/{FIRST}/draft"), Some(session))
        .await
        .json::<DraftAnswer>()
        .draft
}

/// Waits until the page's channel brings the conversation's summary with
/// `saved` messages at the list's revision `revision`.
async fn until_saved(page: &mut SyncChannel, id: &str, saved: u64, revision: u64) {
    page.until(|event| {
        matches!(event, SyncEvent::Conversation { conversation }
            if conversation.id.as_str() == id
                && (conversation.saved, conversation.saved_revision) == (saved, revision))
    })
    .await;
}

/// The texts of a list, newest first.
fn texts(list: &SavedList) -> Vec<&str> {
    list.messages.iter().map(|message| message.text.as_str()).collect()
}

// About 300 ms: two hundred saves on a local backend.
#[tokio::test]
async fn saving_the_composers_message_empties_the_draft_an_edit_exchanges_them_and_undo_leaves_the_draft() {
    let harness = Harness::new();
    let (backend, laptop) = harness.start_set_up().await;
    let phone = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    create(&backend, &laptop, FIRST).await;
    let mut phone_page = backend.sync(&phone).await;
    let first = &phone_page.snapshot().await.conversations[0];
    assert_eq!((first.saved, first.saved_revision), (0, 0));

    // Ana saves the composer's message with a file for later: the draft
    // that held it is empty, in the same answer.
    let shot = upload(&backend, &laptop, "before.png", "image/png", &png(1)).await;
    let files = json!([{ "type": "upload", "ref": shot.id, "fileName": "before.png" }]);
    let compare = format!("Compare {MARK}");
    put_draft(&backend, &laptop, 0, &compare, &files).await;
    let (saved, emptied) = from_draft(&backend, &laptop, 1, &compare, &files).await;
    assert_eq!(
        saved.files,
        vec![DraftFile::Upload {
            r#ref: shot.id.clone(),
            file_name: "before.png".into(),
            media_type: "image/png".into(),
            sha256: shot.sha256.clone(),
            snippet: None,
        }]
    );
    assert_eq!((emptied.revision, emptied.text.as_str(), emptied.files.len(), &emptied.replaced), (2, "", 0, &None));
    assert_eq!(draft(&backend, &phone).await, emptied);
    until_saved(&mut phone_page, FIRST, 1, 1).await;

    // The phone wrote a draft the laptop never showed; the laptop's save,
    // built on the revision before, keeps it to restore.
    put_draft(&backend, &phone, 2, "From the phone", &json!([])).await;
    let (dns, emptied) = from_draft(&backend, &laptop, 2, "Check prod’s DNS next", &json!([])).await;
    let replaced = emptied.replaced.expect("the phone's draft is kept");
    assert_eq!((emptied.revision, emptied.text.as_str(), replaced.text.as_str()), (4, "", "From the phone"));
    until_saved(&mut phone_page, FIRST, 2, 2).await;
    let read = list(&backend, &laptop).await;
    assert_eq!((read.revision, texts(&read)), (2, vec!["Check prod’s DNS next", compare.as_str()]));

    // With a half-written message in the composer, Edit puts the saved one
    // in the draft and saves the half-written one, in one go.
    put_draft(&backend, &laptop, 4, "Then the IPv6", &json!([])).await;
    let edited = edit(&backend, &laptop, &saved.id).await;
    assert_eq!(edited.status, StatusCode::OK, "{}", String::from_utf8_lossy(&edited.body));
    let edited = edited.json::<DraftAnswer>().draft;
    assert_eq!((edited.revision, edited.text.as_str(), &edited.files), (6, compare.as_str(), &saved.files));
    assert_eq!(draft(&backend, &laptop).await, edited);
    until_saved(&mut phone_page, FIRST, 2, 3).await;
    assert_eq!(texts(&list(&backend, &laptop).await), vec!["Then the IPv6", "Check prod’s DNS next"]);
    // An empty composer saves nothing in exchange.
    put_draft(&backend, &laptop, 6, "", &json!([])).await;
    let edited = edit(&backend, &laptop, &dns.id).await.json::<DraftAnswer>().draft;
    assert_eq!(edited.text, "Check prod’s DNS next");
    let read = list(&backend, &laptop).await;
    assert_eq!(texts(&read), vec!["Then the IPv6"]);

    // Delete removes one, and the id is gone for every route then; Undo
    // saves it again and leaves the draft the composer holds as it is.
    let ipv6 = read.messages[0].clone();
    let path = format!("/api/conversations/{FIRST}/saved/{}", ipv6.id);
    assert_eq!(backend.delete(&path, &laptop).await.status, StatusCode::NO_CONTENT);
    until_saved(&mut phone_page, FIRST, 0, 5).await;
    assert_eq!(backend.delete(&path, &laptop).await.refusal(), (StatusCode::NOT_FOUND, ErrorCode::SavedNotFound));
    assert_eq!(edit(&backend, &laptop, &ipv6.id).await.refusal(), (StatusCode::NOT_FOUND, ErrorCode::SavedNotFound));
    let typed = draft(&backend, &laptop).await;
    let undone = add(&backend, &laptop, &ipv6.text, &json!([])).await;
    assert_eq!(undone.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&undone.body));
    assert_eq!(draft(&backend, &laptop).await, typed);
    until_saved(&mut phone_page, FIRST, 1, 6).await;
    assert_eq!(
        add(&backend, &laptop, "one mark", &files).await.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)
    );

    // A conversation keeps two hundred.
    for index in 1..200 {
        let added = add(&backend, &laptop, &format!("Note {index}"), &json!([])).await;
        assert_eq!(added.status, StatusCode::CREATED);
    }
    assert_eq!(
        add(&backend, &laptop, "One more", &json!([])).await.refusal(),
        (StatusCode::CONFLICT, ErrorCode::SavedFull)
    );
    let full = backend
        .post(
            &format!("/api/conversations/{FIRST}/saved/from-draft"),
            Some(&laptop),
            json!({ "base": 9, "text": "One more", "files": [] }),
        )
        .await;
    assert_eq!(full.refusal(), (StatusCode::CONFLICT, ErrorCode::SavedFull));
    until_saved(&mut phone_page, FIRST, 200, 205).await;
    backend.close().await;
}

// About a second: two turns against a scripted endpoint.
#[tokio::test]
async fn a_saved_message_is_sent_once_as_the_composer_chose_and_leaves_the_list() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let shot = upload(&backend, &master, "shot.png", "image/png", &png(2)).await;
    let files = json!([{ "type": "upload", "ref": shot.id, "fileName": "shot.png" }]);
    let look = save(&backend, &master, &format!("Look at {MARK}"), &files).await;
    let steer = save(&backend, &master, "Also check the IPv6 records", &json!([])).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // A send the session fails, here for a file on a device that is gone,
    // leaves the message on the list.
    let lost = save(
        &backend,
        &master,
        &format!("Read {MARK}"),
        &json!([{ "type": "remote_file", "deviceId": "a-gone-device", "path": "/var/log/app.log" }]),
    )
    .await;
    let send_saved = |id: &str, delivery| ClientFrame::SendSaved { id: id.to_owned(), delivery };
    socket.send(&send_saved(&lost.id, SavedDelivery::Message)).await;
    socket
        .until(|frame| matches!(frame, ServerFrame::Error { .. }))
        .await;
    let read = list(&backend, &master).await;
    assert_eq!((read.revision, read.messages.len()), (3, 3));
    let path = format!("/api/conversations/{FIRST}/saved/{}", lost.id);
    assert_eq!(backend.delete(&path, &master).await.status, StatusCode::NO_CONTENT);

    // Sent as the next message, it starts a turn with its id, its file
    // where its mark stood, and leaves the list.
    vendor.respond(answer(&["Seen."], 1, 1));
    socket.send(&send_saved(&look.id, SavedDelivery::Message)).await;
    socket.until_idle().await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let user = blocks
        .iter()
        .find_map(|block| match block {
            Block::User(user) => Some(user),
            _ => None,
        })
        .expect("the message is in the transcript");
    assert_eq!(user.turn_id.as_str(), look.id);
    assert!(matches!(
        user.content.as_slice(),
        [UserContentBlock::Text { text }, UserContentBlock::Image { source: MediaSource::Ref { .. } }, ..]
            if text == "Look at "
    ), "{:?}", user.content);
    assert_eq!(texts(&list(&backend, &master).await), vec!["Also check the IPv6 records"]);

    // While a turn runs, one goes as a steer under its own id; sent again,
    // as another page might, it is gone and goes nowhere.
    vendor.respond(MockResponse::event_stream(": thinking\n\n").stay_open());
    socket.send(&send("m2", "Hold on")).await;
    vendor.received(2).await;
    socket.send(&send_saved(&steer.id, SavedDelivery::Steer)).await;
    let accepted = socket
        .until(|frame| matches!(frame, ServerFrame::SteerResult { .. }))
        .await;
    assert_eq!(
        accepted.last(),
        Some(&ServerFrame::SteerResult {
            steer_id: BlockId::try_from(steer.id.clone()).unwrap(),
            outcome: SteerOutcome::Accepted,
        })
    );
    socket.send(&send_saved(&steer.id, SavedDelivery::Steer)).await;
    let rejected = socket
        .until(|frame| matches!(frame, ServerFrame::Rejected { .. }))
        .await;
    assert_eq!(
        rejected.last(),
        Some(&ServerFrame::Rejected {
            command: ClientFrameKind::SendSaved,
            reason: ErrorCode::SavedNotFound.to_string(),
        })
    );
    assert!(
        !rejected.iter().any(|frame| matches!(frame, ServerFrame::SteerResult { .. })),
        "{rejected:?}"
    );
    let read = list(&backend, &master).await;
    assert_eq!((read.revision, read.messages.len()), (6, 0));
    socket.stop().await;
    backend.close().await;
}
