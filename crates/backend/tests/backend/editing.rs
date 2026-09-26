//! Message editing through a conversation (`message-editing.md` § Commit and
//! idempotency, § Durability and failure boundaries): an edit the page sends
//! over the socket replaces its message and what followed it, gives the model
//! only the history it kept, restores the todos to the point before the
//! edited message and leaves the files the removed turns wrote. Sent again
//! over a socket that took the conversation over, or after a restart of the
//! backend, the same edit answers its receipt without asking the model again,
//! and an edit from the old snapshot is refused. No test calls a real model;
//! the device is a real runner.

use demi_agent::testing::{client_text, model_of};
use demi_agent_protocol::{ClientFrame, EditOutcome, EditRequest, ServerFrame};
use demi_core::{Block, UserContentBlock};
use demi_provider::testing::MockVendor;
use serde_json::json;

use crate::conversations::{FIRST, Socket, anthropic, answer, create, on_device, tool_result, tool_use};
use crate::support::Harness;

/// The outcome of the edit `frame` asks for, which `socket` answers.
async fn edit(socket: &mut Socket, frame: &ClientFrame) -> EditOutcome {
    socket.send(frame).await;
    let answered = socket.until(|frame| matches!(frame, ServerFrame::EditResult { .. })).await;
    match answered.into_iter().last() {
        Some(ServerFrame::EditResult { outcome, .. }) => outcome,
        other => unreachable!("{other:?}"),
    }
}

#[tokio::test]
async fn an_edit_restores_the_todos_keeps_the_files_and_answers_its_receipt_after_a_takeover_and_a_restart() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let (paired, root) = on_device(&harness, &backend, &master, FIRST).await;
    let model = model_of(&provider, "claude-opus-4-8");
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open(&model).await;
    let shell = |id: &str, script: &str| {
        tool_use(id, "shell_exec", &json!({ "description": id, "script": script, "timeoutMs": 60_000 }))
    };
    vendor.respond(answer(&["answer-A-kept"], 1, 1));
    socket.chat("m1", "A-kept").await;
    vendor.respond(shell("toolu_effects", "printf permanent > sentinel.txt && demi todo add \"permanent todo\""));
    vendor.respond(answer(&["answer-B-removed"], 1, 1));
    socket.chat("m2", "B-removed").await;
    vendor.respond(answer(&["answer-C-removed"], 1, 1));
    socket.chat("m3", "C-removed").await;

    // The editor's snapshot: the transcript and its version.
    socket.send(&ClientFrame::SyncTranscript {}).await;
    let synced = socket.until(|frame| matches!(frame, ServerFrame::TranscriptReset { .. })).await;
    let Some(ServerFrame::TranscriptReset { blocks, version, .. }) = synced.into_iter().last() else {
        unreachable!()
    };
    let target = blocks
        .iter()
        .find(|block| {
            matches!(block, Block::User(user)
                if matches!(&user.content[..], [UserContentBlock::Text { text }] if text == "B-removed"))
        })
        .expect("the second message")
        .id()
        .clone();
    let request = EditRequest {
        operation_id: "edit-1".try_into().unwrap(),
        target_block_id: target,
        version,
        content: client_text("B-edited"),
    };
    let repeated = ClientFrame::EditAndSend { request: request.clone() };
    let stale = ClientFrame::EditAndSend {
        request: EditRequest {
            operation_id: "edit-2".try_into().unwrap(),
            ..request
        },
    };

    let asked = vendor.requests().len();
    vendor.respond(answer(&["answer-edited"], 1, 1));
    let accepted = edit(&mut socket, &repeated).await;
    assert!(matches!(accepted, EditOutcome::Accepted { .. }), "{accepted:?}");
    socket.until(|frame| matches!(frame, ServerFrame::Phase { phase: demi_core::SessionPhase::Idle })).await;
    // The model reads the history the edit kept and the new message, on a
    // fresh runtime; the removed turns and their tool result are gone.
    let replayed = vendor.requests()[asked].json()["messages"].to_string();
    assert!(replayed.contains("A-kept") && replayed.contains("answer-A-kept") && replayed.contains("B-edited"), "{replayed}");
    assert!(!replayed.contains("B-removed") && !replayed.contains("C-removed"), "{replayed}");
    assert!(!replayed.contains("tool_result"), "{replayed}");
    // The files the removed turn wrote stay.
    assert_eq!(std::fs::read_to_string(format!("{root}/sentinel.txt")).unwrap(), "permanent");

    // Another socket takes the conversation over: the same edit answers its
    // receipt, and the old snapshot is refused, neither asking the model.
    let mut second = Socket::connect(&backend, &master, FIRST).await;
    second.open(&model).await;
    assert_eq!(edit(&mut second, &repeated).await, accepted);
    assert!(matches!(edit(&mut second, &stale).await, EditOutcome::Rejected { .. }));
    drop((socket, second));

    // After a restart as well: the receipt is durable.
    let address = backend.address();
    backend.close().await;
    let backend = harness.start_at(address).await;
    backend.until_online(&master, paired.id(), true).await;
    let mut third = Socket::connect(&backend, &master, FIRST).await;
    third.open(&model).await;
    assert_eq!(edit(&mut third, &repeated).await, accepted);
    assert!(matches!(edit(&mut third, &stale).await, EditOutcome::Rejected { .. }));
    assert_eq!(vendor.requests().len(), asked + 1, "no edit asked the model again");

    // The todos are as they were before the edited message; the file stays.
    let before = vendor.requests().len();
    vendor.respond(shell("toolu_check", "cat sentinel.txt && demi todo list"));
    vendor.respond(answer(&["checked"], 1, 1));
    third.chat("m4", "Verify the effects").await;
    let checked = tool_result(&vendor.requests()[before + 1].json(), "toolu_check");
    assert!(checked.contains("permanent") && checked.contains("No todos"), "{checked}");
    assert!(!checked.contains("permanent todo"), "{checked}");
    backend.close().await;
}
