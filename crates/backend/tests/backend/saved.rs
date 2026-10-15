//! Messages saved for later (`web-api.md` § Messages saved for later,
//! `runtime.md` § Client frames): a save of the composer's message empties
//! the conversation's draft, keeping one its page never showed to restore,
//! an edit exchanges a saved message with the draft in one go, Undo's save
//! leaves the draft alone, a removal and a send take one off the list, and the conversation's summary counts
//! them with the list's revision. Every write answers the revisions after it,
//! and a write sent again answers as it first did: a save by its id, an edit
//! by its operation id, a removal of a message gone already. A saved message
//! leaves the list once the session takes it as the turn the page named, and
//! stays when the session fails its send; a send of it with that turn again
//! answers as the first did, and one with another turn is rejected. No test
//! calls a real model.

use demi_conversation_socket_protocol::{
    ClientFrame, ClientFrameKind, SavedDelivery, ServerFrame, SteerOutcome,
};
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_shared_types::{Block, BlockId, MediaSource, TurnId, UserContentBlock};
use demi_web_api_protocol::drafts::{ConversationDraft, DraftAnswer, DraftFile, DraftWritten, Revisions};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::saved::{FromDraftAnswer, SavedAnswer, SavedList, SavedMessage, SavedRemoved};
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::{FIRST, SECOND, Socket, anthropic, answer, choose, create, send, transcript};
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

/// The id a page gives a message it saves.
fn fresh() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Adds a message for later under the page's `id`, as Undo does.
async fn add_as(backend: &TestBackend, session: &Session, id: &str, text: &str, files: &Value) -> Answer {
    backend
        .post(
            &format!("/api/conversations/{FIRST}/saved"),
            Some(session),
            json!({ "id": id, "text": text, "files": files }),
        )
        .await
}

/// Adds a message for later under an id of its own.
async fn add(backend: &TestBackend, session: &Session, text: &str, files: &Value) -> Answer {
    add_as(backend, session, &fresh(), text, files).await
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
            json!({ "id": fresh(), "base": base, "text": text, "files": files }),
        )
        .await;
    assert_eq!(saved.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&saved.body));
    let FromDraftAnswer { message, draft, revisions } = saved.json();
    assert_eq!(revisions.draft, draft.revision);
    (message, draft)
}

/// The edit `op` of the saved message `message`, the draft becoming the
/// message `becomes`.
async fn edit_as(backend: &TestBackend, session: &Session, op: &str, message: &str, becomes: &str) -> Answer {
    backend
        .post(
            &format!("/api/conversations/{FIRST}/saved/{message}/edit"),
            Some(session),
            json!({ "op": op, "id": becomes }),
        )
        .await
}

async fn edit(backend: &TestBackend, session: &Session, message: &str) -> Answer {
    edit_as(backend, session, &fresh(), message, &fresh()).await
}

/// Deletes the saved message `message`; answers the revisions after it.
async fn delete(backend: &TestBackend, session: &Session, message: &str) -> Revisions {
    let path = format!("/api/conversations/{FIRST}/saved/{message}");
    let removed = backend.delete(&path, session).await;
    assert_eq!(removed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&removed.body));
    removed.json::<SavedRemoved>().revisions
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
    let edited = edited.json::<DraftWritten>().draft;
    assert_eq!((edited.revision, edited.text.as_str(), &edited.files), (6, compare.as_str(), &saved.files));
    assert_eq!(draft(&backend, &laptop).await, edited);
    until_saved(&mut phone_page, FIRST, 2, 3).await;
    assert_eq!(texts(&list(&backend, &laptop).await), vec!["Then the IPv6", "Check prod’s DNS next"]);
    // An empty composer saves nothing in exchange.
    put_draft(&backend, &laptop, 6, "", &json!([])).await;
    let edited = edit(&backend, &laptop, &dns.id).await.json::<DraftWritten>().draft;
    assert_eq!(edited.text, "Check prod’s DNS next");
    let read = list(&backend, &laptop).await;
    assert_eq!(texts(&read), vec!["Then the IPv6"]);

    // Delete removes one and answers the revisions; deleted again, as a
    // stale page does, it answers the same. Undo saves it again under a new
    // id and leaves the draft the composer holds as it is.
    let ipv6 = read.messages[0].clone();
    let removed = delete(&backend, &laptop, &ipv6.id).await;
    assert_eq!(removed.saved, 5);
    until_saved(&mut phone_page, FIRST, 0, 5).await;
    assert_eq!(delete(&backend, &laptop, &ipv6.id).await, removed);
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
            json!({ "id": fresh(), "base": 9, "text": "One more", "files": [] }),
        )
        .await;
    assert_eq!(full.refusal(), (StatusCode::CONFLICT, ErrorCode::SavedFull));
    // The page words the refusal from its code and the limit it names.
    assert_eq!(full.error().params["limit"], json!(200));
    until_saved(&mut phone_page, FIRST, 200, 205).await;
    backend.close().await;
}

// About 50 ms: a few saves on a local backend.
#[tokio::test]
async fn a_save_sent_again_with_its_id_makes_one_message_and_another_content_or_conversation_is_refused() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    create(&backend, &master, FIRST).await;
    let id = fresh();

    // The page's save did not reach it, as far as the page knows: it sends
    // the same save again, and the list holds one message either way.
    let first = add_as(&backend, &master, &id, "Check the DNS", &json!([])).await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&first.body));
    let again = add_as(&backend, &master, &id, "Check the DNS", &json!([])).await;
    assert_eq!(again.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&again.body));
    assert_eq!(again.json::<SavedAnswer>(), first.json::<SavedAnswer>());
    let read = list(&backend, &master).await;
    assert_eq!((read.revision, read.messages.len(), read.messages[0].id.as_str()), (1, 1, id.as_str()));

    // The same id for another text is another save, which the id forbids.
    assert_eq!(
        add_as(&backend, &master, &id, "Something else", &json!([])).await.refusal(),
        (StatusCode::CONFLICT, ErrorCode::IdTaken)
    );
    // So is a conversation's save of an id another conversation keeps.
    create(&backend, &master, SECOND).await;
    let elsewhere = backend
        .post(
            &format!("/api/conversations/{SECOND}/saved"),
            Some(&master),
            json!({ "id": id, "text": "Check the DNS", "files": [] }),
        )
        .await;
    assert_eq!(elsewhere.refusal(), (StatusCode::CONFLICT, ErrorCode::IdTaken));

    // The composer's message saved twice: one message, and the second answer
    // carries the draft as it is.
    put_draft(&backend, &master, 0, "Now typed", &json!([])).await;
    let from = fresh();
    let body = json!({ "id": from, "base": 1, "text": "Now typed", "files": [] });
    let path = format!("/api/conversations/{FIRST}/saved/from-draft");
    let saved = backend.post(&path, Some(&master), body.clone()).await;
    assert_eq!(saved.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&saved.body));
    let saved = saved.json::<FromDraftAnswer>();
    let again = backend.post(&path, Some(&master), body).await;
    assert_eq!(again.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&again.body));
    let again = again.json::<FromDraftAnswer>();
    assert_eq!((again.message.clone(), again.draft), (saved.message, saved.draft));
    assert_eq!(list(&backend, &master).await.messages.len(), 2);

    // An id that is no UUID is no id.
    let named = backend
        .post(
            &format!("/api/conversations/{FIRST}/saved"),
            Some(&master),
            json!({ "id": "saved-1", "text": "x", "files": [] }),
        )
        .await;
    assert_eq!(named.status, StatusCode::BAD_REQUEST);
    backend.close().await;
}

// About 50 ms: a few edits on a local backend.
#[tokio::test]
async fn an_edit_sent_again_answers_as_it_first_did_whatever_happened_since() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    create(&backend, &master, FIRST).await;
    let dns = save(&backend, &master, "Check the DNS", &json!([])).await;
    let ipv6 = save(&backend, &master, "Then the IPv6", &json!([])).await;
    let other = save(&backend, &master, "Other", &json!([])).await;

    // With the composer empty, the edit moves the message into the draft and
    // makes no message; sent again, it answers the same, not 404.
    let op = fresh();
    let first = edit_as(&backend, &master, &op, &dns.id, &fresh()).await;
    assert_eq!(first.status, StatusCode::OK, "{}", String::from_utf8_lossy(&first.body));
    let first = first.json::<DraftWritten>();
    assert_eq!((first.draft.text.as_str(), first.revisions), ("Check the DNS", Revisions { draft: first.draft.revision, saved: 4 }));
    let again = edit_as(&backend, &master, &op, &dns.id, &fresh()).await;
    assert_eq!(again.status, StatusCode::OK, "{}", String::from_utf8_lossy(&again.body));
    assert_eq!(again.json::<DraftWritten>(), first);

    // With "Check the DNS" in the composer, an edit of the IPv6 message
    // saves it under the page's id; that message is deleted afterwards, and
    // the edit sent again still answers as it first did and moves nothing.
    let op = fresh();
    let becomes = fresh();
    let second = edit_as(&backend, &master, &op, &ipv6.id, &becomes).await.json::<DraftWritten>();
    assert_eq!(second.draft.text, "Then the IPv6");
    let read = list(&backend, &master).await;
    assert_eq!(texts(&read), vec!["Check the DNS", "Other"]);
    assert_eq!(read.messages[0].id, becomes);
    delete(&backend, &master, &becomes).await;
    let again = edit_as(&backend, &master, &op, &ipv6.id, &becomes).await;
    assert_eq!(again.status, StatusCode::OK, "{}", String::from_utf8_lossy(&again.body));
    assert_eq!(again.json::<DraftWritten>(), second);
    assert_eq!(texts(&list(&backend, &master).await), vec!["Other"]);

    // A new edit naming an id a message holds is refused, and nothing moves.
    let held = save(&backend, &master, "Held", &json!([])).await;
    assert_eq!(
        edit_as(&backend, &master, &fresh(), &other.id, &held.id).await.refusal(),
        (StatusCode::CONFLICT, ErrorCode::IdTaken)
    );
    assert_eq!(texts(&list(&backend, &master).await), vec!["Held", "Other"]);

    // An edit's answer is kept a day.
    harness.clock.advance(jiff::SignedDuration::from_hours(25));
    let late = save(&backend, &master, "Late", &json!([])).await;
    let op = fresh();
    edit_as(&backend, &master, &op, &late.id, &fresh()).await;
    harness.clock.advance(jiff::SignedDuration::from_hours(25));
    assert_eq!(
        edit_as(&backend, &master, &op, &late.id, &fresh()).await.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::SavedNotFound)
    );
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
    let send_saved = |id: &str, turn: &str, delivery| ClientFrame::SendSaved {
        id: id.to_owned(),
        turn_id: TurnId::try_from(turn).unwrap(),
        delivery,
    };
    socket.send(&send_saved(&lost.id, &fresh(), SavedDelivery::Message)).await;
    socket
        .until(|frame| matches!(frame, ServerFrame::Error { .. }))
        .await;
    let read = list(&backend, &master).await;
    assert_eq!((read.revision, read.messages.len()), (3, 3));
    delete(&backend, &master, &lost.id).await;

    // Sent as the next message, it starts a turn with the id the page named,
    // its file where its mark stood, and leaves the list.
    vendor.respond(answer(&["Seen."], 1, 1));
    let look_turn = fresh();
    socket.send(&send_saved(&look.id, &look_turn, SavedDelivery::Message)).await;
    socket.until_idle().await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let user = blocks
        .iter()
        .find_map(|block| match block {
            Block::User(user) => Some(user),
            _ => None,
        })
        .expect("the message is in the transcript");
    assert_eq!(user.turn_id.as_str(), look_turn);
    assert!(matches!(
        user.content.as_slice(),
        [UserContentBlock::Text { text }, UserContentBlock::Image { source: MediaSource::Ref { .. } }, ..]
            if text == "Look at "
    ), "{:?}", user.content);
    assert_eq!(texts(&list(&backend, &master).await), vec!["Also check the IPv6 records"]);

    // While a turn runs, one goes as a steer under the id the page named;
    // sent again with that id, as the page does after a reload, it answers
    // as it did, and nothing is sent twice.
    vendor.respond(MockResponse::event_stream(": thinking\n\n").stay_open());
    socket.send(&send("m2", "Hold on")).await;
    vendor.received(2).await;
    let steer_turn = fresh();
    let accepted_steer = ServerFrame::SteerResult {
        steer_id: BlockId::try_from(steer_turn.clone()).unwrap(),
        outcome: SteerOutcome::Accepted,
    };
    socket.send(&send_saved(&steer.id, &steer_turn, SavedDelivery::Steer)).await;
    let accepted = socket
        .until(|frame| matches!(frame, ServerFrame::SteerResult { .. }))
        .await;
    assert_eq!(accepted.last(), Some(&accepted_steer));
    socket.send(&send_saved(&steer.id, &steer_turn, SavedDelivery::Steer)).await;
    let again = socket
        .until(|frame| matches!(frame, ServerFrame::SteerResult { .. }))
        .await;
    assert_eq!(again.last(), Some(&accepted_steer));
    // Another page's send of it, as another turn, finds it gone and goes nowhere.
    socket.send(&send_saved(&steer.id, &fresh(), SavedDelivery::Steer)).await;
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

// About a second: two turns against a scripted endpoint.
#[tokio::test]
async fn undo_after_a_stale_delete_saves_a_new_message_whose_send_is_a_new_turn() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let check = save(&backend, &master, "Check the DNS", &json!([])).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let send_saved = |id: &str, turn: &str| ClientFrame::SendSaved {
        id: id.to_owned(),
        turn_id: TurnId::try_from(turn).unwrap(),
        delivery: SavedDelivery::Message,
    };

    // Page B sends it.
    vendor.respond(answer(&["Checked."], 1, 1));
    let first_turn = fresh();
    socket.send(&send_saved(&check.id, &first_turn)).await;
    socket.until_idle().await;
    let after_send = list(&backend, &master).await;
    assert!(after_send.messages.is_empty());

    // Page A still lists it and deletes it: the message is gone already,
    // and the delete answers the revisions as they are.
    let removed = delete(&backend, &master, &check.id).await;
    assert_eq!(removed.saved, after_send.revision);
    // A save of that id again, as a stale page's retry, makes nothing.
    let stale = add_as(&backend, &master, &check.id, "Check the DNS", &json!([])).await;
    assert_eq!(stale.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&stale.body));
    assert!(list(&backend, &master).await.messages.is_empty());

    // Its Undo saves the message again under a new id, and sending that is
    // a new turn, under the id of its own send.
    let undone = save(&backend, &master, "Check the DNS", &json!([])).await;
    assert_ne!(undone.id, check.id);
    vendor.respond(answer(&["Checked again."], 1, 1));
    let second_turn = fresh();
    socket.send(&send_saved(&undone.id, &second_turn)).await;
    socket.until_idle().await;
    let turns: Vec<String> = transcript(&backend, &master, FIRST)
        .await
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::User(user) => Some(user.turn_id.as_str().to_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(turns, vec![first_turn, second_turn]);
    assert!(list(&backend, &master).await.messages.is_empty());
    backend.close().await;
}
