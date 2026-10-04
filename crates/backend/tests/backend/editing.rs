//! Message editing through a conversation (`message-editing.md` § Commit and
//! idempotency, § Durability and failure boundaries): an edit the page sends
//! over the socket replaces its message and what followed it, gives the model
//! only the history it kept, and leaves the files the removed turns wrote.
//! Sent again
//! over another socket of the conversation, or after a restart of the
//! backend, the same edit answers its receipt without asking the model again,
//! and an edit from the old snapshot is refused. While the edit's commit is
//! held, the model is not asked and a reload reads the history before it;
//! the page hears of the edit once it commits. The page sees a turn end only
//! once the save that ends it has committed, so an edit it sends the moment
//! it sees idle is admitted. No test calls a real model; the device is a real
//! runner.

use std::time::Duration;

use demi_agent_server::testing::client_text;
use demi_conversation_socket_protocol::{ClientFrame, EditOutcome, EditRequest, ServerFrame};
use demi_provider_common::testing::MockVendor;
use demi_shared_types::{Block, SessionPhase, UserContentBlock};
use serde_json::json;

use crate::conversations::{
    FIRST, Socket, answer, anthropic, choose, create, on_device, send, tool_use,
    transcript,
};
use crate::support::Harness;

/// The outcome of the edit `frame` asks for, which `socket` answers.
async fn edit(socket: &mut Socket, frame: &ClientFrame) -> EditOutcome {
    socket.send(frame).await;
    let answered = socket
        .until(|frame| matches!(frame, ServerFrame::EditResult { .. }))
        .await;
    match answered.into_iter().last() {
        Some(ServerFrame::EditResult { outcome, .. }) => outcome,
        other => unreachable!("{other:?}"),
    }
}

/// An edit, as operation `operation`, of the user's message `target` into
/// `replacement`, from the snapshot the editor reads over `socket` now: the
/// transcript and its version.
async fn edit_request(
    socket: &mut Socket,
    target: &str,
    operation: &str,
    replacement: &str,
) -> EditRequest {
    socket.send(&ClientFrame::SyncTranscript {}).await;
    let synced = socket
        .until(|frame| matches!(frame, ServerFrame::TranscriptReset { .. }))
        .await;
    let Some(ServerFrame::TranscriptReset {
        blocks, version, ..
    }) = synced.into_iter().last()
    else {
        unreachable!()
    };
    let target_block_id = blocks
        .iter()
        .find(|block| {
            matches!(block, Block::User(user)
                if matches!(&user.content[..], [UserContentBlock::Text { text }] if text == target))
        })
        .expect("the message")
        .id()
        .clone();
    EditRequest {
        operation_id: operation.try_into().unwrap(),
        target_block_id,
        version,
        content: client_text(replacement),
    }
}

/// Whether a frame tells the page that the session is idle.
fn idle(frame: &ServerFrame) -> bool {
    matches!(
        frame,
        ServerFrame::Phase {
            phase: SessionPhase::Idle
        }
    )
}

// Over a second: a real device runs the turns' jobs, and its runner comes back
// after the backend's restart.
#[tokio::test]
async fn an_edit_keeps_the_files_and_answers_its_receipt_on_another_socket_and_after_a_restart() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let (paired, root) = on_device(&harness, &backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let shell = |id: &str, script: &str| {
        tool_use(
            id,
            "shell_exec",
            &json!({ "description": id, "script": script, "timeoutMs": 60_000 }),
        )
    };
    vendor.respond(answer(&["answer-A-kept"], 1, 1));
    socket.chat("m1", "A-kept").await;
    vendor.respond(shell("toolu_effects", "printf permanent > sentinel.txt"));
    vendor.respond(answer(&["answer-B-removed"], 1, 1));
    socket.chat("m2", "B-removed").await;
    vendor.respond(answer(&["answer-C-removed"], 1, 1));
    socket.chat("m3", "C-removed").await;

    let request = edit_request(&mut socket, "B-removed", "edit-1", "B-edited").await;
    let repeated = ClientFrame::EditAndSend {
        request: request.clone(),
    };
    let stale = ClientFrame::EditAndSend {
        request: EditRequest {
            operation_id: "edit-2".try_into().unwrap(),
            ..request
        },
    };

    let asked = vendor.requests().len();
    vendor.respond(answer(&["answer-edited"], 1, 1));
    let accepted = edit(&mut socket, &repeated).await;
    assert!(
        matches!(accepted, EditOutcome::Accepted { .. }),
        "{accepted:?}"
    );
    socket.until(idle).await;
    // The model reads the history the edit kept and the new message, on a
    // fresh runtime; the removed turns and their tool result are gone.
    let replayed = vendor.requests()[asked].json()["messages"].to_string();
    assert!(
        replayed.contains("A-kept")
            && replayed.contains("answer-A-kept")
            && replayed.contains("B-edited"),
        "{replayed}"
    );
    assert!(
        !replayed.contains("B-removed") && !replayed.contains("C-removed"),
        "{replayed}"
    );
    assert!(!replayed.contains("tool_result"), "{replayed}");
    // The files the removed turn wrote stay.
    assert_eq!(
        std::fs::read_to_string(format!("{root}/sentinel.txt")).unwrap(),
        "permanent"
    );

    // Another socket of the conversation, beside the first: the same edit
    // answers its receipt there, and the old snapshot is refused, neither
    // asking the model.
    let mut second = Socket::connect(&backend, &master, FIRST).await;
    second.open().await;
    assert_eq!(edit(&mut second, &repeated).await, accepted);
    assert!(matches!(
        edit(&mut second, &stale).await,
        EditOutcome::Rejected { .. }
    ));
    drop((socket, second));

    // After a restart as well: the receipt is durable. The backend comes
    // back at its address, where the device's runner reconnects.
    let address = backend.address();
    backend.close().await;
    let backend = harness.start_at(address).await;
    backend.until_online(&master, paired.id(), true).await;
    let mut third = Socket::connect(&backend, &master, FIRST).await;
    third.open().await;
    assert_eq!(edit(&mut third, &repeated).await, accepted);
    assert!(matches!(
        edit(&mut third, &stale).await,
        EditOutcome::Rejected { .. }
    ));
    assert_eq!(
        vendor.requests().len(),
        asked + 1,
        "no edit asked the model again"
    );
    backend.close().await;
}

#[tokio::test]
async fn an_edit_reaches_the_page_and_the_model_only_once_its_transaction_commits() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["answer-A-kept"], 1, 1));
    socket.chat("m1", "A-kept").await;
    vendor.respond(answer(&["answer-B-removed"], 1, 1));
    // The socket saw the turn end, so every save of the turns has committed,
    // and the next save is the edit's.
    socket.chat("m2", "B-removed").await;
    let history = transcript(&backend, &master, FIRST).await.blocks;
    let request = edit_request(&mut socket, "B-removed", "edit-1", "B-edited").await;

    let hold = backend.hold_commits();
    let asked = vendor.requests().len();
    vendor.respond(answer(&["answer-edited"], 1, 1));
    socket.send(&ClientFrame::EditAndSend { request }).await;
    tokio::time::timeout(Duration::from_secs(10), hold.until_waiting(1))
        .await
        .expect("the edit reaches its commit");
    // The edit's rows are written and its commit waits. A process that died
    // now would leave the database as a reload reads it here, with the
    // history before the edit, and the model is not asked. The page hears
    // of the edit only after the commit, which the agent shows where its
    // outbox can be read while the save waits (its editing tests).
    assert_eq!(transcript(&backend, &master, FIRST).await.blocks, history);
    assert_eq!(
        vendor.requests().len(),
        asked,
        "the model is not asked before the commit"
    );

    // Once the commit completes, the replacement and the result reach the
    // page, and the model is asked with the replacement.
    hold.release();
    let answered = socket
        .until(|frame| matches!(frame, ServerFrame::EditResult { .. }))
        .await;
    assert!(
        matches!(
            answered.last(),
            Some(ServerFrame::EditResult {
                outcome: EditOutcome::Accepted { .. },
                ..
            })
        ),
        "{answered:?}"
    );
    assert!(
        answered
            .iter()
            .any(|frame| matches!(frame, ServerFrame::TranscriptPatch { .. })),
        "{answered:?}"
    );
    socket.until(idle).await;
    let replayed = vendor.requests()[asked].json()["messages"].to_string();
    assert!(
        replayed.contains("B-edited") && !replayed.contains("B-removed"),
        "{replayed}"
    );
    backend.close().await;
}

#[tokio::test]
async fn the_page_sees_a_turn_end_once_its_save_commits_and_an_edit_sent_then_is_admitted() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["answer-A"], 1, 1));
    socket.chat("m1", "A-kept").await;

    // The next turn answers, and the save that ends it waits at its commit.
    let hold = backend.hold_commits();
    vendor.respond(answer(&["answer-B"], 1, 1));
    socket.send(&send("m2", "B-removed")).await;
    tokio::time::timeout(Duration::from_secs(10), hold.until_waiting(1))
        .await
        .expect("the turn reaches its save");
    // Meanwhile the page has not seen the turn end: the reset that answers a
    // sync follows every frame sent before it, and none of them says idle.
    socket.send(&ClientFrame::SyncTranscript {}).await;
    let meanwhile = socket
        .until(|frame| matches!(frame, ServerFrame::TranscriptReset { .. }))
        .await;
    assert!(
        !meanwhile.iter().any(idle),
        "the page saw the turn end before its save committed: {meanwhile:?}"
    );
    let Some(ServerFrame::TranscriptReset {
        blocks, version, ..
    }) = meanwhile.last()
    else {
        unreachable!()
    };
    let target_block_id = blocks
        .iter()
        .find(|block| matches!(block, Block::User(user) if user.turn_id.as_str() == "m2"))
        .expect("the message")
        .id()
        .clone();
    let request = EditRequest {
        operation_id: "edit-1".try_into().unwrap(),
        target_block_id,
        version: version.clone(),
        content: client_text("B-edited"),
    };

    // Once the save commits, the page sees the turn end, and an edit it sends
    // at that moment is admitted.
    hold.release();
    socket.until(idle).await;
    vendor.respond(answer(&["answer-edited"], 1, 1));
    let outcome = edit(&mut socket, &ClientFrame::EditAndSend { request }).await;
    assert!(
        matches!(outcome, EditOutcome::Accepted { .. }),
        "{outcome:?}"
    );
    socket.until(idle).await;
    backend.close().await;
}
