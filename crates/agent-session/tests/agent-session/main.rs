//! The session's compaction tests, which the session's own rules keep.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

use demi_agent_session::{
    ActionEnd, AgentSession, CompactionConfig, Continuation, EditContent, EditSubmission, ModelSwitch, NewContext, SeenContext,
    SessionConfig, SessionDeps, SessionEvent, SessionInit, SessionRuntime, StartedCommand, StepOutcomes, SystemPrompt, ToolEffect, ToolFailure, ToolInvocation, ToolOutcome,
};
use demi_conversation_socket_protocol::AbortTarget;
use demi_provider_common::{
    EntriesOf, ErrorCode, InferenceItem, MediaBytes, ProviderEvent, ProviderRuntime,
    RequestLimits, ResultPart, ToolDefinition, UserPart,
    testing::{FixedClock, ScriptedRuntime, Turn, event},
};
use demi_shared_gates::{ActivityGate, GateLease, Purpose};
use demi_shared_types::{
    AgentMessage, AgentMessageEvent, B64Bytes, BlobRef, Block, BlockId, CommandId, CommandReport, FileExtension, ModelSelection, NodeId, OperationId, ReportEvent, Sender,
    SessionPhase, ShellToolView, ShellViewStatus, Timestamp,
    ToolView, TurnId, UserContentBlock,
};
use futures_util::future::LocalBoxFuture;
use serde_json::json;
use tokio::sync::oneshot;

use demi_agent_store::{
    AgentTreeStore, NodeRecord,
    media::HeldMedia,
    testing::{MemoryTreeStore, model_of, model_reading, sent_text, test_model, text},
};
use demi_agent_transcript::testing::SequentialIds;

mod compaction;

type Invoke =
    Rc<dyn Fn(ToolInvocation) -> LocalBoxFuture<'static, Result<ToolOutcome, ToolFailure>>>;

/// A node runtime whose tools the test defines.
struct TestRuntime {
    tools: Vec<(String, Invoke)>,
    admission: ActivityGate,
    /// While set, the preamble hook never answers.
    hanging_preamble: Rc<Cell<bool>>,
    /// What the `execution` source tells the model while the context it
    /// receives does not hold it.
    execution: Option<&'static str>,
    /// What other sources tell the model, each while the context it
    /// receives does not hold it.
    context: Vec<NewContext>,
    /// The system prompt it gives every model.
    prompt: SystemPrompt,
    /// The context each request's sources were shown, as `source: text`.
    seen: Rc<RefCell<Vec<Vec<String>>>>,
    /// While set, the window the node answers for every model in place of
    /// the model's own, as a user's context limit makes it.
    window_in_use: Rc<Cell<Option<u32>>>,
    /// The tools whose consecutive calls run together as one step.
    together: Vec<&'static str>,
    /// The commands the node's calls left running.
    commands: Rc<TestCommands>,
    /// Whether the node is a child, whose turn a Host's absence does not
    /// leave unfinished.
    child: bool,
}

/// The commands of a test node: each runs until the test ends it, and
/// reports what it is told to, named by the title its record keeps.
#[derive(Default)]
struct TestCommands {
    ended: RefCell<std::collections::HashMap<CommandId, tokio::sync::watch::Sender<bool>>>,
    /// The title of the call that started each, as its record keeps it.
    titles: RefCell<std::collections::HashMap<CommandId, String>>,
    /// How many progress reports each command made.
    reports: RefCell<std::collections::HashMap<CommandId, u32>>,
    /// The commands that ended as lost.
    lost: RefCell<std::collections::HashSet<CommandId>>,
    /// Whether each waits for the user's answer.
    awaits: RefCell<std::collections::HashMap<CommandId, tokio::sync::watch::Sender<bool>>>,
}

impl TestCommands {
    /// The command `id`, which a call titled `title` started and which
    /// runs from now on.
    fn start(&self, id: &str, title: &str) -> CommandId {
        let command = CommandId::try_from(id).unwrap();
        self.ended
            .borrow_mut()
            .insert(command.clone(), tokio::sync::watch::Sender::new(false));
        self.titles.borrow_mut().insert(command.clone(), title.to_owned());
        self.awaits
            .borrow_mut()
            .insert(command.clone(), tokio::sync::watch::Sender::new(false));
        command
    }

    /// The title the record of `command` keeps.
    fn title(&self, command: &CommandId) -> String {
        self.titles.borrow().get(command).cloned().unwrap_or_default()
    }

    /// Ends the command `id`.
    fn end(&self, id: &str) {
        self.ended.borrow()[&CommandId::try_from(id).unwrap()].send_replace(true);
    }

    fn has_ended(&self, command: &CommandId) -> bool {
        self.ended
            .borrow()
            .get(command)
            .is_none_or(|ended| *ended.borrow())
    }
}

/// The system prompt the test runtime gives every model.
const SYSTEM_PROMPT: &str = "system prompt";

impl SessionRuntime for TestRuntime {
    fn enter_action(&self) -> LocalBoxFuture<'_, GateLease> {
        Box::pin(self.admission.enter(Purpose::Demand))
    }

    fn reserve_edit(
        &self,
    ) -> LocalBoxFuture<'_, Result<Option<demi_shared_gates::Reservation>, String>> {
        Box::pin(async { Ok(None) })
    }

    fn system_prompt<'a>(
        &'a self,
        _model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<SystemPrompt, String>> {
        let prompt = self.prompt.clone();
        Box::pin(async move { Ok(prompt) })
    }

    fn context_window<'a>(&'a self, model: &'a ModelSelection) -> LocalBoxFuture<'a, u32> {
        let window = self
            .window_in_use
            .get()
            .unwrap_or(model.model.context_window);
        Box::pin(async move { window })
    }

    fn preamble(&self) -> LocalBoxFuture<'_, Option<String>> {
        if self.hanging_preamble.get() {
            return Box::pin(std::future::pending());
        }
        Box::pin(async { None })
    }

    fn context<'a>(
        &'a self,
        seen: &'a [SeenContext<'a>],
        _turn: &'a TurnId,
    ) -> LocalBoxFuture<'a, Vec<NewContext>> {
        self.seen.borrow_mut().push(
            seen.iter()
                .map(|seen| format!("{}: {}", seen.source, seen.text))
                .collect(),
        );
        let execution = self.execution.map(|text| NewContext {
            source: "execution".into(),
            text: text.into(),
            instructions: Vec::new(),
            cwd: None,
        });
        let news: Vec<NewContext> = execution
            .into_iter()
            .chain(self.context.iter().cloned())
            .filter(|news| !seen.iter().any(|seen| seen.text == news.text))
            .collect();
        Box::pin(async move { news })
    }

    fn agent(&self) -> Option<u64> {
        Some(0)
    }

    /// The node is the root: every agent is below it.
    fn is_below(&self, _node: &NodeId) -> bool {
        true
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

    fn user_recovers_turns(&self) -> bool {
        !self.child
    }

    fn runs_together(&self, tool: &str) -> bool {
        self.together.contains(&tool)
    }

    fn command_ended(&self, command: &CommandId) -> LocalBoxFuture<'static, ()> {
        let ended = self
            .commands
            .ended
            .borrow()
            .get(command)
            .map(tokio::sync::watch::Sender::subscribe);
        Box::pin(async move {
            if let Some(mut ended) = ended {
                // The sender lives as long as the test's commands.
                let _ = ended.wait_for(|ended| *ended).await;
            }
        })
    }

    fn awaits_user(&self, command: &CommandId) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.commands
            .awaits
            .borrow()
            .get(command)
            .map(tokio::sync::watch::Sender::subscribe)
    }

    /// The nth progress report, as having run n seconds, and the end once
    /// it ended ([`running_report`], [`ended_report`]).
    fn report<'a>(
        &'a self,
        command: &'a CommandId,
        _: &'a demi_shared_types::ModelSelection,
        _: RequestLimits,
    ) -> LocalBoxFuture<'a, Option<CommandReport>> {
        let title = self.commands.title(command);
        let report = if self.commands.lost.borrow().contains(command) {
            test_report(command.as_str(), &title, lost())
        } else if self.commands.has_ended(command) {
            test_report(command.as_str(), &title, ReportEvent::Ended { exit_code: None })
        } else {
            let mut reports = self.commands.reports.borrow_mut();
            let count = reports.entry(command.clone()).or_default();
            *count += 1;
            test_report(command.as_str(), &title, running(*count))
        };
        Box::pin(async move { Some(report) })
    }

    fn invoke_step(&self, calls: Vec<ToolInvocation>) -> StepOutcomes<'_> {
        let calls: futures_util::stream::FuturesUnordered<_> = calls
            .into_iter()
            .enumerate()
            .map(|(index, call)| {
                let (_, invoke) = self
                    .tools
                    .iter()
                    .find(|(name, _)| *name == call.tool_name)
                    .expect("the session invokes only the tools it was given");
                let ran = invoke(call);
                async move { (index, ran.await) }
            })
            .collect();
        Box::pin(calls)
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
        output: vec![ResultPart::Text(text.to_owned())],
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
/// finish; one whose input names a `commandId` tells its session it started
/// that command, in the background when `background`.
fn gated_tool(name: &str) -> ((String, Invoke), Releases, oneshot::Receiver<()>) {
    let (started, started_rx) = oneshot::channel();
    let started = Rc::new(RefCell::new(Some(started)));
    let releases: Releases = Rc::default();
    let gate = releases.clone();
    let invoke = tool(name, move |call| {
        if let Some(command) = call.input["commandId"].as_str() {
            call.command.started(StartedCommand {
                command: CommandId::try_from(command).unwrap(),
                background: call.input["background"].as_bool().unwrap_or(false),
            });
        }
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

/// A tool `work` that starts the command its input's `commandId` names in
/// `commands` and leaves it running, in the background when its input's
/// `background` says so, titled by its input's `description`.
fn background_tool(commands: &Rc<TestCommands>) -> (String, Invoke) {
    let commands = commands.clone();
    tool("work", move |call| {
        let command = commands.start(
            call.input["commandId"].as_str().unwrap(),
            call.input["description"].as_str().unwrap(),
        );
        let background = call.input["background"].as_bool().unwrap_or(false);
        let view = ToolView::Shell(ShellToolView {
            request_files: None,
            run_files: None,
            status: ShellViewStatus::Running,
            command_id: command.clone(),
            exit_code: None,
            lost_reason: None,
            running_ms: 0,
            idle_ms: 0,
            chunks: Vec::new(),
            view_truncated: false,
            files: None,
            files_truncated: None,
            path_changes: Vec::new(),
        });
        Box::pin(async move {
            Ok(ToolOutcome {
                view: Some(view),
                effect: Some(ToolEffect::Background { command, background }),
                ..output("status: running")
            })
        })
    })
}

/// A response that calls `work` for the command `command`, reporting every
/// `background`, titled `description`.
fn background_call(command: &str, background: bool, description: &str) -> Vec<ProviderEvent> {
    vec![
        event::tool_call(
            &format!("work-{command}"),
            "work",
            json!({ "commandId": command, "background": background, "description": description }),
        ),
        event::response(1, 1),
    ]
}

/// An edit of the `user` block of the turn `turn` in the session's
/// transcript as it is now, to one text. Equal requests have equal
/// digests.
async fn edit_of(
    session: &AgentSession,
    turn: &str,
    operation: &str,
    replacement: &str,
) -> EditSubmission {
    let snapshot = session.history().await.unwrap();
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
        content: Some(vec![EditContent::Content(UserContentBlock::Text {
            text: replacement.to_owned(),
        })]),
        digest: format!("{operation}:{replacement}"),
    }
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
        execution: None,
        context: Vec::new(),
        prompt: SystemPrompt::identity(SYSTEM_PROMPT),
        seen: Rc::default(),
        window_in_use: Rc::default(),
        together: Vec::new(),
        commands: Rc::default(),
        child: false,
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
    clock: Arc<dyn demi_shared_types::Clock>,
) -> AgentSession {
    start_with(Box::new(provider.clone()), runtime, store, config, clock).await
}

/// A session whose provider runtime is `provider`.
async fn start_with(
    provider: Box<dyn ProviderRuntime>,
    runtime: TestRuntime,
    store: &Rc<MemoryTreeStore>,
    config: SessionConfig,
    clock: Arc<dyn demi_shared_types::Clock>,
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
async fn restore_session(
    checkpoint: demi_agent_store::Checkpoint,
    store: &Rc<MemoryTreeStore>,
    provider: &ScriptedRuntime,
    runtime: TestRuntime,
    clock: Arc<dyn demi_shared_types::Clock>,
) -> (AgentSession, Continuation) {
    restore_configured(
        checkpoint,
        store,
        provider,
        runtime,
        clock,
        SessionConfig::default(),
    )
    .await
}

/// Restores the node `root` from `checkpoint` with `config`, saving into
/// `store`.
async fn restore_configured(
    checkpoint: demi_agent_store::Checkpoint,
    store: &Rc<MemoryTreeStore>,
    provider: &ScriptedRuntime,
    runtime: TestRuntime,
    clock: Arc<dyn demi_shared_types::Clock>,
    config: SessionConfig,
) -> (AgentSession, Continuation) {
    let deps = SessionDeps {
        runtime: Rc::new(runtime),
        store: store.session_store(&root()),
        ids: Rc::new(SequentialIds::new("restored")),
        clock,
        config,
    };
    AgentSession::restore(checkpoint, root(), Box::new(provider.clone()), deps, &[]).await.unwrap()
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
        sender: Some(Sender {
            id: NodeId::try_from("child").unwrap(),
            number: 1,
            description: "UI implementation".into(),
            round: 1,
        }),
        recipient_id: root(),
        timestamp: Timestamp::UNIX_EPOCH,
        content: format!("Result {id}"),
        event: AgentMessageEvent::Message {},
        origin: None,
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

/// A tool result's texts as a request carries them.
fn sent_texts(output: &[&str]) -> Vec<ResultPart> {
    output
        .iter()
        .map(|text| ResultPart::Text((*text).to_owned()))
        .collect()
}


/// A test command's report: its event, and no output.
fn test_report(command: &str, title: &str, event: ReportEvent) -> CommandReport {
    CommandReport {
        command_id: CommandId::try_from(command).unwrap(),
        title: title.to_owned(),
        event,
        output: String::new(),
        media: Vec::new(),
    }
}

/// A test command's nth progress report: it has run n seconds.
fn running(count: u32) -> ReportEvent {
    ReportEvent::Running {
        running_ms: u64::from(count) * 1000,
        idle_ms: 0,
    }
}

/// Why a test command was lost.
fn lost() -> ReportEvent {
    ReportEvent::Lost {
        reason: "the Host's runner started anew".to_owned(),
    }
}

