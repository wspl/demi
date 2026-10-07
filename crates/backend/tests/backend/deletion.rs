//! Deleting a conversation (`storage.md` § Deleting a conversation,
//! `web-api.md` § Sidebar mutations, read state and page synchronization):
//! nothing refuses it, a running conversation's command stops, and nothing
//! finds the conversation after; a start finishes a deletion a crash cut
//! short; and the collection after it removes only the blobs that nothing
//! names and that are older than a day. The model is an Anthropic endpoint
//! the test scripts; the device is a real runner.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_provider_common::testing::MockVendor;
use demi_shared_types::{Block, BlobRef, MediaSource, UserContentBlock};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::search::SearchResults;
use demi_web_api_protocol::state::SyncEvent;
use jiff::SignedDuration;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{
    FIRST, SECOND, Socket, answer, anthropic, choose, create, on_device, send, summaries,
    transcript,
};
use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD, Session, TestBackend, eventually};
use crate::uploads::{png, upload, with_upload};
use crate::work::shell;

const DAY: SignedDuration = SignedDuration::from_hours(24);

/// The database file of the conversation `id`.
fn database(harness: &Harness, id: &str) -> PathBuf {
    harness
        .data_dir()
        .join("conversations")
        .join(format!("{id}.sqlite"))
}

/// How many deletions the control database records as pending.
fn pending(harness: &Harness) -> i64 {
    harness
        .control_database()
        .query_row("SELECT count(*) FROM conversation_deletions", [], |row| {
            row.get(0)
        })
        .unwrap()
}

/// Where the blob `blob` lies in `session`'s namespace.
fn blob_path(harness: &Harness, session: &Session, blob: &str) -> PathBuf {
    harness
        .data_dir()
        .join("blobs")
        .join(session.user.id.as_str())
        .join(blob)
}

/// Whether the process `pid` runs.
/// The conversations a search for `query` lists.
async fn found(backend: &TestBackend, session: &Session, query: &str) -> Vec<String> {
    let answer = backend.get(&format!("/api/search?q={query}"), Some(session)).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.body));
    answer
        .json::<SearchResults>()
        .results
        .into_iter()
        .map(|result| result.conversation_id.into_string())
        .collect()
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

// About two seconds: a real device runs the command the deletion stops.
#[tokio::test]
async fn deleting_a_running_conversation_stops_its_command_and_nothing_finds_it_after() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    create(&backend, &master, SECOND).await;
    let (_paired, root) = on_device(&harness, &backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let mut page = backend.sync(&master).await;
    page.snapshot().await;
    let command = "sh -c \"echo \\$\\$ > cmd.pid; exec /bin/sleep 30\"";
    vendor.respond(shell("t1", command, 30_000));
    socket.send(&send("m1", "Wait")).await;
    let pid = Path::new(&root).join("cmd.pid");
    eventually("the command runs", || {
        let started = std::fs::read_to_string(&pid).is_ok_and(|pid| pid.ends_with('\n'));
        async move { started }
    })
    .await;
    let pid = std::fs::read_to_string(&pid).unwrap().trim().to_owned();
    assert!(alive(&pid));
    eventually("search finds the message", || {
        let found = found(&backend, &master, "Wait");
        async move { found.await == [FIRST] }
    })
    .await;

    let path = format!("/api/conversations/{FIRST}");
    let deleted = backend.delete(&path, &master).await;
    assert_eq!(
        deleted.status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&deleted.body)
    );
    eventually("the command ended", || {
        let alive = alive(&pid);
        async move { !alive }
    })
    .await;
    // The page's tree closes, and an open of it again finds no conversation;
    // the page's synchronization channel drops the conversation.
    socket.until(|frame| *frame == ServerFrame::Closed).await;
    socket.send(&ClientFrame::Open {}).await;
    let refused = socket
        .until(|frame| matches!(frame, ServerFrame::Error { .. }))
        .await
        .pop();
    assert!(
        matches!(&refused, Some(ServerFrame::Error { code: Some(code), .. }) if code == "conversation_not_found"),
        "{refused:?}"
    );
    page.until(|event| {
        matches!(event, SyncEvent::ConversationDeleted { id } if id.as_str() == FIRST)
    })
    .await;

    // Nothing finds it: its history, a change, another deletion, its socket.
    let not_found = (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound);
    let read = backend
        .get(&format!("{path}/transcript"), Some(&master))
        .await;
    assert_eq!(read.refusal(), not_found);
    let renamed = backend.patch(&path, &master, json!({ "title": "Again" })).await;
    assert_eq!(renamed.refusal(), not_found);
    assert_eq!(backend.delete(&path, &master).await.refusal(), not_found);
    assert!(matches!(
        Socket::try_connect(&backend, &master, FIRST).await,
        Err(404)
    ));
    let listed: Vec<String> = summaries(&backend, &master)
        .await
        .into_iter()
        .map(|summary| summary.id.as_str().to_owned())
        .collect();
    assert_eq!(listed, [SECOND]);
    assert!(found(&backend, &master, "Wait").await.is_empty());
    assert!(!database(&harness, FIRST).exists());
    assert_eq!(pending(&harness), 0);
    backend.close().await;
}

#[tokio::test]
async fn a_start_finishes_a_deletion_a_crash_left_after_its_first_step() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["Done."], 1, 1));
    socket.chat("m1", "Go").await;
    drop(socket);
    backend.close().await;
    // As the first step's commit leaves it: the records are gone, the
    // deletion is pending, and the database is still there.
    harness
        .control_database()
        .execute_batch(&format!(
            "DELETE FROM conversations WHERE id = '{FIRST}';
             INSERT INTO conversation_deletions (id, user_id, deleted_at)
               VALUES ('{FIRST}', '{}', 0);",
            master.user.id
        ))
        .unwrap();
    assert!(database(&harness, FIRST).exists());

    let backend = harness.start().await;
    assert!(!database(&harness, FIRST).exists());
    assert_eq!(pending(&harness), 0);
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let read = backend
        .get(
            &format!("/api/conversations/{FIRST}/transcript"),
            Some(&master),
        )
        .await;
    assert_eq!(
        read.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound)
    );
    backend.close().await;
}

#[tokio::test]
async fn the_collection_after_a_deletion_keeps_what_a_fork_names_and_what_is_younger_than_a_day()
{
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    harness.clock.follow_system();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    // A screenshot the message sends, which a fork of the conversation holds
    // too.
    let shot = upload(&backend, &master, "shot.png", "image/png", &png(1)).await;
    vendor.respond(answer(&["Seen."], 1, 1));
    socket
        .send(&with_upload("m1", "Look", &[(&shot, "shot.png")]))
        .await;
    socket.until_idle().await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let text = blocks
        .iter()
        .rev()
        .find(|block| matches!(block, Block::Text(_)))
        .unwrap()
        .id()
        .clone();
    let forked = backend
        .post(
            &format!("/api/conversations/{FIRST}/fork"),
            Some(&master),
            json!({ "id": SECOND, "blockId": text }),
        )
        .await;
    assert_eq!(
        forked.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&forked.body)
    );
    let named = transcript(&backend, &master, SECOND)
        .await
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::User(user) => user.content.iter().find_map(|part| match part {
                UserContentBlock::Image {
                    source: MediaSource::Ref { r#ref, .. },
                } => Some(r#ref.to_string()),
                _ => None,
            }),
            _ => None,
        })
        .expect("the fork's message holds the screenshot");
    // An upload nothing names.
    let unsent = upload(&backend, &master, "unsent.png", "image/png", &png(2)).await;
    // A day and an hour later, a blob written an hour ago, whose row is
    // not written yet.
    harness.clock.advance(DAY + SignedDuration::from_hours(1));
    let young_bytes = png(3);
    let young = blob_path(&harness, &master, BlobRef::of(&young_bytes).as_str());
    std::fs::write(&young, &young_bytes).unwrap();
    let hour_ago = demi_shared_types::Clock::now(&*harness.clock).as_millisecond() - 3_600_000;
    let hour_ago = SystemTime::UNIX_EPOCH + Duration::from_millis(u64::try_from(hour_ago).unwrap());
    std::fs::File::options()
        .write(true)
        .open(&young)
        .unwrap()
        .set_modified(hour_ago)
        .unwrap();

    let deleted = backend
        .delete(&format!("/api/conversations/{FIRST}"), &master)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    backend.until_collected(&master).await;
    assert!(blob_path(&harness, &master, &named).exists());
    assert!(young.exists());
    assert!(!blob_path(&harness, &master, unsent.sha256.as_str()).exists());
    let unsent_records: i64 = harness
        .control_database()
        .query_row(
            "SELECT count(*) FROM attachments WHERE id = ?1",
            [unsent.id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unsent_records, 0, "the upload record goes with its blob");
    backend.close().await;
}
