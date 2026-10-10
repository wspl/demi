//! Input that waits for a boundary (`runtime.md` § Input, § Send now,
//! § Command reports, `subagents.md` § Communication): human steers, also
//! sent now, agent messages and command reports.

use demi_agent_store::CommandInterval;
use demi_agent_transcript::testing::agent_message_envelope;
use demi_provider_common::testing::TokioClock;
use demi_agent_session::WindowEnd;
use demi_shared_types::{
    AgentMessage, AgentMessageEvent, BlockId, CompletionOutcome, PendingSteer, WakeupPlacement,
};
use tokio_util::sync::CancellationToken;

use super::*;

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
                [UserPart::Text(text)] => Some(text.clone()),
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

/// How a watching tool's window ended, and the token that would stop its
/// command.
type Watched = Rc<RefCell<Option<(WindowEnd, CancellationToken)>>>;

/// A tool that watches its command as a shell tool does, until input or a
/// send now ends its window, and answers what ended it.
fn watching_tool() -> ((String, Invoke), Watched, oneshot::Receiver<()>) {
    let (started, started_rx) = oneshot::channel();
    let started = Rc::new(RefCell::new(Some(started)));
    let watched: Watched = Rc::default();
    let ended = watched.clone();
    let invoke = tool("watch", move |call| {
        if let Some(started) = started.borrow_mut().take() {
            let _ = started.send(());
        }
        let ended = ended.clone();
        Box::pin(async move {
            let end = call.arrival.arrived().await;
            *ended.borrow_mut() = Some((end, call.cancel));
            Ok(output(match end {
                WindowEnd::SentNow => "moved to the background",
                WindowEnd::Input => "input arrived",
            }))
        })
    });
    (invoke, watched, started_rx)
}

/// Each request's tool results' texts, by call id.
fn sent_results(items: &[InferenceItem]) -> Vec<(String, Vec<ResultPart>)> {
    items
        .iter()
        .filter_map(|item| match item {
            InferenceItem::ToolResult {
                tool_use_id,
                output,
                ..
            } => Some((tool_use_id.clone(), output.clone())),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "local")]
async fn steer_now_returns_the_running_call_at_once_and_the_turn_goes_on_with_the_steer() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "watch", json!({})),
            event::tool_call("call-2", "later", json!({})),
            event::response(1, 1),
        ]),
        Turn::Events(vec![event::text("skipping them"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let (watch, watched, started) = watching_tool();
    let (later, later_runs) = counted("later", "ran");
    let session = start(&provider, vec![watch, later], &store, SessionConfig::default()).await;
    let running = session.send(text("run the tests"), turn("t1")).unwrap();
    started.await.unwrap();

    session.steer(text("skip the e2e tests"), steer_id("s1")).unwrap();
    session.steer_now(&steer_id("s1"));

    assert_eq!(running.await, Ok(ActionEnd::Completed));
    // The call returned for the send now, and nothing stops its command.
    let (end, command) = watched.borrow_mut().take().unwrap();
    assert_eq!(end, WindowEnd::SentNow);
    assert!(!command.is_cancelled());
    // The call after it never ran.
    assert_eq!(later_runs.get(), 0);
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "tool_call:completed",
            "tool_call:error",
            "response",
            "steer",
            "text",
            "response"
        ]
    );
    assert_eq!(
        tool_output(&blocks[2]),
        (
            ToolCallStatus::Error,
            texts(&["Tool call not run: the user sent a message"])
        )
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(steers(&requests[1].items), ["skip the e2e tests"]);
    assert_eq!(
        sent_results(&requests[1].items),
        [
            ("call-1".to_owned(), sent_texts(&["moved to the background"])),
            (
                "call-2".to_owned(),
                sent_texts(&["Tool call not run: the user sent a message"])
            ),
        ]
    );
    assert!(session.pending_steers().is_empty());
}

#[tokio::test(flavor = "local")]
async fn steer_now_cuts_the_stream_where_it_is_and_the_next_request_carries_the_steer() {
    let provider = ScriptedRuntime::new([
        partial_then_hang("Running the whole sui"),
        Turn::Events(vec![event::text("skipping them"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let running = session.send(text("run the tests"), turn("t1")).unwrap();
    until(|| session.transcript().blocks.len() == 2).await;

    // A steer that is no longer pending changes nothing.
    session.steer_now(&steer_id("missing"));
    session.steer(text("skip the e2e tests"), steer_id("s1")).unwrap();
    session.steer_now(&steer_id("s1"));

    assert_eq!(running.await, Ok(ActionEnd::Completed));
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "text", "steer", "text", "response"]
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    // What streamed stays, and the next request goes on from it.
    assert_eq!(
        item_kinds(&requests[1].items),
        ["user_message", "assistant_text", "user_steer"]
    );
    assert_eq!(steers(&requests[1].items), ["skip the e2e tests"]);
}

#[tokio::test(flavor = "local")]
async fn a_queued_message_sent_now_ends_the_turn_without_a_marker_and_runs_next() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "watch", json!({})),
            event::response(1, 1),
        ]),
        Turn::Events(vec![event::text("third first"), event::response(1, 1)]),
        Turn::Events(vec![event::text("then second"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let (watch, watched, started) = watching_tool();
    let session = start(&provider, vec![watch], &store, SessionConfig::default()).await;
    let running = session.send(text("first"), turn("t1")).unwrap();
    started.await.unwrap();
    let second = session.send(text("second"), turn("t2")).unwrap();
    let third = session.send(text("third"), turn("t3")).unwrap();
    session.steer(text("pending"), steer_id("s1")).unwrap();

    assert!(!session.send_queued_message(&turn("missing")));
    assert!(session.send_queued_message(&turn("t3")));

    assert_eq!(running.await, Ok(ActionEnd::Completed));
    assert_eq!(third.await, Ok(ActionEnd::Completed));
    assert_eq!(second.await, Ok(ActionEnd::Completed));
    let (end, command) = watched.borrow_mut().take().unwrap();
    assert_eq!(end, WindowEnd::SentNow);
    assert!(!command.is_cancelled());
    // The turn ended at the boundary with the pending steer written, and no
    // stopped marker; the message sent now ran before the one it passed.
    assert_eq!(
        kinds(&session.transcript().blocks),
        [
            "user",
            "tool_call:completed",
            "response",
            "steer",
            "user",
            "text",
            "response",
            "user",
            "text",
            "response"
        ]
    );
    let users: Vec<String> = session
        .transcript()
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::User(user) => Some(user.turn_id.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(users, ["t1", "t3", "t2"]);
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
                content: sent_text("the task"),
            },
            InferenceItem::AssistantText {
                model_id: "test-model".into(),
                text: "task answer".into(),
            },
            InferenceItem::UserSteer {
                content: sent_text(&agent_message_envelope(&agent_message("result"))),
            },
        ]
    );
    assert_eq!(requests[2].turn_id, requests[1].turn_id);
}

// One session, four scripted requests and a minute of paused time: a few
// milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_stop_writes_the_waiting_message_before_its_marker_and_holds_nothing_afterwards() {
    let provider = ScriptedRuntime::new([
        Turn::Events(background_call("7", None, "Build")),
        Turn::Events(vec![event::text("building"), event::response(1, 1)]),
        Turn::pending(),
        Turn::Events(vec![event::text("read it"), event::response(1, 1)]),
        Turn::Events(vec![event::text("checked"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let runtime = test_runtime(Vec::new());
    let commands = runtime.commands.clone();
    let runtime = TestRuntime {
        tools: vec![background_tool(&commands)],
        ..runtime
    };
    let session = start_at(
        &provider,
        runtime,
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
    until(|| provider.requests().len() == 3).await;
    session
        .accept_agent_message(agent_message("unread"))
        .await
        .unwrap();

    session.abort().await;
    running.await.unwrap();

    // The message waiting for the stopped turn's boundary is written into
    // it, before the marker, as a pending steer is.
    assert_eq!(
        kinds(&session.transcript().blocks[5..]),
        ["user", "agent_message", "abort"]
    );
    // A message that comes later wakes the session as usual.
    session
        .accept_agent_message(agent_message("late"))
        .await
        .unwrap();
    session.settled().await;
    let requests = provider.requests();
    assert_eq!(requests.len(), 4);
    // The stopped turn's message is history by now.
    assert_eq!(
        steers(&requests[3].items),
        [
            agent_message_envelope(&agent_message("unread")),
            agent_message_envelope(&agent_message("late"))
        ]
    );
    // So does the report of the command the first turn left running, once
    // it ends.
    commands.end("7");
    until(|| provider.requests().len() == 5).await;
    session.settled().await;
    assert_eq!(
        provider.requests()[4].items.last(),
        Some(&InferenceItem::UserMessage {
            content: sent_text("Build: ended"),
        })
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
    fn save(
        &self,
        update: demi_agent_store::CheckpointUpdate,
    ) -> LocalBoxFuture<'_, Result<(), StoreError>> {
        let wait = self.release.borrow_mut().take();
        Box::pin(async move {
            if let Some(wait) = wait {
                let _ = wait.await;
            }
            self.inner.save(update).await
        })
    }

    fn load(&self) -> LocalBoxFuture<'_, Result<Option<demi_agent_store::Checkpoint>, StoreError>> {
        self.inner.load()
    }

    fn blobs(&self) -> &dyn demi_agent_store::media::BlobStore {
        self.inner.blobs()
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
    checkpoint.state.agent_inputs = vec![demi_agent_store::PendingAgentInput {
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

/// A session whose `work` tool leaves its commands running, and the
/// commands, which the test ends.
async fn reporting_session(
    provider: &ScriptedRuntime,
    store: &Rc<MemoryTreeStore>,
) -> (AgentSession, Rc<TestCommands>) {
    let runtime = test_runtime(Vec::new());
    let commands = runtime.commands.clone();
    let runtime = TestRuntime {
        tools: vec![background_tool(&commands)],
        ..runtime
    };
    let session = start_at(
        provider,
        runtime,
        store,
        SessionConfig::default(),
        Arc::new(TokioClock::new(Timestamp::UNIX_EPOCH)),
    )
    .await;
    (session, commands)
}

fn answer(text: &str) -> Turn {
    Turn::Events(vec![event::text(text), event::response(1, 1)])
}

// One session, five scripted requests and seventeen minutes of paused
// time: a few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_command_left_running_reports_every_interval_until_its_end_and_as_its_interval_changes() {
    let provider = ScriptedRuntime::new([
        Turn::Events(background_call("17", Some(300_000), "Run the suite")),
        answer("the suite runs"),
        answer("still waiting"),
        answer("still waiting"),
        answer("the suite is done"),
    ]);
    let store = MemoryTreeStore::new();
    let (session, commands) = reporting_session(&provider, &store).await;

    session
        .send(text("run the suite"), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    // The turn ended with its answer; the command runs on, and the
    // checkpoint keeps how often it reports.
    assert_eq!(provider.requests().len(), 2);
    let command = CommandId::try_from("17").unwrap();
    assert_eq!(
        store.checkpoint(&root()).unwrap().state.intervals,
        [CommandInterval {
            command_id: command.clone(),
            interval_ms: Some(300_000),
        }]
    );
    assert!(session.status().commands);
    tokio::time::sleep(Duration::from_secs(299)).await;
    assert_eq!(provider.requests().len(), 2);
    tokio::time::sleep(Duration::from_secs(2)).await;
    until(|| provider.requests().len() == 3).await;
    session.settled().await;
    assert_eq!(
        provider.requests()[2].items.last(),
        Some(&InferenceItem::UserMessage {
            content: sent_text("Run the suite: still running (1)"),
        })
    );
    let blocks = session.transcript().blocks;
    assert!(matches!(
        &blocks[blocks.len() - 3],
        Block::Wakeup(wakeup) if wakeup.placement == WakeupPlacement::NewTurn
    ));
    tokio::time::sleep(Duration::from_secs(300)).await;
    until(|| provider.requests().len() == 4).await;
    session.settled().await;

    // A resident command reports only its end.
    assert!(session.set_interval(&command, None));
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(provider.requests().len(), 4);
    commands.end("17");
    until(|| provider.requests().len() == 5).await;
    session.settled().await;
    assert_eq!(
        provider.requests()[4].items.last(),
        Some(&InferenceItem::UserMessage {
            content: sent_text("Run the suite: ended"),
        })
    );
    assert!(!session.status().commands);
    session.flush().await.unwrap();
    assert!(store.checkpoint(&root()).unwrap().state.intervals.is_empty());
    assert!(!session.set_interval(&command, Some(60_000)));
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn reports_that_arrive_during_a_turn_join_it_as_one_steer() {
    let (answer_gated, release) = gated_turn(vec![event::text("still building"), event::response(1, 1)]);
    let provider = ScriptedRuntime::new([
        Turn::Events(
            [
                background_call("17", None, "Build").remove(0),
                background_call("18", None, "Lint").remove(0),
                event::response(1, 1),
            ]
            .to_vec(),
        ),
        answer("both run"),
        answer_gated,
        answer("checked"),
    ]);
    let store = MemoryTreeStore::new();
    let (session, commands) = reporting_session(&provider, &store).await;
    session
        .send(text("build and lint"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    let running = session.send(text("anything else?"), turn("t2")).unwrap();
    until(|| provider.requests().len() == 3).await;

    commands.end("17");
    commands.end("18");
    until(|| session.status().input).await;
    // A report waits for the boundary, never among the pending steers.
    assert!(session.pending_steers().is_empty());
    let _ = release.send(());
    running.await.unwrap();

    let blocks = session.transcript().blocks;
    let wakeups: Vec<&Block> = blocks
        .iter()
        .filter(|block| matches!(block, Block::Wakeup(_)))
        .collect();
    assert!(matches!(
        wakeups.as_slice(),
        [Block::Wakeup(wakeup)] if wakeup.placement == WakeupPlacement::Steer
    ));
    assert_eq!(steers(&provider.requests()[3].items), ["Build: ended\n\nLint: ended"]);
}

// A report that arrived before dispose, and a command that still runs, are
// kept: the restored session tells the first and watches the second.
#[tokio::test(flavor = "local", start_paused = true)]
async fn waiting_reports_and_running_commands_survive_dispose() {
    let provider = ScriptedRuntime::new([
        Turn::Events(
            [
                background_call("17", None, "Build").remove(0),
                background_call("18", None, "Serve").remove(0),
                event::response(1, 1),
            ]
            .to_vec(),
        ),
        Turn::pending(),
    ]);
    let store = MemoryTreeStore::new();
    let (session, commands) = reporting_session(&provider, &store).await;
    let running = session.send(text("build and serve"), turn("t1")).unwrap();
    until(|| provider.requests().len() == 2).await;
    commands.end("17");
    until(|| session.status().input).await;
    session.dispose().await.unwrap();
    drop(running);
    drop(session);
    let state = store.checkpoint(&root()).unwrap().state;
    assert_eq!(state.reports, ["Build: ended"]);
    assert_eq!(
        state.intervals,
        [CommandInterval {
            command_id: CommandId::try_from("18").unwrap(),
            interval_ms: None,
        }]
    );

    let later = ScriptedRuntime::new([answer("resumed"), answer("served")]);
    let runtime = TestRuntime {
        commands: commands.clone(),
        ..test_runtime(Vec::new())
    };
    let (restored, continuation) = restore_session(
        store.checkpoint(&root()).unwrap(),
        &store,
        &later,
        runtime,
        Arc::new(TokioClock::new(Timestamp::UNIX_EPOCH)),
    );
    assert!(continuation.interrupted);
    restored.resume().unwrap().await.unwrap();
    assert_eq!(steers(&later.requests()[0].items), ["Build: ended"]);
    commands.end("18");
    until(|| later.requests().len() == 2).await;
    restored.settled().await;
    // The report names the command by the title of the call that started
    // it, as the restored transcript holds it.
    assert_eq!(
        later.requests()[1].items.last(),
        Some(&InferenceItem::UserMessage {
            content: sent_text("Serve: ended"),
        })
    );
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_session_restored_after_an_interrupted_turn_holds_its_reports_and_messages_for_the_user()
{
    let provider = ScriptedRuntime::new([
        Turn::Events(background_call("17", None, "Build")),
        answer("building"),
        Turn::pending(),
    ]);
    let store = MemoryTreeStore::new();
    let (session, commands) = reporting_session(&provider, &store).await;
    session
        .send(text("build"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    let _running = session.send(text("keep going"), turn("t2")).unwrap();
    until(|| provider.requests().len() == 3).await;
    session
        .accept_agent_message(agent_message("news"))
        .await
        .unwrap();
    commands.end("17");
    until(|| session.status().input && !session.status().commands).await;
    session.flush().await.unwrap();
    // The process dies in the second turn.
    let crashed = store.copy();
    drop(session);

    let later = ScriptedRuntime::new([answer("caught up")]);
    let clock: Arc<dyn demi_shared_types::Clock> = Arc::new(TokioClock::new(Timestamp::UNIX_EPOCH));
    let (restored, continuation) = restore_session(
        crashed.checkpoint(&root()).unwrap(),
        &crashed,
        &later,
        test_runtime(Vec::new()),
        clock.clone(),
    );
    assert!(continuation.interrupted);
    restored.record_interruption();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(later.requests().is_empty());
    // A second restart before the user acts: the checkpoint is idle now,
    // and its interruption record still holds the input.
    restored.dispose().await.unwrap();
    drop(restored);
    let (restored, continuation) = restore_session(
        crashed.checkpoint(&root()).unwrap(),
        &crashed,
        &later,
        test_runtime(Vec::new()),
        clock,
    );
    assert!(!continuation.interrupted);
    restored.wake();
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
            "Build: ended".to_owned()
        ]
    );
}
