//! Versions: an edit continues from a new version of a message and keeps
//! the old one with its answers, as written; the page shows ‹ 2/2 › on the
//! message, and switching back restores the old branch in the transcript,
//! the history pages, what the model reads next and the search index, and a
//! restart keeps the path the switch made current. Regenerate adds a
//! version of the answer beside the old one. No test calls a real model.

use std::time::Duration;

use demi_conversation_socket_protocol::{
    BlockVersions, ClientFrame, EditOutcome, RegenerateRequest, ServerFrame, TranscriptVersion,
};
use demi_shared_types::{Block, BlockId, UserContentBlock};
use demi_web_api_protocol::conversations::TranscriptPage;
use demi_web_api_protocol::search::SearchResults;
use reqwest::StatusCode;

use crate::conversations::{FIRST, Socket, anthropic, answer, choose, create};
use crate::editing::{edit, edit_request, idle};
use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD, Session, TestBackend, eventually};

/// The live transcript a fresh reset sends, with its version and the
/// versions of its blocks.
async fn sync(socket: &mut Socket) -> (Vec<Block>, TranscriptVersion, Vec<BlockVersions>) {
    socket.send(&ClientFrame::SyncTranscript { from: None, edge: None }).await;
    let frames = socket
        .until(|frame| matches!(frame, ServerFrame::TranscriptReset { .. }))
        .await;
    match frames.into_iter().last() {
        Some(ServerFrame::TranscriptReset { blocks, version, versions, .. }) => (blocks, version, versions),
        other => unreachable!("{other:?}"),
    }
}

/// What the page shows of `blocks` as text: the user's messages and the
/// agent's answers.
fn said(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .filter_map(|block| match block {
            Block::User(user) => match &user.content[..] {
                [UserContentBlock::Text { text }] => Some(text.clone()),
                _ => None,
            },
            Block::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect()
}

/// The id of the block that says `text`.
fn id_of(blocks: &[Block], text: &str) -> BlockId {
    blocks
        .iter()
        .find(|block| said(std::slice::from_ref(block)) == [text])
        .unwrap_or_else(|| panic!("no block says {text}"))
        .id()
        .clone()
}

fn versions(block: BlockId, index: u32, count: u32) -> BlockVersions {
    BlockVersions {
        block_id: block,
        index,
        count,
    }
}

/// The conversation's latest page as the history route answers it.
async fn latest_page(backend: &TestBackend, session: &Session) -> TranscriptPage {
    let read = backend.get(&format!("/api/conversations/{FIRST}/transcript"), Some(session)).await;
    assert_eq!(read.status, StatusCode::OK, "{}", String::from_utf8_lossy(&read.body));
    read.json()
}

/// Whether a search for `word` finds the conversation.
async fn finds(backend: &TestBackend, session: &Session, word: &str) -> bool {
    let read = backend.get(&format!("/api/search?q={word}"), Some(session)).await;
    assert_eq!(read.status, StatusCode::OK);
    !read.json::<SearchResults>().results.is_empty()
}

/// Switches the place of `block` to version `to`, from the snapshot
/// `version`, and reads the frames up to the switch's end: the rewrite's
/// patches, then the new path's context usage before the phase goes idle.
async fn switch(socket: &mut Socket, block: BlockId, to: u32, version: TranscriptVersion) {
    socket
        .send(&ClientFrame::SwitchVersion {
            block_id: block,
            to,
            version,
        })
        .await;
    let frames = socket.until(|frame| idle(frame) || matches!(frame, ServerFrame::Rejected { .. })).await;
    let patched = frames
        .iter()
        .position(|frame| matches!(frame, ServerFrame::TranscriptPatch { .. }))
        .unwrap_or_else(|| panic!("no patches: {frames:?}"));
    assert!(
        frames[patched..]
            .iter()
            .any(|frame| matches!(frame, ServerFrame::ContextUsage { usage: Some(_), .. })),
        "the new path's usage before the switch ends: {frames:?}"
    );
}

// About a second: four turns, an edit, a switch and a restart; the search
// index follows each change within the 20 ms interval the test sets.
#[tokio::test]
async fn an_edit_keeps_the_old_branch_and_switching_back_restores_it_across_a_restart() {
    let vendor = demi_provider_common::testing::MockVendor::start().await;
    let mut harness = Harness::new();
    harness.conversations.search_interval = Duration::from_millis(20);
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["answer-alpha"], 1, 1));
    socket.chat("m1", "question-alpha").await;
    vendor.respond(answer(&["answer-bravo"], 1, 1));
    socket.chat("m2", "question-bravo").await;
    vendor.respond(answer(&["answer-charlie"], 1, 1));
    socket.chat("m3", "question-charlie").await;
    let (before, _, none) = sync(&mut socket).await;
    assert!(none.is_empty(), "{none:?}");

    let request = edit_request(&mut socket, "question-bravo", "edit-1", "question-delta").await;
    vendor.respond(answer(&["answer-delta"], 1, 1));
    let accepted = edit(&mut socket, &ClientFrame::EditAndSend { request }).await;
    assert!(matches!(accepted, EditOutcome::Accepted { .. }), "{accepted:?}");
    socket.until(idle).await;

    // The new version is the second of its place, on the page and in the
    // history route alike.
    let (edited, version, held) = sync(&mut socket).await;
    assert_eq!(said(&edited), ["question-alpha", "answer-alpha", "question-delta", "answer-delta"]);
    let delta = id_of(&edited, "question-delta");
    assert_eq!(held, [versions(delta.clone(), 2, 2)]);
    let page = latest_page(&backend, &master).await;
    assert_eq!(said(&page.blocks), said(&edited));
    assert_eq!(page.versions, [versions(delta.clone(), 2, 2)]);
    eventually("the index holds the new version", || async {
        finds(&backend, &master, "question-delta").await && !finds(&backend, &master, "question-charlie").await
    })
    .await;

    // Back to the first version: the old branch as it was written, block
    // for block.
    switch(&mut socket, delta, 1, version).await;
    let (restored, _, held) = sync(&mut socket).await;
    assert_eq!(restored, before);
    let bravo = id_of(&restored, "question-bravo");
    assert_eq!(held, [versions(bravo.clone(), 1, 2)]);
    let page = latest_page(&backend, &master).await;
    assert_eq!(said(&page.blocks), said(&before));
    assert_eq!(page.versions, [versions(bravo.clone(), 1, 2)]);
    eventually("the index follows the switch", || async {
        finds(&backend, &master, "question-charlie").await && !finds(&backend, &master, "question-delta").await
    })
    .await;

    // The next message continues from what the page shows.
    let asked = vendor.requests().len();
    vendor.respond(answer(&["answer-echo"], 1, 1));
    socket.chat("m5", "question-echo").await;
    let replayed = vendor.requests()[asked].json()["messages"].to_string();
    for kept in ["question-alpha", "answer-bravo", "question-charlie", "answer-charlie", "question-echo"] {
        assert!(replayed.contains(kept), "{kept}: {replayed}");
    }
    assert!(!replayed.contains("delta"), "{replayed}");
    drop(socket);

    // A restart restores the path the switch made current.
    backend.close().await;
    let backend = harness.start().await;
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let (reopened, _, held) = sync(&mut socket).await;
    assert_eq!(
        said(&reopened),
        [
            "question-alpha",
            "answer-alpha",
            "question-bravo",
            "answer-bravo",
            "question-charlie",
            "answer-charlie",
            "question-echo",
            "answer-echo"
        ]
    );
    assert_eq!(held, [versions(bravo, 1, 2)]);
    drop(socket);
    backend.close().await;
}

// Two turns and a Regenerate, well under a second.
#[tokio::test]
async fn regenerate_adds_a_version_of_the_answer_beside_the_old_one() {
    let vendor = demi_provider_common::testing::MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["first-answer"], 1, 1));
    socket.chat("m1", "the-question").await;
    let (before, version, _) = sync(&mut socket).await;

    let asked = vendor.requests().len();
    vendor.respond(answer(&["second-answer"], 1, 1));
    let regenerate = ClientFrame::Regenerate {
        request: RegenerateRequest {
            operation_id: "regenerate-1".try_into().unwrap(),
            target_block_id: id_of(&before, "the-question"),
            version,
        },
    };
    let accepted = edit(&mut socket, &regenerate).await;
    assert!(matches!(accepted, EditOutcome::Accepted { .. }), "{accepted:?}");
    socket.until(idle).await;

    // The message stays, once, and the model reads it without the old
    // answer.
    let replayed = vendor.requests()[asked].json()["messages"].to_string();
    assert!(replayed.contains("the-question") && !replayed.contains("first-answer"), "{replayed}");
    let (regenerated, version, held) = sync(&mut socket).await;
    assert_eq!(said(&regenerated), ["the-question", "second-answer"]);
    assert_eq!(regenerated[0], before[0]);
    // The answer's first block holds its place's versions.
    assert_eq!(held, [versions(regenerated[1].id().clone(), 2, 2)]);

    switch(&mut socket, regenerated[1].id().clone(), 1, version).await;
    let (restored, _, held) = sync(&mut socket).await;
    assert_eq!(restored, before);
    assert_eq!(held, [versions(before[1].id().clone(), 1, 2)]);
    drop(socket);
    backend.close().await;
}
