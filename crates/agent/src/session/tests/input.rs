//! Input that waits for a boundary (`runtime.md` § Input, § Yield wakeups,
//! `subagents.md` § Communication): human steers, agent messages and yield
//! wakeups.

use demi_core::{
    AgentMessage, AgentMessageEvent, BlockId, CompletionOutcome, PendingSteer, UserContentBlock,
    WakeupPlacement,
};

use super::*;
use crate::{
    store::ScheduledWakeup,
    testing::TokioClock,
    transcript::{WAKEUP_TEXT, agent_message_envelope},
};

fn completion() -> AgentMessage {
    AgentMessage {
        id: BlockId::try_from("subagent:child:1").unwrap(),
        event: AgentMessageEvent::Completion {
            outcome: CompletionOutcome::Completed,
        },
        ..agent_message("done")
    }
}

fn steer_id(id: &str) -> BlockId {
    BlockId::try_from(id).unwrap()
}

/// The texts of a request's steers.
fn steers(items: &[InferenceItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            InferenceItem::UserSteer { content } => match content.as_slice() {
                [UserContentBlock::Text { text }] => Some(text.clone()),
                other => panic!("a steer of more than one text: {other:?}"),
            },
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "local")]
async fn a_steer_during_a_tool_reaches_the_next_request_and_a_withdrawn_one_causes_none() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "hold", json!({})),
            event::response(1, 1),
        ]),
        Turn::Events(vec![event::text("skipping it"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let (hold, releases, started) = gated_tool("hold");
    let session = start(&provider, vec![hold], &store, SessionConfig::default()).await;
    let published = Rc::new(RefCell::new(Vec::new()));
    let _listener = session.subscribe({
        let published = published.clone();
        move |event| {
            if let SessionEvent::PendingSteersChanged { pending_steers } = event {
                let ids: Vec<String> = pending_steers
                    .iter()
                    .map(|steer| steer.id.to_string())
                    .collect();
                published.borrow_mut().push(ids);
            }
        }
    });
    let running = session.send(text("run the tests"), turn("t1")).unwrap();
    started.await.unwrap();

    session
        .steer(text("skip the e2e suite"), steer_id("s1"))
        .unwrap();
    session.steer(text("never mind"), steer_id("s2")).unwrap();
    assert!(session.cancel_pending_steer(&steer_id("s2")));
    assert!(!session.cancel_pending_steer(&steer_id("s2")));
    assert_eq!(
        session.pending_steers(),
        [PendingSteer {
            id: steer_id("s1"),
            turn_id: turn("t1"),
            model: test_model(),
            content: text("skip the e2e suite"),
        }]
    );
    let _ = releases.borrow_mut().remove(0).send(());
    running.await.unwrap();

    let requests = provider.requests();
    // No earlier request carries the steer, and the withdrawn one caused no
    // request of its own.
    assert_eq!(requests.len(), 2);
    assert!(steers(&requests[0].items).is_empty());
    assert_eq!(steers(&requests[1].items), ["skip the e2e suite"]);
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "tool_call:completed",
            "response",
            "steer",
            "text",
            "response"
        ]
    );
    assert_eq!(blocks[3].id(), &steer_id("s1"));
    assert_eq!(
        *published.borrow(),
        [vec!["s1"], vec!["s1", "s2"], vec!["s1"], vec![]]
    );
}

#[tokio::test(flavor = "local")]
async fn input_that_arrives_during_the_last_stream_asks_the_provider_once_more() {
    let (answer, release) = gated_turn(vec![event::text("done"), event::response(1, 1)]);
    let provider = ScriptedRuntime::new([
        answer,
        Turn::Events(vec![event::text("noted"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let running = session.send(text("go"), turn("t1")).unwrap();
    until(|| provider.requests().len() == 1).await;

    session
        .steer(text("and add a test"), steer_id("s1"))
        .unwrap();
    let _ = release.send(());
    running.await.unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(steers(&requests[1].items), ["and add a test"]);
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "text", "response", "steer", "text", "response"]
    );
}

#[tokio::test(flavor = "local")]
async fn a_steer_during_a_stream_that_calls_a_tool_is_written_before_the_tool_runs() {
    let (call, release) = gated_turn(vec![
        event::tool_call("call-1", "hold", json!({})),
        event::response(1, 1),
    ]);
    let provider = ScriptedRuntime::new([
        call,
        Turn::Events(vec![event::text("continued"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let (hold, releases, started) = gated_tool("hold");
    let session = start(&provider, vec![hold], &store, SessionConfig::default()).await;
    let running = session.send(text("use the tool"), turn("t1")).unwrap();
    until(|| provider.requests().len() == 1).await;

    session
        .steer(text("arrived during the stream"), steer_id("s1"))
        .unwrap();
    let _ = release.send(());
    started.await.unwrap();

    // The end of the stream is a boundary: the steer is history, and no
    // longer pending, while the tool runs.
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "tool_call:executing", "response", "steer"]
    );
    assert!(session.pending_steers().is_empty());
    let _ = releases.borrow_mut().remove(0).send(());
    running.await.unwrap();
    let request = &provider.requests()[1];
    assert_eq!(
        item_kinds(&request.items),
        ["user_message", "tool_use", "tool_result", "user_steer"]
    );
    assert_eq!(steers(&request.items), ["arrived during the stream"]);
}

#[tokio::test(flavor = "local")]
async fn a_stop_writes_the_pending_steers_before_its_marker_and_a_steer_needs_a_running_turn() {
    let provider = ScriptedRuntime::new([Turn::pending()]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    assert_eq!(
        session.steer(text("too early"), steer_id("s0")),
        Err(SteerError::NotRunning)
    );
    let running = session.send(text("go"), turn("t1")).unwrap();
    until(|| provider.requests().len() == 1).await;
    session.steer(text("keep this"), steer_id("s1")).unwrap();

    session.abort().await;

    assert_eq!(running.await, Ok(ActionEnd::Aborted));
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "steer", "abort"]
    );
    assert!(session.pending_steers().is_empty());
    assert_eq!(
        session.steer(text("too late"), steer_id("s2")),
        Err(SteerError::NotRunning)
    );
}

#[tokio::test(flavor = "local")]
async fn a_queued_message_becomes_a_steer_of_the_running_turn() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "hold", json!({})),
            event::response(1, 1),
        ]),
        Turn::Events(vec![event::text("both done"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let (hold, releases, started) = gated_tool("hold");
    let session = start(&provider, vec![hold], &store, SessionConfig::default()).await;
    let running = session.send(text("first"), turn("t1")).unwrap();
    started.await.unwrap();
    let queued = session.send(text("second"), turn("t2")).unwrap();

    assert_eq!(
        session.steer_queued_message(&turn("missing"), steer_id("s0")),
        Ok(false)
    );
    assert_eq!(
        session.steer_queued_message(&turn("t2"), steer_id("s1")),
        Ok(true)
    );

    assert_eq!(queued.await, Ok(ActionEnd::Dropped));
    assert!(session.queued_messages().is_empty());
    assert_eq!(session.pending_steers()[0].content, text("second"));
    let _ = releases.borrow_mut().remove(0).send(());
    running.await.unwrap();
    assert_eq!(steers(&provider.requests()[1].items), ["second"]);
}

#[tokio::test(flavor = "local")]
async fn agent_messages_that_arrive_during_a_tool_are_saved_and_enter_the_turn_in_order() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "hold", json!({})),
            event::response(1, 1),
        ]),
        Turn::Events(vec![
            event::text("finished with both"),
            event::response(1, 1),
        ]),
    ]);
    let store = MemoryTreeStore::new();
    let (hold, releases, started) = gated_tool("hold");
    let session = start(&provider, vec![hold], &store, SessionConfig::default()).await;
    let running = session.send(text("implement it"), turn("t1")).unwrap();
    started.await.unwrap();

    session
        .accept_agent_message(agent_message("update"))
        .await
        .unwrap();
    session.accept_agent_message(completion()).await.unwrap();

    // Each is durable once admitted, and neither is a queued message or a
    // pending steer the user could withdraw.
    let waiting: Vec<String> = store
        .checkpoint(&root())
        .unwrap()
        .state
        .agent_inputs
        .iter()
        .map(|input| input.message.id.to_string())
        .collect();
    assert_eq!(waiting, ["update", "subagent:child:1"]);
    assert!(session.queued_messages().is_empty());
    assert!(session.pending_steers().is_empty());
    assert!(!session.cancel_pending_steer(&steer_id("update")));
    let _ = releases.borrow_mut().remove(0).send(());
    running.await.unwrap();

    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "tool_call:completed",
            "response",
            "agent_message",
            "agent_message",
            "text",
            "response"
        ]
    );
    let request = &provider.requests()[1];
    assert_eq!(
        steers(&request.items),
        [
            agent_message_envelope(&agent_message("update")),
            agent_message_envelope(&completion())
        ]
    );
    assert!(
        store
            .checkpoint(&root())
            .unwrap()
            .state
            .agent_inputs
            .is_empty()
    );
}

#[tokio::test(flavor = "local")]
async fn agent_messages_to_an_idle_session_open_one_continuation_and_a_repeat_wakes_nothing() {
    let provider = ScriptedRuntime::new([Turn::Events(vec![
        event::text("combined result"),
        event::response(1, 1),
    ])]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;

    let (one, two) = tokio::join!(
        session.accept_agent_message(agent_message("one")),
        session.accept_agent_message(agent_message("two"))
    );
    one.unwrap();
    two.unwrap();
    session.settled().await;
    session
        .accept_agent_message(agent_message("one"))
        .await
        .unwrap();
    session.settled().await;

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(item_kinds(&requests[0].items), ["user_steer", "user_steer"]);
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["agent_message", "agent_message", "text", "response"]
    );
    assert!(session.queued_messages().is_empty());
}

#[tokio::test(flavor = "local")]
async fn retry_after_a_failed_continuation_reruns_it_from_its_message_and_keeps_the_turn_before() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![event::text("task answer"), event::response(1, 1)]),
        Turn::Events(vec![event::error("the vendor failed", None)]),
        Turn::Events(vec![event::text("read the result"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    session
        .send(text("the task"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    session
        .accept_agent_message(agent_message("result"))
        .await
        .unwrap();
    session.settled().await;
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "text", "response", "agent_message", "error"]
    );

    session.retry().unwrap().await.unwrap();

    // The message opened the failed turn: the retry keeps it and the
    // finished turn before it, and asks again with both.
    assert_eq!(
        kinds(&session.transcript().blocks),
        [
            "user",
            "text",
            "response",
            "agent_message",
            "text",
            "response"
        ]
    );
    let requests = provider.requests();
    assert_eq!(
        requests[2].items.as_ref(),
        [
            InferenceItem::UserMessage {
                content: text("the task"),
            },
            InferenceItem::AssistantText {
                model_id: "test-model".into(),
                text: "task answer".into(),
            },
            InferenceItem::UserSteer {
                content: text(&agent_message_envelope(&agent_message("result"))),
            },
        ]
    );
    assert_eq!(requests[2].turn_id, requests[1].turn_id);
}

#[tokio::test(flavor = "local")]
async fn a_message_that_came_after_the_users_stop_waits_for_the_users_next_action() {
    let provider = ScriptedRuntime::new([Turn::pending()]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let running = session.send(text("start work"), turn("t1")).unwrap();
    until(|| provider.requests().len() == 1).await;
    session
        .accept_agent_message(agent_message("unread"))
        .await
        .unwrap();
    session.abort().await;
    running.await.unwrap();

    session
        .accept_agent_message(agent_message("late"))
        .await
        .unwrap();

    assert_eq!(kinds(&session.transcript().blocks), ["user", "abort"]);
    let checkpoint = store.checkpoint(&root()).unwrap();
    assert_eq!(checkpoint.state.agent_inputs.len(), 2);
    let later = ScriptedRuntime::new([Turn::Events(vec![
        event::text("continued by the user"),
        event::response(1, 1),
    ])]);
    let (restored, _) = restore_session(
        checkpoint,
        &store,
        &later,
        test_runtime(Vec::new()),
        Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
    );
    restored.wake();
    restored.settled().await;
    assert!(later.requests().is_empty());
    restored.resume().unwrap().await.unwrap();
    let requests = later.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        steers(&requests[0].items),
        [
            agent_message_envelope(&agent_message("unread")),
            agent_message_envelope(&agent_message("late"))
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn a_message_admitted_while_the_action_finishes_opens_the_next_continuation() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![event::text("first response"), event::response(1, 1)]),
        Turn::Events(vec![event::text("receipt consumed"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let (release, released) = oneshot::channel::<()>();
    let gated = Rc::new(GatedStore {
        inner: store.session_store(&root()),
        release: RefCell::new(Some(released)),
    });
    let deps = SessionDeps {
        runtime: Rc::new(test_runtime(Vec::new())),
        store: gated.clone(),
        ids: Rc::new(SequentialIds::new("id")),
        clock: Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
        config: SessionConfig::default(),
    };
    let init = SessionInit {
        id: root(),
        cwd: "/workspace".to_owned(),
        model: test_model(),
        runtime: Box::new(provider.clone()),
    };
    let session = AgentSession::create(init, deps);
    store
        .create_node(
            NodeRecord::root(root(), Timestamp::UNIX_EPOCH),
            session.first_checkpoint(),
        )
        .await
        .unwrap();
    let running = session.send(text("work"), turn("t1")).unwrap();
    // The action's own save waits in the store: the action is finishing.
    until(|| gated.release.borrow().is_none()).await;

    let admitted = session.accept_agent_message(agent_message("race"));
    let _ = release.send(());
    admitted.await.unwrap();
    running.await.unwrap();
    session.settled().await;

    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "text",
            "response",
            "agent_message",
            "text",
            "response"
        ]
    );
}

/// A store whose first save waits until the test lets it go on.
struct GatedStore {
    inner: Rc<dyn SessionStore>,
    release: RefCell<Option<oneshot::Receiver<()>>>,
}

impl SessionStore for GatedStore {
    fn save<'a>(
        &'a self,
        update: crate::store::CheckpointUpdate,
        guard: &'a crate::store::CommitGuard,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        let wait = self.release.borrow_mut().take();
        Box::pin(async move {
            if let Some(wait) = wait {
                let _ = wait.await;
            }
            self.inner.save(update, guard).await
        })
    }

    fn load(&self) -> LocalBoxFuture<'_, Result<Option<crate::store::Checkpoint>, StoreError>> {
        self.inner.load()
    }
}

#[tokio::test(flavor = "local")]
async fn an_agent_message_is_refused_for_another_recipient_or_other_content_under_its_id() {
    let provider = ScriptedRuntime::new([Turn::Events(vec![
        event::text("read it"),
        event::response(1, 1),
    ])]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;

    let elsewhere = AgentMessage {
        recipient_id: NodeId::try_from("elsewhere").unwrap(),
        ..agent_message("m1")
    };
    let empty = AgentMessage {
        content: "  ".into(),
        ..agent_message("m2")
    };
    assert_eq!(
        session.accept_agent_message(elsewhere).await,
        Err(AgentMessageError::Recipient)
    );
    assert!(matches!(
        session.accept_agent_message(empty).await,
        Err(AgentMessageError::Invalid(_))
    ));
    session
        .accept_agent_message(agent_message("m3"))
        .await
        .unwrap();
    session.settled().await;
    let other = AgentMessage {
        content: "something else".into(),
        ..agent_message("m3")
    };
    assert_eq!(
        session.accept_agent_message(other).await,
        Err(AgentMessageError::DifferentContent)
    );
    assert_eq!(provider.requests().len(), 1);
}

#[tokio::test(flavor = "local")]
async fn a_restored_message_wakes_the_session_once_and_a_written_one_is_never_delivered_again() {
    let provider = ScriptedRuntime::new([Turn::Events(vec![
        event::text("original answer"),
        event::response(1, 1),
    ])]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    session
        .send(text("task"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    drop(session);
    let mut checkpoint = store.checkpoint(&root()).unwrap();
    checkpoint.state.agent_inputs = vec![crate::store::PendingAgentInput {
        turn_id: turn("t1"),
        model: test_model(),
        message: agent_message("restored"),
    }];
    let later = ScriptedRuntime::new([Turn::Events(vec![
        event::text("recovered"),
        event::response(1, 1),
    ])]);
    let clock = Arc::new(FixedClock(Timestamp::UNIX_EPOCH));

    let (restored, continuation) = restore_session(
        checkpoint,
        &store,
        &later,
        test_runtime(Vec::new()),
        clock.clone(),
    );
    assert!(!continuation.interrupted);
    restored.wake();
    restored.settled().await;
    restored
        .accept_agent_message(agent_message("restored"))
        .await
        .unwrap();
    restored.settled().await;
    drop(restored);
    let (again, _) = restore_session(
        store.checkpoint(&root()).unwrap(),
        &store,
        &later,
        test_runtime(Vec::new()),
        clock,
    );
    again.wake();
    again
        .accept_agent_message(agent_message("restored"))
        .await
        .unwrap();

    assert_eq!(later.requests().len(), 1);
    let written = again
        .transcript()
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::AgentMessage(_)))
        .count();
    assert_eq!(written, 1);
    assert!(again.is_settled());
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn yield_ends_the_turn_and_its_wakeup_opens_a_continuation_once_the_action_ended() {
    let provider = ScriptedRuntime::new([
        yield_call(120_000),
        Turn::Events(vec![
            event::text("checked the build"),
            event::response(1, 1),
        ]),
    ]);
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
        .send(text("build it"), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    // The turn ends after the round: one request.
    assert_eq!(provider.requests().len(), 1);
    let blocks = session.transcript().blocks;
    let Block::ToolCall(call) = &blocks[1] else {
        panic!("{blocks:?}");
    };
    let Some(demi_core::ToolView::YieldWakeup {
        wakeup_id,
        duration_ms: 120_000,
    }) = &call.view
    else {
        panic!("{call:?}");
    };
    assert_eq!(
        call.output,
        texts(&[&format!(
            "yield scheduled\nwakeupId: {wakeup_id}\ndurationMs: 120000"
        )])
    );
    // The wait started when the action ended, and the checkpoint keeps it.
    let scheduled = store.checkpoint(&root()).unwrap().state.wakeups;
    let [
        ScheduledWakeup {
            id,
            duration_ms: 120_000,
            due_at: Some(_),
        },
    ] = scheduled.as_slice()
    else {
        panic!("one wakeup is scheduled: {scheduled:?}");
    };
    assert_eq!(id, wakeup_id);
    tokio::time::sleep(Duration::from_secs(119)).await;
    assert_eq!(provider.requests().len(), 1);
    tokio::time::sleep(Duration::from_secs(2)).await;
    until(|| provider.requests().len() == 2).await;
    session.settled().await;

    let blocks = session.transcript().blocks;
    assert_eq!(kinds(&blocks[3..]), ["wakeup", "text", "response"]);
    assert!(
        matches!(&blocks[3], Block::Wakeup(wakeup) if wakeup.placement == WakeupPlacement::NewTurn)
    );
    let request = &provider.requests()[1];
    assert_eq!(
        request.items.last(),
        Some(&InferenceItem::UserMessage {
            content: text(WAKEUP_TEXT),
        })
    );
    assert!(store.checkpoint(&root()).unwrap().state.wakeups.is_empty());
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn stop_cancels_the_oldest_scheduled_wakeup_once_nothing_runs() {
    let provider = ScriptedRuntime::new([yield_call(60_000)]);
    let store = MemoryTreeStore::new();
    let session = start_at(
        &provider,
        test_runtime(vec![yield_tool()]),
        &store,
        SessionConfig::default(),
        Arc::new(TokioClock::new(Timestamp::UNIX_EPOCH)),
    )
    .await;
    session
        .send(text("wait a minute"), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    let stopped = session.abort().await;

    assert_eq!(
        stopped,
        AbortResult {
            target: Some(AbortTarget::PendingYieldWakeup),
            can_abort_again: false,
        }
    );
    session.flush().await.unwrap();
    assert!(store.checkpoint(&root()).unwrap().state.wakeups.is_empty());
    tokio::time::sleep(Duration::from_secs(120)).await;
    // A run past the script would panic.
    assert_eq!(provider.requests().len(), 1);
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_wakeup_that_fires_during_a_turn_joins_it_as_a_steer() {
    let (answer, release) = gated_turn(vec![event::text("still building"), event::response(1, 1)]);
    let provider = ScriptedRuntime::new([
        yield_call(1_000),
        answer,
        Turn::Events(vec![event::text("checked"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start_at(
        &provider,
        test_runtime(vec![yield_tool()]),
        &store,
        SessionConfig::default(),
        Arc::new(TokioClock::new(Timestamp::UNIX_EPOCH)),
    )
    .await;
    session
        .send(text("start the build"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    let running = session.send(text("anything else?"), turn("t2")).unwrap();
    until(|| provider.requests().len() == 2).await;

    tokio::time::sleep(Duration::from_secs(2)).await;
    // The fired wakeup waits for the boundary, never among the pending
    // steers.
    assert!(session.pending_steers().is_empty());
    let _ = release.send(());
    running.await.unwrap();

    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks[3..]),
        ["user", "text", "response", "wakeup", "text", "response"]
    );
    assert!(
        matches!(&blocks[6], Block::Wakeup(wakeup) if wakeup.placement == WakeupPlacement::Steer)
    );
    assert_eq!(steers(&provider.requests()[2].items), [WAKEUP_TEXT]);
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_wakeup_survives_dispose_and_one_due_meanwhile_fires_once_the_session_is_restored() {
    let provider = ScriptedRuntime::new([yield_call(60_000)]);
    let store = MemoryTreeStore::new();
    let clock: Arc<dyn demi_core::Clock> = Arc::new(TokioClock::new(Timestamp::UNIX_EPOCH));
    let session = start_at(
        &provider,
        test_runtime(vec![yield_tool()]),
        &store,
        SessionConfig::default(),
        clock.clone(),
    )
    .await;
    session
        .send(text("wait"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    session.dispose().await.unwrap();
    drop(session);
    tokio::time::sleep(Duration::from_secs(90)).await;

    let later = ScriptedRuntime::new([Turn::Events(vec![
        event::text("woke up"),
        event::response(1, 1),
    ])]);
    let (restored, _) = restore_session(
        store.checkpoint(&root()).unwrap(),
        &store,
        &later,
        test_runtime(Vec::new()),
        clock,
    );
    until(|| later.requests().len() == 1).await;
    restored.settled().await;

    assert_eq!(
        kinds(&restored.transcript().blocks[3..]),
        ["wakeup", "text", "response"]
    );
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_session_restored_after_an_interrupted_turn_holds_its_wakeups_and_messages_for_the_user()
{
    let provider = ScriptedRuntime::new([yield_call(1_000), Turn::pending()]);
    let store = MemoryTreeStore::new();
    let clock: Arc<dyn demi_core::Clock> = Arc::new(TokioClock::new(Timestamp::UNIX_EPOCH));
    let session = start_at(
        &provider,
        test_runtime(vec![yield_tool()]),
        &store,
        SessionConfig::default(),
        clock.clone(),
    )
    .await;
    session
        .send(text("wait"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    let _running = session.send(text("keep going"), turn("t2")).unwrap();
    until(|| provider.requests().len() == 2).await;
    session
        .accept_agent_message(agent_message("news"))
        .await
        .unwrap();
    // The process dies in the second turn.
    let crashed = store.copy();
    drop(session);
    tokio::time::sleep(Duration::from_secs(5)).await;

    let later = ScriptedRuntime::new([Turn::Events(vec![
        event::text("caught up"),
        event::response(1, 1),
    ])]);
    let (restored, continuation) = restore_session(
        crashed.checkpoint(&root()).unwrap(),
        &crashed,
        &later,
        test_runtime(Vec::new()),
        clock,
    );
    assert!(continuation.interrupted);
    restored.record_interruption();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(later.requests().is_empty());

    restored
        .send(text("what happened?"), turn("t3"))
        .unwrap()
        .await
        .unwrap();

    let request = &later.requests()[0];
    assert_eq!(
        steers(&request.items),
        [
            agent_message_envelope(&agent_message("news")),
            WAKEUP_TEXT.to_owned()
        ]
    );
}
