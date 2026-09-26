//! The session's rules that only a session with tools shows: tool dispatch
//! and its save, stops and dispose while a tool runs, restore after a crash,
//! the queue's saves, model switches inside a turn, the stream's blocks and
//! the order of events. The server's scenario tests cover the frames.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use demi_agent_protocol::{AbortTarget, ModelSwitchApply, TranscriptPatch};
use demi_core::{
    AgentMessage, AgentMessageEvent, Block, BlockId, FailureSource, NodeId, OperationId, Sender,
    SessionPhase, Timestamp, ToolCallStatus, ToolResultContentBlock, TurnId, UserContentBlock,
};
use demi_gates::{ActivityGate, GateLease, Purpose};
use demi_provider::{
    ErrorCode, InferenceItem, ProviderEvent, ToolDefinition,
    testing::{FixedClock, ScriptedRuntime, Turn, event},
};
use futures_util::future::LocalBoxFuture;
use serde_json::json;
use tokio::{sync::oneshot, task::JoinHandle};

use super::*;
use crate::{
    store::{AgentTreeStore, EditReceipt, NodeRecord},
    testing::{MemoryTreeStore, SequentialIds, model_of, test_model, text},
};

mod compaction;
mod editing;
mod input;
mod recovery;

type Invoke =
    Rc<dyn Fn(ToolInvocation) -> LocalBoxFuture<'static, Result<ToolOutcome, ToolFailure>>>;

/// A node runtime whose tools the test defines.
struct TestRuntime {
    tools: Vec<(String, Invoke)>,
    admission: ActivityGate,
    /// While set, the preamble hook never answers.
    hanging_preamble: Rc<Cell<bool>>,
}

impl SessionRuntime for TestRuntime {
    fn harness_name(&self) -> &str {
        "test"
    }

    fn enter_action(&self) -> LocalBoxFuture<'_, GateLease> {
        Box::pin(self.admission.enter(Purpose::Demand))
    }

    fn reserve_edit(&self) -> LocalBoxFuture<'_, Result<Option<demi_gates::Reservation>, String>> {
        Box::pin(async { Ok(None) })
    }

    fn system_prompt(&self) -> LocalBoxFuture<'_, String> {
        Box::pin(async { "system prompt".to_owned() })
    }

    fn preamble(&self) -> LocalBoxFuture<'_, Option<String>> {
        if self.hanging_preamble.get() {
            return Box::pin(std::future::pending());
        }
        Box::pin(async { None })
    }

    fn context<'a>(&'a self, _seen: &'a [&'a str]) -> LocalBoxFuture<'a, Option<String>> {
        Box::pin(async { None })
    }

    fn tools(&self) -> Arc<[ToolDefinition]> {
        self.tools
            .iter()
            .map(|(name, _)| ToolDefinition {
                name: name.clone(),
                description: String::new(),
                input_schema: serde_json::Map::new(),
            })
            .collect()
    }

    fn invoke_tool(
        &self,
        call: ToolInvocation,
    ) -> LocalBoxFuture<'_, Result<ToolOutcome, ToolFailure>> {
        let (_, invoke) = self
            .tools
            .iter()
            .find(|(name, _)| *name == call.tool_name)
            .expect("the session invokes only the tools it was given");
        invoke(call)
    }
}

fn tool<F>(name: &str, invoke: F) -> (String, Invoke)
where
    F: Fn(ToolInvocation) -> LocalBoxFuture<'static, Result<ToolOutcome, ToolFailure>> + 'static,
{
    (name.to_owned(), Rc::new(invoke))
}

fn output(text: &str) -> ToolOutcome {
    ToolOutcome {
        output: vec![ToolResultContentBlock::Text {
            text: text.to_owned(),
        }],
        is_error: false,
        view: None,
        effect: None,
    }
}

/// A tool that answers `answer`, and how many times it ran.
fn counted(name: &str, answer: &'static str) -> ((String, Invoke), Rc<Cell<u32>>) {
    let runs = Rc::new(Cell::new(0));
    let invoke = tool(name, {
        let runs = runs.clone();
        move |_| {
            runs.set(runs.get() + 1);
            Box::pin(async move { Ok(output(answer)) })
        }
    });
    (invoke, runs)
}

/// What lets each waiting call of a gated tool finish, in call order.
type Releases = Rc<RefCell<Vec<oneshot::Sender<()>>>>;

/// A tool that tells the test it started, then waits until the test lets it
/// finish.
fn gated_tool(name: &str) -> ((String, Invoke), Releases, oneshot::Receiver<()>) {
    let (started, started_rx) = oneshot::channel();
    let started = Rc::new(RefCell::new(Some(started)));
    let releases: Releases = Rc::default();
    let gate = releases.clone();
    let invoke = tool(name, move |_| {
        if let Some(started) = started.borrow_mut().take() {
            let _ = started.send(());
        }
        let (release, released) = oneshot::channel();
        gate.borrow_mut().push(release);
        Box::pin(async move {
            let _ = released.await;
            Ok(output("finished"))
        })
    });
    (invoke, releases, started_rx)
}

/// A `yield` of `duration_ms`.
fn yield_tool() -> (String, Invoke) {
    tool("yield", |call| {
        let duration_ms = call.input["durationMs"]
            .as_u64()
            .and_then(|duration| u32::try_from(duration).ok())
            .expect("the test's yield names its duration");
        Box::pin(async move {
            Ok(ToolOutcome {
                effect: Some(ToolEffect::ScheduleYield { duration_ms }),
                ..output("")
            })
        })
    })
}

fn yield_call(duration_ms: u64) -> Turn {
    Turn::Events(vec![
        event::tool_call("yield-1", "yield", json!({ "durationMs": duration_ms })),
        event::response(1, 1),
    ])
}

/// An edit of the `user` block of the turn `turn` in the session's
/// transcript as it is now, to one text. Equal requests have equal
/// digests.
fn edit_of(
    session: &AgentSession,
    turn: &str,
    operation: &str,
    replacement: &str,
) -> EditSubmission {
    let snapshot = session.transcript();
    let target = snapshot
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::User(user) if user.turn_id.as_str() == turn => Some(user.id.clone()),
            _ => None,
        })
        .expect("the turn has a user block");
    EditSubmission {
        operation_id: OperationId::try_from(operation).unwrap(),
        target,
        version: snapshot.version,
        content: vec![EditContent::Content(UserContentBlock::Text {
            text: replacement.to_owned(),
        })],
        digest: format!("{operation}:{replacement}"),
    }
}

/// Submits `submission` in a task of its own, as a connection does.
fn spawn_edit(
    session: &AgentSession,
    submission: EditSubmission,
) -> JoinHandle<Result<EditReceipt, EditError>> {
    let session = session.clone();
    tokio::task::spawn_local(async move { session.edit_and_send(submission).await })
}

/// Lets the other tasks run until `done` holds.
async fn until(done: impl Fn() -> bool) {
    for _ in 0..1_000 {
        if done() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("the condition never held");
}

fn root() -> NodeId {
    NodeId::try_from("root").unwrap()
}

fn turn(id: &str) -> TurnId {
    TurnId::try_from(id).unwrap()
}

/// A session of the node `root` in `store`, whose first checkpoint the store
/// already holds.
async fn start(
    provider: &ScriptedRuntime,
    tools: Vec<(String, Invoke)>,
    store: &Rc<MemoryTreeStore>,
    config: SessionConfig,
) -> AgentSession {
    start_on(provider, test_runtime(tools), store, config).await
}

fn test_runtime(tools: Vec<(String, Invoke)>) -> TestRuntime {
    TestRuntime {
        tools,
        admission: ActivityGate::new(),
        hanging_preamble: Rc::default(),
    }
}

async fn start_on(
    provider: &ScriptedRuntime,
    runtime: TestRuntime,
    store: &Rc<MemoryTreeStore>,
    config: SessionConfig,
) -> AgentSession {
    start_at(
        provider,
        runtime,
        store,
        config,
        Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
    )
    .await
}

/// A session whose wall clock is `clock`.
async fn start_at(
    provider: &ScriptedRuntime,
    runtime: TestRuntime,
    store: &Rc<MemoryTreeStore>,
    config: SessionConfig,
    clock: Arc<dyn demi_core::Clock>,
) -> AgentSession {
    start_with(Box::new(provider.clone()), runtime, store, config, clock).await
}

/// A session whose provider runtime is `provider`.
async fn start_with(
    provider: Box<dyn ProviderRuntime>,
    runtime: TestRuntime,
    store: &Rc<MemoryTreeStore>,
    config: SessionConfig,
    clock: Arc<dyn demi_core::Clock>,
) -> AgentSession {
    let deps = SessionDeps {
        runtime: Rc::new(runtime),
        store: store.session_store(&root()),
        ids: Rc::new(SequentialIds::new("id")),
        clock,
        config,
    };
    let init = SessionInit {
        id: root(),
        cwd: "/workspace".to_owned(),
        model: test_model(),
        runtime: provider,
    };
    let session = AgentSession::create(init, deps);
    store
        .create_node(
            NodeRecord::root(root(), Timestamp::UNIX_EPOCH),
            session.first_checkpoint(),
        )
        .await
        .unwrap();
    session
}

/// Restores the node `root` from `checkpoint`, saving into `store`.
fn restore_session(
    checkpoint: crate::store::Checkpoint,
    store: &Rc<MemoryTreeStore>,
    provider: &ScriptedRuntime,
    runtime: TestRuntime,
    clock: Arc<dyn demi_core::Clock>,
) -> (AgentSession, Continuation) {
    restore_configured(
        checkpoint,
        store,
        provider,
        runtime,
        clock,
        SessionConfig::default(),
    )
}

/// Restores the node `root` from `checkpoint` with `config`, saving into
/// `store`.
fn restore_configured(
    checkpoint: crate::store::Checkpoint,
    store: &Rc<MemoryTreeStore>,
    provider: &ScriptedRuntime,
    runtime: TestRuntime,
    clock: Arc<dyn demi_core::Clock>,
    config: SessionConfig,
) -> (AgentSession, Continuation) {
    let deps = SessionDeps {
        runtime: Rc::new(runtime),
        store: store.session_store(&root()),
        ids: Rc::new(SequentialIds::new("restored")),
        clock,
        config,
    };
    AgentSession::restore(checkpoint, root(), Box::new(provider.clone()), deps).unwrap()
}

/// Which runtime of a [`NumberedRuntime`]'s family served each request, and
/// which closed, in order.
#[derive(Debug, Default)]
struct RuntimeLog {
    forks: usize,
    served: Vec<usize>,
    closed: Vec<usize>,
}

/// A runtime of one script with an identity: the first is number 0, and each
/// fork takes the next number. The family shares the script and the log,
/// and nothing else.
struct NumberedRuntime {
    script: ScriptedRuntime,
    number: usize,
    log: Rc<RefCell<RuntimeLog>>,
}

impl NumberedRuntime {
    fn first(script: &ScriptedRuntime) -> (Self, Rc<RefCell<RuntimeLog>>) {
        let log = Rc::new(RefCell::new(RuntimeLog::default()));
        let runtime = Self {
            script: script.clone(),
            number: 0,
            log: log.clone(),
        };
        (runtime, log)
    }
}

impl ProviderRuntime for NumberedRuntime {
    fn run(&mut self, request: demi_provider::InferenceRequest) -> demi_provider::ProviderRun<'_> {
        self.log.borrow_mut().served.push(self.number);
        self.script.run(request)
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        let mut log = self.log.borrow_mut();
        log.forks += 1;
        Box::new(Self {
            script: self.script.clone(),
            number: log.forks,
            log: self.log.clone(),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        self.log.borrow_mut().closed.push(self.number);
        self.script.close()
    }
}

/// A run that streams `partial` and then waits until it is stopped.
fn partial_then_hang(partial: &str) -> Turn {
    use futures_util::StreamExt;
    let delta = event::text(partial);
    Turn::Stream(Box::new(move |_| {
        futures_util::stream::iter([delta])
            .chain(futures_util::stream::pending())
            .boxed_local()
    }))
}

/// A message from the agent `child` of the tree to the root.
fn agent_message(id: &str) -> AgentMessage {
    AgentMessage {
        id: BlockId::try_from(id).unwrap(),
        sender: Sender {
            id: NodeId::try_from("child").unwrap(),
            description: "UI implementation".into(),
            round: 1,
        },
        recipient_id: root(),
        timestamp: Timestamp::UNIX_EPOCH,
        content: format!("Result {id}"),
        event: AgentMessageEvent::Message {},
    }
}

/// A run that waits until the test lets it go on, then plays `events`.
fn gated_turn(events: Vec<ProviderEvent>) -> (Turn, oneshot::Sender<()>) {
    use futures_util::StreamExt;
    let (release, released) = oneshot::channel::<()>();
    let turn = Turn::Stream(Box::new(move |_| {
        futures_util::stream::once(released)
            .flat_map(move |_| futures_util::stream::iter(events.clone()))
            .boxed_local()
    }));
    (turn, release)
}

/// Each block's type, and a tool call's status beside it.
fn kinds(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .map(|block| match block {
            Block::ToolCall(call) => format!("tool_call:{}", call.status),
            other => serde_json::to_value(other).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned(),
        })
        .collect()
}

fn item_kinds(items: &[InferenceItem]) -> Vec<&'static str> {
    items
        .iter()
        .map(|item| match item {
            InferenceItem::UserMessage { .. } => "user_message",
            InferenceItem::UserSteer { .. } => "user_steer",
            InferenceItem::AssistantText { .. } => "assistant_text",
            InferenceItem::AssistantThinking { .. } => "assistant_thinking",
            InferenceItem::AssistantRedactedThinking { .. } => "assistant_redacted_thinking",
            InferenceItem::ToolUse { .. } => "tool_use",
            InferenceItem::ToolResult { .. } => "tool_result",
        })
        .collect()
}

fn tool_output(block: &Block) -> (ToolCallStatus, Vec<ToolResultContentBlock>) {
    let Block::ToolCall(call) = block else {
        panic!("not a tool call: {block:?}");
    };
    (call.status, call.output.clone())
}

fn texts(output: &[&str]) -> Vec<ToolResultContentBlock> {
    output
        .iter()
        .map(|text| ToolResultContentBlock::Text {
            text: (*text).to_owned(),
        })
        .collect()
}

#[tokio::test(flavor = "local")]
async fn a_tool_runs_after_its_call_is_saved_and_the_turn_continues_with_its_result() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "echo", json!({ "value": "hello" })),
            event::response(10, 5),
        ]),
        Turn::Events(vec![event::text("done"), event::response(20, 5)]),
    ]);
    let store = MemoryTreeStore::new();
    let stored_when_run = Rc::new(RefCell::new(Vec::new()));
    let echo = tool("echo", {
        let store = store.clone();
        let stored_when_run = stored_when_run.clone();
        move |call| {
            *stored_when_run.borrow_mut() = kinds(&store.checkpoint(&root()).unwrap().transcript);
            Box::pin(async move { Ok(output(&call.input.to_string())) })
        }
    });
    let session = start(&provider, vec![echo], &store, SessionConfig::default()).await;

    let end = session
        .send(text("use the tool"), turn("t1"))
        .unwrap()
        .await;

    assert_eq!(end, Ok(ActionEnd::Completed));
    assert_eq!(
        *stored_when_run.borrow(),
        ["user", "tool_call:executing", "response"]
    );
    assert_eq!(
        kinds(&session.transcript().blocks),
        [
            "user",
            "tool_call:completed",
            "response",
            "text",
            "response"
        ]
    );
    let requests = provider.requests();
    assert_eq!(
        item_kinds(&requests[1].items),
        ["user_message", "tool_use", "tool_result"]
    );
    assert_eq!(
        requests[1].items[2],
        InferenceItem::ToolResult {
            tool_use_id: "call-1".into(),
            output: texts(&[r#"{"value":"hello"}"#]),
            is_error: false,
        }
    );
    assert_eq!(requests[0].turn_id, "t1");
    assert_eq!(requests[1].turn_id, "t1");
    assert_ne!(requests[0].request_id, requests[1].request_id);
}

#[tokio::test(flavor = "local")]
async fn a_turn_saves_at_each_dispatch_and_at_its_end_with_only_the_changed_rows() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "noop", json!({})),
            event::response(1, 1),
        ]),
        Turn::Events(vec![
            event::tool_call("call-2", "noop", json!({})),
            event::response(1, 1),
        ]),
        Turn::Events(vec![event::text("done"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let noop = tool("noop", |_| Box::pin(async { Ok(output("ok")) }));
    let config = SessionConfig {
        persist_interval: Duration::from_secs(60),
        ..SessionConfig::default()
    };
    let session = start(&provider, vec![noop], &store, config).await;

    session
        .send(text("run tools"), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    // The node's creation, one save before each round of tools, and the one
    // at the action's end, each with only the rows changed since.
    let saves = store.saves();
    let rows: Vec<Vec<(usize, String)>> = saves[1..]
        .iter()
        .map(|(_, update)| {
            update
                .changed_blocks
                .iter()
                .map(|(index, block)| (*index, kinds(std::slice::from_ref(block)).remove(0)))
                .collect()
        })
        .collect();
    let row = |index: usize, kind: &str| (index, kind.to_owned());
    assert_eq!(
        rows,
        [
            vec![
                row(0, "user"),
                row(1, "tool_call:executing"),
                row(2, "response")
            ],
            vec![
                row(1, "tool_call:completed"),
                row(3, "tool_call:executing"),
                row(4, "response")
            ],
            vec![
                row(3, "tool_call:completed"),
                row(5, "text"),
                row(6, "response")
            ],
        ]
    );
    let (_, last) = saves.last().unwrap();
    assert_eq!(
        (last.block_count, last.state.phase),
        (7, SessionPhase::Idle)
    );
}

#[tokio::test(flavor = "local")]
async fn a_failing_tool_and_an_unknown_tool_complete_their_calls_as_errors_and_the_turn_goes_on() {
    // The run ends after its tool calls without a response, as a CLI's
    // does when it needs the results to finish.
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "broken", json!({})),
            event::tool_call("call-2", "missing", json!({})),
        ]),
        Turn::Events(vec![event::text("recovered"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let broken = tool("broken", |_| {
        Box::pin(async { Err(ToolFailure("it broke".into())) })
    });
    let session = start(&provider, vec![broken], &store, SessionConfig::default()).await;

    session.send(text("go"), turn("t1")).unwrap().await.unwrap();

    let blocks = session.transcript().blocks;
    assert_eq!(
        tool_output(&blocks[1]),
        (ToolCallStatus::Error, texts(&["Tool failed: it broke"]))
    );
    assert_eq!(
        tool_output(&blocks[2]),
        (ToolCallStatus::Error, texts(&["Tool not found: missing"]))
    );
    assert_eq!(kinds(&blocks[3..]), ["text", "response"]);
}

#[tokio::test(flavor = "local")]
async fn stop_during_a_tool_drops_the_call_and_records_the_stop_before_it_answers() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "slow", json!({})),
            event::response(1, 1),
        ]),
        Turn::Events(vec![event::text("continued"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let (slow, _releases, started) = gated_tool("slow");
    let session = start(&provider, vec![slow], &store, SessionConfig::default()).await;
    let running = session.send(text("go"), turn("t1")).unwrap();
    started.await.unwrap();

    let stopped = session.abort().await;

    assert_eq!(
        stopped,
        AbortResult {
            target: Some(AbortTarget::ActiveTool),
            can_abort_again: false,
        }
    );
    // The record is in the transcript when `abort` answers.
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        ["user", "tool_call:error", "response", "abort"]
    );
    assert_eq!(
        tool_output(&blocks[1]).1,
        texts(&["Tool call aborted: slow"])
    );
    assert_eq!(running.await, Ok(ActionEnd::Aborted));
    assert_eq!(session.phase(), SessionPhase::Idle);
    // The next request carries the aborted call's result.
    session
        .send(text("go on"), turn("t2"))
        .unwrap()
        .await
        .unwrap();
    let request = &provider.requests()[1];
    assert_eq!(
        item_kinds(&request.items),
        ["user_message", "tool_use", "tool_result", "user_message"]
    );
    assert_eq!(
        request.items[2],
        InferenceItem::ToolResult {
            tool_use_id: "call-1".into(),
            output: texts(&["Tool call aborted: slow"]),
            is_error: true,
        }
    );
}

#[tokio::test(flavor = "local")]
async fn stop_while_a_hook_hangs_records_the_stop_without_waiting_for_the_hook() {
    let provider = ScriptedRuntime::new(Vec::new());
    let store = MemoryTreeStore::new();
    let runtime = TestRuntime {
        hanging_preamble: Rc::new(Cell::new(true)),
        ..test_runtime(Vec::new())
    };
    let session = start_on(&provider, runtime, &store, SessionConfig::default()).await;
    let running = session.send(text("go"), turn("t1")).unwrap();
    until(|| session.phase() == SessionPhase::Running).await;
    tokio::task::yield_now().await;

    let stopped = session.abort().await;

    assert_eq!(stopped.target, Some(AbortTarget::ActiveTurn));
    assert_eq!(running.await, Ok(ActionEnd::Aborted));
    // The message never wrote its turn; the stop leaves its marker.
    assert_eq!(kinds(&session.transcript().blocks), ["abort"]);
    assert!(provider.requests().is_empty());
}

#[tokio::test(flavor = "local")]
async fn dispose_during_a_tool_saves_the_interrupted_turn_and_keeps_the_queue() {
    let provider = ScriptedRuntime::new([Turn::Events(vec![
        event::tool_call("call-1", "slow", json!({})),
        event::response(1, 1),
    ])]);
    let store = MemoryTreeStore::new();
    let (slow, _releases, started) = gated_tool("slow");
    let session = start(&provider, vec![slow], &store, SessionConfig::default()).await;
    let running = session.send(text("go"), turn("t1")).unwrap();
    started.await.unwrap();
    let queued = session.send(text("later"), turn("t2")).unwrap();

    session.dispose().await.unwrap();

    assert_eq!(running.await, Ok(ActionEnd::Aborted));
    assert_eq!(queued.await, Ok(ActionEnd::Detached));
    let checkpoint = store.checkpoint(&root()).unwrap();
    assert_eq!(checkpoint.state.phase, SessionPhase::Running);
    assert_eq!(
        kinds(&checkpoint.transcript),
        ["user", "tool_call:error", "response", "error"]
    );
    assert_eq!(
        tool_output(&checkpoint.transcript[1]).1,
        texts(&["Tool call aborted: slow"])
    );
    let Block::Error(record) = &checkpoint.transcript[3] else {
        unreachable!()
    };
    assert_eq!(record.code.as_deref(), Some("interrupted"));
    assert_eq!(
        record.message,
        "The agent session was shut down while this turn was running."
    );
    let queue: Vec<&str> = checkpoint
        .state
        .queue
        .iter()
        .map(|message| message.id.as_str())
        .collect();
    assert_eq!(queue, ["t2"]);
    assert_eq!(provider.closes(), 1);
    assert_eq!(
        session.send(text("after"), turn("t3")).err(),
        Some(AdmissionError::Closed)
    );
}

#[tokio::test(flavor = "local")]
async fn restore_after_a_crash_during_a_tool_completes_the_call_as_interrupted_without_running_it()
{
    let provider = ScriptedRuntime::new([Turn::Events(vec![
        event::tool_call("call-1", "write_once", json!({ "path": "a.txt" })),
        event::response(1, 1),
    ])]);
    let store = MemoryTreeStore::new();
    let (write_once, _releases, started) = gated_tool("write_once");
    let session = start(
        &provider,
        vec![write_once],
        &store,
        SessionConfig::default(),
    )
    .await;
    let _running = session.send(text("write it"), turn("t1")).unwrap();
    started.await.unwrap();
    // The process dies here: what the store holds is all that is left.
    let crashed = store.copy();
    drop(session);

    let checkpoint = crashed.checkpoint(&root()).unwrap();
    assert_eq!(checkpoint.state.phase, SessionPhase::Running);
    let later = ScriptedRuntime::new([Turn::Events(vec![
        event::text("carried on"),
        event::response(1, 1),
    ])]);
    let (counting, runs) = counted("write_once", "wrote again");
    let deps = SessionDeps {
        runtime: Rc::new(test_runtime(vec![counting])),
        store: crashed.session_store(&root()),
        ids: Rc::new(SequentialIds::new("restored")),
        clock: Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
        config: SessionConfig::default(),
    };
    let (restored, continuation) =
        AgentSession::restore(checkpoint, root(), Box::new(later.clone()), deps).unwrap();

    assert!(continuation.interrupted);
    assert_eq!(restored.phase(), SessionPhase::Idle);
    let blocks = restored.transcript().blocks;
    assert_eq!(
        tool_output(&blocks[1]),
        (
            ToolCallStatus::Error,
            texts(&[
                "Tool call interrupted: write_once (the process died before a result was recorded)"
            ])
        )
    );
    restored
        .send(text("go on"), turn("t2"))
        .unwrap()
        .await
        .unwrap();
    assert_eq!(runs.get(), 0);
    let request = &later.requests()[0];
    assert_eq!(
        item_kinds(&request.items),
        ["user_message", "tool_use", "tool_result", "user_message"]
    );
}

#[tokio::test(flavor = "local")]
async fn restore_refuses_a_checkpoint_another_harness_saved() {
    let provider = ScriptedRuntime::new(Vec::new());
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    drop(session);
    let mut checkpoint = store.checkpoint(&root()).unwrap();
    checkpoint.state.harness = "other".into();
    let deps = SessionDeps {
        runtime: Rc::new(test_runtime(Vec::new())),
        store: store.session_store(&root()),
        ids: Rc::new(SequentialIds::new("id")),
        clock: Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
        config: SessionConfig::default(),
    };

    let refused = AgentSession::restore(checkpoint, root(), Box::new(provider), deps).err();

    assert_eq!(
        refused,
        Some(RestoreError::Harness {
            stored: "other".into(),
            expected: "test".into(),
        })
    );
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn saves_run_one_at_a_time_even_when_a_scheduled_save_meets_a_flush() {
    // The run answers after 5 ms, so the persister's save of the user block
    // is still in flight when the answer's text completes and the action's
    // own save follows.
    let provider = ScriptedRuntime::new([Turn::Stream(Box::new(|_| {
        use futures_util::StreamExt;
        futures_util::stream::once(tokio::time::sleep(Duration::from_millis(5)))
            .flat_map(|()| futures_util::stream::iter([event::text("done"), event::response(1, 1)]))
            .boxed_local()
    }))]);
    let store = MemoryTreeStore::new();
    let slow = Rc::new(SlowStore {
        inner: store.session_store(&root()),
        in_flight: RefCell::new(0),
        most: RefCell::new(0),
    });
    let deps = SessionDeps {
        runtime: Rc::new(test_runtime(Vec::new())),
        store: slow.clone(),
        ids: Rc::new(SequentialIds::new("id")),
        clock: Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
        config: SessionConfig {
            persist_interval: Duration::from_millis(1),
            ..SessionConfig::default()
        },
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

    session.send(text("go"), turn("t1")).unwrap().await.unwrap();

    assert_eq!(*slow.most.borrow(), 1);
    // After the node's creation: the scheduled save, then the action's.
    let saves = store.saves();
    assert_eq!(saves.len(), 3);
    let (_, last) = &saves[2];
    assert_eq!(last.block_count, session.transcript().blocks.len());
    assert_eq!(last.state.phase, SessionPhase::Idle);
}

/// A store whose saves take 20 ms, counting how many overlap.
struct SlowStore {
    inner: Rc<dyn SessionStore>,
    in_flight: RefCell<u32>,
    most: RefCell<u32>,
}

impl SessionStore for SlowStore {
    fn save<'a>(
        &'a self,
        update: crate::store::CheckpointUpdate,
        guard: &'a crate::store::CommitGuard,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            *self.in_flight.borrow_mut() += 1;
            let now = *self.in_flight.borrow();
            let most = (*self.most.borrow()).max(now);
            *self.most.borrow_mut() = most;
            tokio::time::sleep(Duration::from_millis(20)).await;
            *self.in_flight.borrow_mut() -= 1;
            self.inner.save(update, guard).await
        })
    }

    fn load(&self) -> LocalBoxFuture<'_, Result<Option<Checkpoint>, StoreError>> {
        self.inner.load()
    }
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_message_queued_while_a_tool_runs_is_saved_without_a_transcript_change() {
    let provider = ScriptedRuntime::new([Turn::Events(vec![
        event::tool_call("call-1", "slow", json!({})),
        event::response(1, 1),
    ])]);
    let store = MemoryTreeStore::new();
    let (slow, _releases, started) = gated_tool("slow");
    let session = start(&provider, vec![slow], &store, SessionConfig::default()).await;
    let _running = session.send(text("go"), turn("t1")).unwrap();
    started.await.unwrap();
    let saves = store.saves().len();

    let _queued = session.send(text("next"), turn("t2")).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let saves = &store.saves()[saves..];
    assert_eq!(saves.len(), 1);
    let (_, update) = &saves[0];
    assert!(update.changed_blocks.is_empty());
    assert_eq!(update.state.queue.len(), 1);
    assert_eq!(update.state.queue[0].id, turn("t2"));
}

#[tokio::test(flavor = "local")]
async fn an_immediate_switch_lands_inside_the_running_turn_and_a_next_turn_switch_waits() {
    let script = || {
        ScriptedRuntime::new([
            Turn::Events(vec![
                event::tool_call("call-1", "slow", json!({})),
                event::response(1, 1),
            ]),
            Turn::Events(vec![event::text("one"), event::response(1, 1)]),
            Turn::Events(vec![event::text("two"), event::response(1, 1)]),
        ])
    };
    let model_ids = |provider: &ScriptedRuntime| -> Vec<String> {
        provider
            .requests()
            .iter()
            .map(|request| request.model_id.clone())
            .collect()
    };
    for (apply, expected) in [
        (
            ModelSwitchApply::Immediate,
            ["test-model", "model-b", "model-b"],
        ),
        (
            ModelSwitchApply::NextTurn,
            ["test-model", "test-model", "model-b"],
        ),
    ] {
        let provider = script();
        let store = MemoryTreeStore::new();
        let (slow, releases, started) = gated_tool("slow");
        let session = start(&provider, vec![slow], &store, SessionConfig::default()).await;
        let running = session.send(text("first"), turn("t1")).unwrap();
        started.await.unwrap();
        session
            .update_model(ModelSwitch {
                model: model_of("stub", "model-b"),
                runtime: None,
                apply,
            })
            .unwrap();
        releases.borrow_mut().remove(0).send(()).unwrap();
        running.await.unwrap();
        session
            .send(text("second"), turn("t2"))
            .unwrap()
            .await
            .unwrap();

        assert_eq!(model_ids(&provider), expected, "{apply}");
        // Each block keeps the model current when it was written.
        let blocks = session.transcript().blocks;
        assert_eq!(blocks[0].model().model.id, "test-model");
        assert_eq!(blocks.last().unwrap().model().model.id, "model-b");
        assert_eq!(
            store.checkpoint(&root()).unwrap().state.model.model.id,
            "model-b"
        );
    }
}

#[tokio::test(flavor = "local")]
async fn a_switch_to_another_provider_runs_the_next_turn_on_its_runtime_and_closes_the_old_one() {
    let first = ScriptedRuntime::new([Turn::Events(vec![
        event::text("from a"),
        event::response(1, 1),
    ])]);
    let second = ScriptedRuntime::new([Turn::Events(vec![
        event::text("from b"),
        event::response(1, 1),
    ])]);
    let replaced = ScriptedRuntime::new(Vec::new());
    let store = MemoryTreeStore::new();
    let session = start(&first, Vec::new(), &store, SessionConfig::default()).await;
    session.send(text("hi"), turn("t1")).unwrap().await.unwrap();

    // A pending switch that a later one replaces is closed too.
    for runtime in [&replaced, &second] {
        let switch = ModelSwitch {
            model: model_of("other", "model-b"),
            runtime: Some(Box::new(runtime.clone())),
            apply: ModelSwitchApply::NextTurn,
        };
        session.update_model(switch).unwrap();
    }
    assert!(!session.needs_runtime_for(&model_of("other", "model-c")));
    session
        .send(text("again"), turn("t2"))
        .unwrap()
        .await
        .unwrap();

    assert_eq!((first.requests().len(), first.closes()), (1, 1));
    assert_eq!((replaced.requests().len(), replaced.closes()), (0, 1));
    assert_eq!(second.requests()[0].model_id, "model-b");
    assert_eq!(second.closes(), 0);
}

#[tokio::test(flavor = "local")]
async fn an_immediate_switch_to_another_provider_continues_the_turn_on_the_new_runtime() {
    let first = ScriptedRuntime::new([Turn::Events(vec![
        event::tool_call("call-1", "slow", json!({})),
        event::response(1, 1),
    ])]);
    let second = ScriptedRuntime::new([Turn::Events(vec![
        event::text("from b"),
        event::response(1, 1),
    ])]);
    let store = MemoryTreeStore::new();
    let (slow, releases, started) = gated_tool("slow");
    let session = start(&first, vec![slow], &store, SessionConfig::default()).await;
    let running = session.send(text("go"), turn("t1")).unwrap();
    started.await.unwrap();

    session
        .update_model(ModelSwitch {
            model: model_of("other", "model-b"),
            runtime: Some(Box::new(second.clone())),
            apply: ModelSwitchApply::Immediate,
        })
        .unwrap();
    releases.borrow_mut().remove(0).send(()).unwrap();
    running.await.unwrap();

    assert_eq!((first.requests().len(), first.closes()), (1, 1));
    // The continuation of the same turn carries the completed call.
    let request = &second.requests()[0];
    assert_eq!(
        (request.turn_id.as_str(), request.model_id.as_str()),
        ("t1", "model-b")
    );
    assert_eq!(
        item_kinds(&request.items),
        ["user_message", "tool_use", "tool_result"]
    );
}

#[tokio::test(flavor = "local")]
async fn the_stream_becomes_blocks_and_each_delta_one_patch() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            ProviderEvent::ThinkingStart,
            event::thinking("private "),
            event::thinking("notes"),
            ProviderEvent::ThinkingSignature("anthropic:sig".into()),
            ProviderEvent::RedactedThinking("opaque".into()),
            event::text("Hello "),
            event::text("world"),
            event::tool_call("call-1", "echo", json!("{\"broken\":")),
            event::response(3, 4),
        ]),
        Turn::Events(vec![event::text("next"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let batches = Rc::new(RefCell::new(Vec::new()));
    let _subscription = session.subscribe({
        let batches = batches.clone();
        move |event| {
            if let SessionEvent::TranscriptChanged { patches, .. } = event {
                batches.borrow_mut().push(patches.clone());
            }
        }
    });

    session
        .send(text("think"), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "thinking",
            "redacted_thinking",
            "text",
            "tool_call:error",
            "response",
            "text",
            "response"
        ]
    );
    let Block::Thinking(thinking) = &blocks[1] else {
        unreachable!()
    };
    assert_eq!(
        (thinking.text.as_str(), thinking.signature.as_deref()),
        ("private notes", Some("anthropic:sig"))
    );
    let Block::Text(answer) = &blocks[3] else {
        unreachable!()
    };
    assert_eq!(
        (answer.text.as_str(), answer.forkable),
        ("Hello world", true)
    );
    // A malformed input is kept as the text the vendor sent.
    let Block::ToolCall(call) = &blocks[4] else {
        unreachable!()
    };
    assert_eq!(call.input, "{\"broken\":");
    assert_eq!(tool_output(&blocks[4]).1, texts(&["Tool not found: echo"]));
    // Signed thinking and redacted data go back as they came, in order.
    let replayed = &provider.requests()[1].items;
    assert_eq!(
        replayed[1..3],
        [
            InferenceItem::AssistantThinking {
                model_id: "test-model".into(),
                text: "private notes".into(),
                signature: Some("anthropic:sig".into()),
            },
            InferenceItem::AssistantRedactedThinking {
                model_id: "test-model".into(),
                data: "opaque".into(),
            },
        ]
    );
    let InferenceItem::ToolUse { input, .. } = &replayed[4] else {
        panic!("{replayed:?}")
    };
    assert_eq!(*input, json!("{\"broken\":"));
    let appends: Vec<String> = batches
        .borrow()
        .iter()
        .flatten()
        .filter_map(|patch| match patch {
            TranscriptPatch::AppendText { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    // A thinking start and its first text open the block in one patch.
    assert_eq!(appends, ["notes", "world"]);
}

#[tokio::test(flavor = "local")]
async fn a_provider_failure_ends_the_turn_with_its_record_and_the_queued_message_runs_next() {
    let failing = Turn::Stream(Box::new(|_| {
        use futures_util::StreamExt;
        futures_util::stream::iter([event::text("partial")])
            .chain(futures_util::stream::once(async {
                event::error("the vendor is overloaded", Some(ErrorCode::Overloaded))
            }))
            .boxed_local()
    }));
    let provider = ScriptedRuntime::new([
        failing,
        Turn::Events(vec![event::text("second"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let reports = Rc::new(RefCell::new(Vec::new()));
    let _subscription = session.subscribe({
        let reports = reports.clone();
        move |event| {
            if let SessionEvent::Error { report } | SessionEvent::ActionFailed { report } = event {
                reports.borrow_mut().push(report.clone());
            }
        }
    });
    let first = session.send(text("first"), turn("t1")).unwrap();
    let second = session.send(text("second"), turn("t2")).unwrap();

    let failed = *first.await.unwrap_err();
    second.await.unwrap();

    assert_eq!(failed.message, "the vendor is overloaded");
    assert_eq!(failed.code.as_deref(), Some("overloaded"));
    let diagnostics = failed.diagnostics.clone().unwrap();
    assert_eq!(diagnostics.source, FailureSource::Unknown);
    assert_eq!(
        diagnostics.client_request_id.as_deref(),
        Some(provider.requests()[0].request_id.as_str())
    );
    // Reported once as the turn's error, once as the failed action.
    assert_eq!(*reports.borrow(), [failed.clone(), failed]);
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        ["user", "text", "error", "user", "text", "response"]
    );
    let Block::Error(record) = &blocks[2] else {
        unreachable!()
    };
    assert_eq!(
        record.diagnostics.as_ref().unwrap().source,
        FailureSource::Unknown
    );
}

#[tokio::test(flavor = "local")]
async fn stop_takes_the_running_action_then_the_first_waiting_one_then_nothing() {
    let provider = ScriptedRuntime::new([
        Turn::pending(),
        Turn::Events(vec![event::text("third"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let first = session.send(text("hang"), turn("t1")).unwrap();
    let second = session.send(text("queued"), turn("t2")).unwrap();
    let third = session.send(text("also queued"), turn("t3")).unwrap();
    until(|| provider.requests().len() == 1).await;

    // The second stop comes while the first is still being recorded, so it
    // takes the first waiting message.
    let (stopped, removed) = tokio::join!(session.abort(), session.abort());

    assert_eq!(
        stopped,
        AbortResult {
            target: Some(AbortTarget::ActiveProviderStream),
            can_abort_again: true,
        }
    );
    assert_eq!(
        removed,
        AbortResult {
            target: Some(AbortTarget::QueuedMessage),
            can_abort_again: true,
        }
    );
    assert_eq!(first.await, Ok(ActionEnd::Aborted));
    assert_eq!(second.await, Ok(ActionEnd::Dropped));
    assert_eq!(third.await, Ok(ActionEnd::Completed));
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "abort", "user", "text", "response"]
    );
    assert_eq!(
        session.abort().await,
        AbortResult {
            target: None,
            can_abort_again: false,
        }
    );
}

#[tokio::test(flavor = "local")]
async fn a_repeated_message_id_runs_one_turn() {
    let provider = ScriptedRuntime::new([Turn::Events(vec![
        event::text("once"),
        event::response(1, 1),
    ])]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let first = session.send(text("hello"), turn("stable")).unwrap();
    assert_eq!(
        session.send(text("hello"), turn("stable")).unwrap().await,
        Ok(ActionEnd::Duplicate)
    );
    first.await.unwrap();
    assert_eq!(
        session.send(text("hello"), turn("stable")).unwrap().await,
        Ok(ActionEnd::Duplicate)
    );
    assert_eq!(provider.requests().len(), 1);
}

#[tokio::test(flavor = "local")]
async fn queued_messages_can_leave_the_queue_or_run_next_and_the_queue_is_saved() {
    let provider = ScriptedRuntime::new([
        Turn::pending(),
        Turn::Events(vec![event::text("third"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let _first = session.send(text("first"), turn("t1")).unwrap();
    let second = session.send(text("second"), turn("t2")).unwrap();
    let third = session.send(text("third"), turn("t3")).unwrap();
    let fourth = session.send(text("fourth"), turn("t4")).unwrap();
    let queue = |session: &AgentSession| -> Vec<String> {
        session
            .queued_messages()
            .iter()
            .map(|message| message.id.to_string())
            .collect()
    };

    assert!(session.send_queued_message(&turn("t3")));
    assert_eq!(queue(&session), ["t3", "t2", "t4"]);
    assert!(session.dequeue_message(&turn("t2")));
    assert!(!session.dequeue_message(&turn("t2")));
    assert_eq!(second.await, Ok(ActionEnd::Dropped));
    assert_eq!(session.clear_message_queue(), 2);
    assert_eq!(
        (third.await, fourth.await),
        (Ok(ActionEnd::Dropped), Ok(ActionEnd::Dropped))
    );
    assert!(queue(&session).is_empty());
}

#[tokio::test(flavor = "local")]
async fn events_reach_listeners_in_order_even_when_a_listener_changes_the_session() {
    let provider = ScriptedRuntime::new([Turn::pending()]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _changer = session.subscribe({
        let session = session.clone();
        move |event| {
            // Removes the queued message the moment it is queued.
            if let SessionEvent::QueueChanged { queue } = event
                && let Some(message) = queue.first()
            {
                session.dequeue_message(&message.id);
            }
        }
    });
    let _recorder = session.subscribe({
        let seen = seen.clone();
        move |event| {
            let entry = match event {
                SessionEvent::QueueChanged { queue } => format!("queue {}", queue.len()),
                SessionEvent::PhaseChanged { phase } => format!("phase {phase}"),
                _ => return,
            };
            seen.borrow_mut().push(entry);
        }
    });

    let _first = session.send(text("run"), turn("t1")).unwrap();
    let _second = session.send(text("queued"), turn("t2")).unwrap();

    // Every listener sees the queue grow before either sees it shrink.
    assert_eq!(*seen.borrow(), ["phase running", "queue 1", "queue 0"]);
}
