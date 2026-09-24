//! Message editing at the session (`message-editing.md`): what an edit
//! shows before its save commits, what the session admits while the edit is
//! prepared, a stop or dispose during it, the work that refuses it, the
//! order of saves, where the cut falls, and what stays accepted when the
//! replacement's turn fails.

use demi_core::{AgentMessage, AgentMessageEvent, BlockId, Sender};
use demi_shell::{PortError, StorageOp};

use super::*;
use crate::{testing::TokioClock, transcript::RESUME_TEXT};

fn said(text: &str) -> Turn {
    Turn::Events(vec![event::text(text), event::response(1, 1)])
}

/// Every transcript patch the session publishes from now on.
fn published(session: &AgentSession) -> (Subscription, Rc<RefCell<Vec<TranscriptPatch>>>) {
    let patches = Rc::new(RefCell::new(Vec::new()));
    let subscription = session.subscribe({
        let patches = patches.clone();
        move |event| {
            if let SessionEvent::TranscriptChanged { patches: batch, .. } = event {
                patches.borrow_mut().extend(batch.iter().cloned());
            }
        }
    });
    (subscription, patches)
}

/// A message from another agent of the tree to the root.
fn agent_message() -> AgentMessage {
    AgentMessage {
        id: BlockId::try_from("m1").unwrap(),
        sender: Sender {
            id: NodeId::try_from("child").unwrap(),
            description: "reader".into(),
            round: 1,
        },
        recipient_id: root(),
        timestamp: Timestamp::UNIX_EPOCH,
        content: "found it".into(),
        event: AgentMessageEvent::Message {},
    }
}

/// Stops the running action in a task of its own; the task ends once the
/// stop is recorded.
fn spawn_abort(session: &AgentSession) -> JoinHandle<demi_agent_protocol::AbortResult> {
    let session = session.clone();
    tokio::task::spawn_local(async move { session.abort().await })
}

#[tokio::test(flavor = "local")]
async fn while_an_edit_saves_nothing_of_it_shows_and_the_session_admits_only_the_same_request() {
    let provider = ScriptedRuntime::new([said("answer A"), said("answer B"), said("answer B3")]);
    let store = MemoryTreeStore::new();
    let (runtime, runtimes) = NumberedRuntime::first(&provider);
    let session = start_with(
        Box::new(runtime),
        test_runtime(Vec::new()),
        &store,
        SessionConfig::default(),
        Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
    )
    .await;
    for message in ["A", "B"] {
        session
            .send(text(message), turn(message))
            .unwrap()
            .await
            .unwrap();
    }
    let before = session.transcript();
    let (_subscription, patches) = published(&session);
    let stored = || store.checkpoint(&root()).unwrap();

    // A save that fails: while it waits nothing of the candidate shows, and
    // afterwards only the candidate's runtime has closed.
    let gate = store.hold_saves();
    store.fail_saves(1);
    let failing = spawn_edit(&session, edit_of(&session, "B", "op1", "B2"));
    until(|| gate.waiting() == 1).await;
    assert_eq!(session.transcript(), before);
    assert!(patches.borrow().is_empty());
    assert_eq!(stored().transcript, before.blocks);
    gate.release();
    assert_eq!(
        failing.await.unwrap(),
        Err(EditError::Failed("the database refused the save".into()))
    );
    assert_eq!(session.transcript(), before);
    assert!(patches.borrow().is_empty());
    assert_eq!(stored().transcript, before.blocks);
    assert!(stored().state.edits.is_empty());
    assert_eq!(runtimes.borrow().closed, [1]);

    // A save that commits: while it waits, the same request shares its
    // acceptance, and the session refuses every other change.
    let gate = store.hold_saves();
    let submission = edit_of(&session, "B", "op2", "B3");
    let first = spawn_edit(&session, submission.clone());
    until(|| gate.waiting() == 1).await;
    assert_eq!(session.transcript(), before);
    assert!(patches.borrow().is_empty());
    assert_eq!(stored().transcript, before.blocks);
    let in_flight = session.check_edit(
        &submission.operation_id,
        &submission.digest,
        &submission.version,
    );
    assert!(matches!(in_flight, Ok(EditCheck::InFlight(_))));
    let repeated = spawn_edit(&session, submission.clone());
    tokio::task::yield_now().await;
    let conflicting = EditSubmission {
        digest: "op2:another request".into(),
        ..submission.clone()
    };
    assert_eq!(
        session.edit_and_send(conflicting).await,
        Err(EditError::Conflict)
    );
    assert_eq!(
        session
            .edit_and_send(edit_of(&session, "A", "op3", "A2"))
            .await,
        Err(EditError::Busy)
    );
    assert_eq!(
        session.send(text("C"), turn("C")).err(),
        Some(AdmissionError::Editing)
    );
    assert_eq!(
        session.steer(text("mind B"), BlockId::try_from("s1").unwrap()),
        Err(SteerError::Editing)
    );
    let switch = ModelSwitch {
        model: model_of("stub", "other-model"),
        runtime: None,
        apply: ModelSwitchApply::NextTurn,
    };
    assert_eq!(session.update_model(switch), Err(AdmissionError::Editing));
    assert_eq!(session.retry().err(), Some(AdmissionError::Editing));
    assert_eq!(session.compact().err(), Some(AdmissionError::Editing));
    assert_eq!(
        session.accept_agent_message(agent_message()).await,
        Err(AgentMessageError::Editing)
    );
    let read = StorageOp::Read { key: "todo".into() };
    assert_eq!(
        session.storage(read, Vec::new()).await,
        Err(PortError::Storage(
            "Command storage is reserved for a transcript edit".into()
        ))
    );
    gate.release();

    let receipt = first.await.unwrap().unwrap();
    assert_eq!(repeated.await.unwrap(), Ok(receipt.clone()));
    session.settled().await;
    // One rewrite, published once its save committed, then the replacement's
    // turn.
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        ["user", "text", "response", "user", "text", "response"]
    );
    assert_eq!(blocks[..3], before.blocks[..3]);
    assert!(matches!(&blocks[3], Block::User(user) if user.turn_id == receipt.turn_id));
    let patches = patches.borrow();
    assert!(
        matches!(patches.first(), Some(TranscriptPatch::Replace { value }) if value[..] == blocks[..4])
    );
    let rewrites = patches
        .iter()
        .filter(|patch| matches!(patch, TranscriptPatch::Replace { .. }))
        .count();
    assert_eq!(rewrites, 1);
    assert_eq!(stored().transcript, blocks);
    assert_eq!(stored().state.edits, [receipt]);
    assert!(session.queued_messages().is_empty());
    // The replacement asked on a fork of its own (2); the runtime that had
    // read B's answer (0) closed, after the failed edit's fork (1).
    let runtimes = runtimes.borrow();
    assert_eq!(runtimes.served, [0, 0, 2]);
    assert_eq!(runtimes.closed, [1, 0]);
}

#[tokio::test(flavor = "local")]
async fn a_stop_while_an_edit_prepares_rejects_it_and_while_it_saves_the_save_decides() {
    let provider = ScriptedRuntime::new([said("answer A")]);
    let store = MemoryTreeStore::new();
    let hang = Rc::new(Cell::new(false));
    let runtime = TestRuntime {
        hanging_preamble: hang.clone(),
        ..test_runtime(Vec::new())
    };
    let session = start_on(&provider, runtime, &store, SessionConfig::default()).await;
    session
        .send(text("A"), turn("A"))
        .unwrap()
        .await
        .unwrap();
    let before = session.transcript();

    // Stopped while its preamble is prepared: rejected, and the accepted
    // history gains no stop marker.
    hang.set(true);
    let preparing = spawn_edit(&session, edit_of(&session, "A", "op1", "A2"));
    until(|| session.phase() == SessionPhase::Running).await;
    tokio::task::yield_now().await;
    let stopped = session.abort().await;
    assert_eq!(stopped.target, Some(AbortTarget::ActiveTurn));
    assert_eq!(preparing.await.unwrap(), Err(EditError::Stopped));
    assert_eq!(session.transcript(), before);
    assert!(session.is_settled());
    hang.set(false);

    // Stopped while a save that fails runs: the stop waits for the save, and
    // the edit is rejected with its failure.
    let gate = store.hold_saves();
    store.fail_saves(1);
    let failing = spawn_edit(&session, edit_of(&session, "A", "op2", "A2"));
    until(|| gate.waiting() == 1).await;
    let stopping = spawn_abort(&session);
    tokio::task::yield_now().await;
    assert!(!stopping.is_finished());
    gate.release();
    assert_eq!(
        failing.await.unwrap(),
        Err(EditError::Failed("the database refused the save".into()))
    );
    assert_eq!(
        stopping.await.unwrap().target,
        Some(AbortTarget::ActiveTurn)
    );
    assert_eq!(session.transcript(), before);

    // Stopped while a save that commits runs: the edit is accepted, the stop
    // is recorded after the replacement, and no turn starts.
    let gate = store.hold_saves();
    let committing = spawn_edit(&session, edit_of(&session, "A", "op3", "A3"));
    until(|| gate.waiting() == 1).await;
    let stopping = spawn_abort(&session);
    tokio::task::yield_now().await;
    gate.release();
    let receipt = committing.await.unwrap().unwrap();
    assert_eq!(
        stopping.await.unwrap().target,
        Some(AbortTarget::ActiveTurn)
    );
    session.settled().await;
    let blocks = session.transcript().blocks;
    assert_eq!(kinds(&blocks), ["user", "abort"]);
    assert!(matches!(&blocks[0], Block::User(user) if user.turn_id == receipt.turn_id));
    assert_eq!(provider.requests().len(), 1);
    let stored = store.checkpoint(&root()).unwrap();
    assert_eq!(stored.transcript, blocks);
    assert_eq!(stored.state.edits, [receipt]);
}

#[tokio::test(flavor = "local")]
async fn dispose_during_an_edits_save_waits_for_it_and_keeps_the_accepted_replacement() {
    let provider = ScriptedRuntime::new([said("answer A")]);
    let store = MemoryTreeStore::new();
    let (runtime, runtimes) = NumberedRuntime::first(&provider);
    let session = start_with(
        Box::new(runtime),
        test_runtime(Vec::new()),
        &store,
        SessionConfig::default(),
        Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
    )
    .await;
    session
        .send(text("A"), turn("A"))
        .unwrap()
        .await
        .unwrap();
    let gate = store.hold_saves();
    let editing = spawn_edit(&session, edit_of(&session, "A", "op1", "A2"));
    until(|| gate.waiting() == 1).await;

    let disposing = {
        let session = session.clone();
        tokio::task::spawn_local(async move { session.dispose().await })
    };
    tokio::task::yield_now().await;
    // The save that started finishes first, and no runtime closes meanwhile.
    assert!(!disposing.is_finished());
    assert!(runtimes.borrow().closed.is_empty());
    gate.release();

    let receipt = editing.await.unwrap().unwrap();
    disposing.await.unwrap().unwrap();
    // The replaced runtime (0) closed at the acceptance and the
    // replacement's (1) with the session; the replacement's turn never
    // asked, and its record says it was interrupted.
    let runtimes = runtimes.borrow();
    assert_eq!(runtimes.closed, [0, 1]);
    assert_eq!(runtimes.served, [0]);
    let stored = store.checkpoint(&root()).unwrap();
    assert_eq!(stored.transcript, session.transcript().blocks);
    assert_eq!(kinds(&stored.transcript), ["user", "error"]);
    assert!(matches!(&stored.transcript[0], Block::User(user) if user.turn_id == receipt.turn_id));
    assert_eq!(stored.state.edits, [receipt]);
    assert_eq!(stored.state.phase, SessionPhase::Running);
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_scheduled_wakeup_refuses_an_edit_and_still_fires() {
    let provider = ScriptedRuntime::new([yield_call(60_000), said("checked the build")]);
    let store = MemoryTreeStore::new();
    let clock = Arc::new(TokioClock::new(Timestamp::UNIX_EPOCH));
    let session = start_at(
        &provider,
        test_runtime(vec![yield_tool()]),
        &store,
        SessionConfig::default(),
        clock,
    )
    .await;
    session
        .send(text("build it"), turn("A"))
        .unwrap()
        .await
        .unwrap();
    let before = session.transcript();

    let refused = session
        .edit_and_send(edit_of(&session, "A", "op1", "build it again"))
        .await;

    assert_eq!(refused, Err(EditError::Busy));
    assert_eq!(session.transcript(), before);
    assert!(session.status().wakeups);
    tokio::time::sleep(Duration::from_secs(61)).await;
    until(|| provider.requests().len() == 2).await;
    session.settled().await;
    assert_eq!(
        kinds(&session.transcript().blocks[3..]),
        ["wakeup", "text", "response"]
    );
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_save_under_way_when_an_edit_starts_commits_first_and_brings_back_no_removed_row() {
    let provider = ScriptedRuntime::new([Turn::Events(vec![
        event::tool_call("call-1", "write_once", json!({})),
        event::response(1, 1),
    ])]);
    let store = MemoryTreeStore::new();
    let (write_once, _releases, started) = gated_tool("write_once");
    let session = start(&provider, vec![write_once], &store, SessionConfig::default()).await;
    let _running = session.send(text("write it"), turn("A")).unwrap();
    started.await.unwrap();
    // The process dies during the tool.
    let crashed = store.copy();
    drop(session);
    let (answer, release) = gated_turn(vec![event::text("wrote it"), event::response(1, 1)]);
    let later = ScriptedRuntime::new([answer]);
    let (restored, _) = restore_session(
        crashed.checkpoint(&root()).unwrap(),
        &crashed,
        &later,
        test_runtime(Vec::new()),
        Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
    );
    // The call completed as interrupted is due for a scheduled save, which
    // the store holds.
    let gate = crashed.hold_saves();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(gate.waiting(), 1);
    let before = restored.transcript();

    let editing = spawn_edit(&restored, edit_of(&restored, "A", "op1", "write it twice"));
    tokio::time::sleep(Duration::from_secs(2)).await;

    // The edit waits behind that save, and shows nothing.
    assert_eq!(gate.waiting(), 1);
    assert_eq!(restored.transcript(), before);
    gate.release();
    let receipt = editing.await.unwrap().unwrap();
    // Committed after the earlier save, the replacement is what the store
    // holds, before its turn writes anything.
    let stored = crashed.checkpoint(&root()).unwrap();
    assert_eq!(kinds(&stored.transcript), ["user"]);
    assert_eq!(stored.transcript, restored.transcript().blocks);
    let _ = release.send(());
    restored.settled().await;
    let stored = crashed.checkpoint(&root()).unwrap();
    assert_eq!(kinds(&stored.transcript), ["user", "text", "response"]);
    assert_eq!(stored.transcript, restored.transcript().blocks);
    assert_eq!(stored.state.edits, [receipt]);
}

#[tokio::test(flavor = "local")]
async fn an_accepted_edit_stays_accepted_when_its_turn_fails_and_neither_a_repeat_nor_resume_runs_its_tool_again()
 {
    let provider = ScriptedRuntime::new([
        said("answer A"),
        Turn::Events(vec![
            event::tool_call("effect-1", "effect", json!({})),
            event::response(1, 1),
        ]),
        Turn::Events(vec![event::error("the continuation failed", None)]),
        said("recovered"),
    ]);
    let store = MemoryTreeStore::new();
    let runs = Rc::new(Cell::new(0));
    let effect = tool("effect", {
        let runs = runs.clone();
        move |_| {
            runs.set(runs.get() + 1);
            Box::pin(async { Ok(output("permanent result")) })
        }
    });
    let session = start(&provider, vec![effect], &store, SessionConfig::default()).await;
    session
        .send(text("A"), turn("A"))
        .unwrap()
        .await
        .unwrap();
    let failures = Rc::new(RefCell::new(Vec::new()));
    let _listener = session.subscribe({
        let failures = failures.clone();
        move |event| {
            if let SessionEvent::Error { report } = event {
                failures.borrow_mut().push(report.message.clone());
            }
        }
    });
    let submission = edit_of(&session, "A", "op1", "A2");

    let receipt = session.edit_and_send(submission.clone()).await.unwrap();
    session.settled().await;

    // The turn failed after its tool ran: the replacement and its receipt
    // stay, and the failure is reported.
    assert_eq!(runs.get(), 1);
    assert_eq!(*failures.borrow(), ["the continuation failed"]);
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        ["user", "tool_call:completed", "response", "error"]
    );
    assert!(matches!(&blocks[0], Block::User(user) if user.turn_id == receipt.turn_id));
    assert_eq!(
        store.checkpoint(&root()).unwrap().state.edits,
        [receipt.clone()]
    );
    // The same request again is answered from its receipt: no rewrite, no
    // request.
    let version = session.transcript().version;
    assert_eq!(session.edit_and_send(submission).await, Ok(receipt));
    assert_eq!(session.transcript().version, version);
    assert_eq!(provider.requests().len(), 3);
    // Resume continues after the tool's result and does not run it again.
    session.resume().unwrap().await.unwrap();
    assert_eq!(runs.get(), 1);
    let request = provider.requests().pop().unwrap();
    assert_eq!(
        item_kinds(&request.items),
        ["user_message", "tool_use", "tool_result", "user_message"]
    );
    assert_eq!(
        request.items[3],
        InferenceItem::UserMessage {
            content: text(RESUME_TEXT),
        }
    );
}

#[tokio::test(flavor = "local")]
async fn an_edit_of_the_first_a_middle_or_the_last_message_keeps_exactly_the_blocks_before_it() {
    let (steered, release) = gated_turn(vec![event::text("answer B1"), event::response(1, 1)]);
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("note-1", "note", json!({})),
            event::response(1, 1),
        ]),
        said("answer A"),
        steered,
        said("answer B2"),
        said("answer C"),
        // One answer for each replacement, in the order below.
        said("replaced A"),
        said("replaced B"),
        said("replaced C"),
    ]);
    let store = MemoryTreeStore::new();
    let note = tool("note", |_| Box::pin(async { Ok(output("noted")) }));
    let written = start(&provider, vec![note.clone()], &store, SessionConfig::default()).await;
    written
        .send(text("A"), turn("A"))
        .unwrap()
        .await
        .unwrap();
    let running = written.send(text("B"), turn("B")).unwrap();
    until(|| provider.requests().len() == 3).await;
    written
        .steer(text("mind the tests"), BlockId::try_from("s1").unwrap())
        .unwrap();
    let _ = release.send(());
    running.await.unwrap();
    written
        .send(text("C"), turn("C"))
        .unwrap()
        .await
        .unwrap();
    let before = written.transcript().blocks;
    assert_eq!(
        kinds(&before),
        [
            "user",
            "tool_call:completed",
            "response",
            "text",
            "response",
            "user",
            "text",
            "response",
            "steer",
            "text",
            "response",
            "user",
            "text",
            "response"
        ]
    );
    let question = |message: &str| InferenceItem::UserMessage {
        content: text(message),
    };
    let answer = |message: &str| InferenceItem::AssistantText {
        model_id: "test-model".into(),
        text: message.into(),
    };
    let turn_a = vec![
        question("A"),
        InferenceItem::ToolUse {
            model_id: "test-model".into(),
            tool_use_id: "note-1".into(),
            tool_name: "note".into(),
            input: json!({}),
        },
        InferenceItem::ToolResult {
            tool_use_id: "note-1".into(),
            output: texts(&["noted"]),
            is_error: false,
        },
        answer("answer A"),
    ];
    let turn_b = vec![
        question("B"),
        answer("answer B1"),
        InferenceItem::UserSteer {
            content: text("mind the tests"),
        },
        answer("answer B2"),
    ];

    for (edited, index, kept) in [
        ("A", 0, Vec::new()),
        ("B", 5, turn_a.clone()),
        ("C", 11, [turn_a.clone(), turn_b.clone()].concat()),
    ] {
        let copy = store.copy();
        let (session, _) = restore_session(
            copy.checkpoint(&root()).unwrap(),
            &copy,
            &provider,
            test_runtime(vec![note.clone()]),
            Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
        );
        let (_subscription, patches) = published(&session);
        let replacement = format!("{edited}2");

        let receipt = session
            .edit_and_send(edit_of(&session, edited, "op1", &replacement))
            .await
            .unwrap();
        session.settled().await;

        // The blocks before the message stay as they were; the message and
        // everything after it are one replacement turn.
        let blocks = session.transcript().blocks;
        assert_eq!(blocks[..index], before[..index], "editing {edited}");
        assert_eq!(kinds(&blocks[index..]), ["user", "text", "response"]);
        let Block::User(user) = &blocks[index] else {
            unreachable!();
        };
        assert_eq!(user.content, text(&replacement));
        assert_eq!(user.turn_id, receipt.turn_id);
        assert!(
            matches!(patches.borrow().first(), Some(TranscriptPatch::Replace { value }) if value[..] == blocks[..=index])
        );
        assert_eq!(copy.checkpoint(&root()).unwrap().transcript, blocks);
        // What the replacement asks: the kept turns, then the message.
        let expected = [kept, vec![question(&replacement)]].concat();
        assert_eq!(
            provider.requests().last().unwrap().items.as_ref(),
            expected.as_slice(),
            "editing {edited}"
        );
    }
}
