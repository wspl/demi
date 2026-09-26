//! What the scenarios share: a harness, a provider runtime that answers each
//! node of a tree from its own script, a server over an in-memory tree
//! store, and frame builders.

use std::{cell::RefCell, collections::VecDeque, rc::Rc, sync::Arc};

use demi_agent::{
    AgentHarness, AgentServer, AgentTreeStore, Profile, PromptContext, ServerConfig, ServerDeps,
    testing::{
        MemoryTreeStore, NoHost, NoShells, ScriptedProviders, SequentialIds, TestClient,
        TokioClock, test_model,
    },
};
use demi_agent_protocol::{ClientFrame, ServerFrame};
use demi_core::{Block, Clock, ModelSelection, NodeId, Timestamp, TurnId, UserContentBlock};
use demi_provider::{
    InferenceItem, InferenceRequest, ProviderRun, ProviderRuntime,
    testing::{FixedClock, ScriptedRuntime, Turn},
};
use demi_shell::{
    CommandSet, GroupBuilder, JobCaller, LeafBuilder, PortError, PortRequest, PortResponse,
    PortTransport, RpcError, RpcHandler, RpcInvocation, RpcPort, StorageOp, StorageReply,
};
use futures_util::{StreamExt, future::LocalBoxFuture, stream};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// The harness the scenarios run: fixed texts, one command, and the
/// conversation's execution context, which the test can set.
#[derive(Default)]
pub struct TestHarness {
    pub preamble: Option<String>,
    pub profiles: Vec<Profile>,
    /// The execution context: each node whose transcript does not hold it
    /// yet is given it before its next request.
    pub context: RefCell<Option<String>>,
    /// The context texts each request's context hook was given.
    pub seen: RefCell<Vec<Vec<String>>>,
    /// The command help each system prompt was given.
    pub prompts: RefCell<Vec<String>>,
}

impl AgentHarness for TestHarness {
    type Host = NoHost;

    fn name(&self) -> &str {
        "test"
    }

    fn commands(&self) -> Rc<CommandSet> {
        let mut commands = CommandSet::new();
        commands
            .register(
                GroupBuilder::new("greet", "Greets the caller.")
                    .leaf(LeafBuilder::rpc("hello", "Say hello.").bind(Hello)),
            )
            .expect("the test command registers");
        Rc::new(commands)
    }

    fn profiles(&self) -> Vec<Profile> {
        self.profiles.clone()
    }

    async fn system_prompt(&self, _context: PromptContext<'_>, commands: &str) -> String {
        self.prompts.borrow_mut().push(commands.to_owned());
        "system prompt".to_owned()
    }

    async fn preamble(&self, _context: PromptContext<'_>) -> Option<String> {
        self.preamble.clone()
    }

    async fn context(&self, _context: PromptContext<'_>, seen: &[&str]) -> Option<String> {
        self.seen
            .borrow_mut()
            .push(seen.iter().map(|text| (*text).to_owned()).collect());
        let current = self.context.borrow().clone()?;
        (!seen.contains(&current.as_str())).then_some(current)
    }
}

struct Hello;

impl RpcHandler for Hello {
    fn call(
        &self,
        _invocation: RpcInvocation,
        _port: RpcPort,
    ) -> LocalBoxFuture<'_, Result<u8, RpcError>> {
        Box::pin(async { Ok(0) })
    }
}

/// The provider of every node of a tree: the root's requests are answered
/// from the root's script, a child's from the script whose key its first
/// user message contains, in order. A run beyond its script panics: the
/// test did not script it.
#[derive(Clone, Default)]
pub struct Model(Rc<RefCell<Scripts>>);

#[derive(Default)]
struct Scripts {
    root: VecDeque<Turn>,
    children: Vec<(String, VecDeque<Turn>)>,
    requests: Vec<InferenceRequest>,
}

impl Model {
    /// The root's next runs.
    pub fn root(&self, turns: impl IntoIterator<Item = Turn>) {
        self.0.borrow_mut().root.extend(turns);
    }

    /// The next runs of the child whose brief contains `key`.
    pub fn child(&self, key: &str, turns: impl IntoIterator<Item = Turn>) {
        let mut scripts = self.0.borrow_mut();
        match scripts.children.iter_mut().find(|(known, _)| known == key) {
            Some((_, queue)) => queue.extend(turns),
            None => scripts
                .children
                .push((key.to_owned(), turns.into_iter().collect())),
        }
    }

    /// Every request so far, in order.
    pub fn requests(&self) -> Vec<InferenceRequest> {
        self.0.borrow().requests.clone()
    }

    /// The requests of the node whose brief contains `key`.
    pub fn requests_of(&self, key: &str) -> Vec<InferenceRequest> {
        self.requests()
            .into_iter()
            .filter(|request| request.session_id != conversation().as_str())
            .filter(|request| brief(request).contains(key))
            .collect()
    }

    /// The root's requests.
    pub fn root_requests(&self) -> Vec<InferenceRequest> {
        self.requests()
            .into_iter()
            .filter(|request| request.session_id == conversation().as_str())
            .collect()
    }

    /// Whether every scripted run was played.
    pub fn is_done(&self) -> bool {
        let scripts = self.0.borrow();
        scripts.root.is_empty() && scripts.children.iter().all(|(_, queue)| queue.is_empty())
    }
}

/// The text of a request's first user message: a child's preamble and its
/// brief.
pub fn brief(request: &InferenceRequest) -> String {
    request
        .items
        .iter()
        .find_map(|item| match item {
            InferenceItem::UserMessage { content } => Some(texts(content)),
            _ => None,
        })
        .unwrap_or_default()
}

/// Every text of `content`, one per line.
pub fn texts(content: &[UserContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            UserContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every text a request carries, one item per line.
pub fn request_text(request: &InferenceRequest) -> String {
    request
        .items
        .iter()
        .map(|item| match item {
            InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
                texts(content)
            }
            InferenceItem::AssistantText { text, .. } => text.clone(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl ProviderRuntime for Model {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        let turn = {
            let mut scripts = self.0.borrow_mut();
            scripts.requests.push(request.clone());
            if request.session_id == conversation().as_str() {
                scripts.root.pop_front()
            } else {
                let text = brief(&request);
                scripts
                    .children
                    .iter_mut()
                    .find(|(key, _)| text.contains(key.as_str()))
                    .and_then(|(_, queue)| queue.pop_front())
            }
        };
        let Some(turn) = turn else {
            panic!(
                "Model: no run scripted for {} ({})",
                request.session_id,
                brief(&request)
            );
        };
        let events = match turn {
            Turn::Events(events) => stream::iter(events).boxed_local(),
            Turn::Respond(respond) => stream::iter(respond(&request)).boxed_local(),
            Turn::Stream(open) => open(&request),
        };
        events
            .take_until(request.cancel.cancelled_owned())
            .boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(self.clone())
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// Holds the runs [`held`] makes until it is opened.
#[derive(Clone)]
pub struct Gate(Rc<watch::Sender<bool>>);

impl Gate {
    pub fn new() -> Self {
        Self(Rc::new(watch::Sender::new(false)))
    }

    pub fn open(&self) {
        self.0.send_replace(true);
    }
}

/// A run that plays `events` once `gate` is open.
pub fn held(gate: &Gate, events: Vec<demi_provider::ProviderEvent>) -> Turn {
    let mut opened = gate.0.subscribe();
    Turn::Stream(Box::new(move |_| {
        stream::once(async move {
            // The gate lives in the test that holds the run.
            let _ = opened.wait_for(|open| *open).await;
            stream::iter(events)
        })
        .flatten()
        .boxed_local()
    }))
}

/// What a `demi agent` call wrote and how it exited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRun {
    pub code: u8,
    pub stdout: String,
    pub stderr: String,
}

/// A job's port: its output kept, and its command storage the node's at
/// the generation the job recorded, while its call lives.
struct NodePort {
    server: Rc<AgentServer<TestHarness>>,
    caller: JobCaller,
    call: CancellationToken,
    stdout: RefCell<Vec<u8>>,
    stderr: RefCell<Vec<u8>>,
}

impl PortTransport for NodePort {
    fn request(&self, request: PortRequest) -> LocalBoxFuture<'_, Result<PortResponse, PortError>> {
        Box::pin(async move {
            match request {
                PortRequest::Stdout { bytes } => {
                    self.stdout.borrow_mut().extend_from_slice(bytes.as_bytes());
                    Ok(PortResponse::Written {})
                }
                PortRequest::Stderr { bytes } => {
                    self.stderr.borrow_mut().extend_from_slice(bytes.as_bytes());
                    Ok(PortResponse::Written {})
                }
                PortRequest::ReadStdin {} | PortRequest::ReadLiveStdin {} => {
                    Ok(PortResponse::Input { bytes: None })
                }
                PortRequest::Storage { op } => {
                    let reply = self
                        .server
                        .command_storage(&conversation(), &self.caller, op, self.call.clone())
                        .await?;
                    Ok(PortResponse::Storage { reply })
                }
            }
        })
    }
}

/// Runs `demi agent <verb>` with `args` as a job of `node` would, through
/// the node's commands and its command storage; `cancel` is the call's
/// cancellation.
pub async fn agent_call(
    server: &Rc<AgentServer<TestHarness>>,
    node: &NodeId,
    verb: &str,
    args: Value,
    json: bool,
    cancel: CancellationToken,
) -> Result<CommandRun, RpcError> {
    let live = server
        .node(&conversation(), node)
        .expect("the calling node is live");
    let invocation: RpcInvocation = serde_json::from_value(json!({
        "path": ["demi", "agent", verb],
        "argv": [],
        "args": args,
        "json": json,
        "cwd": "/workspace",
        "env": {},
        "context": {
            "conversation": conversation(),
            "caller": { "kind": "agent", "node": node },
            "locale": { "timeZone": "UTC", "languages": ["en"] },
        },
        "stdin": true,
    }))
    .expect("the invocation is well formed");
    let port = Rc::new(NodePort {
        server: server.clone(),
        caller: live.job_caller(),
        call: cancel.clone(),
        stdout: RefCell::new(Vec::new()),
        stderr: RefCell::new(Vec::new()),
    });
    let code = live
        .commands()
        .dispatch(invocation, RpcPort::new(port.clone(), cancel))
        .await?;
    let stdout = String::from_utf8(port.stdout.borrow().clone()).expect("stdout is text");
    let stderr = String::from_utf8(port.stderr.borrow().clone()).expect("stderr is text");
    Ok(CommandRun {
        code,
        stdout,
        stderr,
    })
}

/// `op` on the command storage of the root `root`, as a job that starts now
/// would send it.
pub async fn command_storage(
    server: &Rc<AgentServer<TestHarness>>,
    root: &NodeId,
    op: StorageOp,
) -> Result<StorageReply, PortError> {
    let caller = server
        .node(root, root)
        .expect("the conversation is open")
        .job_caller();
    server
        .command_storage(root, &caller, op, CancellationToken::new())
        .await
}

/// `demi agent <verb>` run to its end as a job of `node`.
pub async fn agent(
    server: &Rc<AgentServer<TestHarness>>,
    node: &NodeId,
    verb: &str,
    args: Value,
) -> CommandRun {
    agent_call(server, node, verb, args, false, CancellationToken::new())
        .await
        .expect("the call is dispatched")
}

/// A server with its store, resolver and harness in reach.
pub struct Fixture {
    pub server: Rc<AgentServer<TestHarness>>,
    pub store: Rc<MemoryTreeStore>,
    pub resolver: Rc<ScriptedProviders>,
    pub harness: Rc<TestHarness>,
}

impl Fixture {
    /// A server of the provider `stub` playing `script`.
    pub fn new(script: &ScriptedRuntime) -> Self {
        Self::with(script, MemoryTreeStore::new(), ServerConfig::default())
    }

    pub fn with(
        script: &ScriptedRuntime,
        store: Rc<MemoryTreeStore>,
        config: ServerConfig,
    ) -> Self {
        let resolver = Rc::new(ScriptedProviders::default());
        resolver.provide("stub", script);
        let clock = Arc::new(FixedClock(start()));
        Self::build(resolver, TestHarness::default(), store, config, clock)
    }

    /// A server of the provider `stub` answering from `model`, with
    /// `harness`.
    pub fn with_model(
        model: &Model,
        harness: TestHarness,
        store: Rc<MemoryTreeStore>,
        config: ServerConfig,
    ) -> Self {
        let resolver = Rc::new(ScriptedProviders::default());
        resolver.provide_runtime("stub", model.clone());
        Self::build(
            resolver,
            harness,
            store,
            config,
            Arc::new(FixedClock(start())),
        )
    }

    /// As [`with_model`](Self::with_model), on a wall clock that moves with
    /// Tokio's, so that yield wakeups fall due.
    pub fn with_model_on_tokio_time(model: &Model, harness: TestHarness) -> Self {
        let resolver = Rc::new(ScriptedProviders::default());
        resolver.provide_runtime("stub", model.clone());
        Self::build(
            resolver,
            harness,
            MemoryTreeStore::new(),
            ServerConfig::default(),
            Arc::new(TokioClock::new(start())),
        )
    }

    fn build(
        resolver: Rc<ScriptedProviders>,
        harness: TestHarness,
        store: Rc<MemoryTreeStore>,
        config: ServerConfig,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let harness = Rc::new(harness);
        let stores = {
            let store = store.clone();
            Rc::new(move |_: &NodeId| store.clone() as Rc<dyn AgentTreeStore>)
        };
        let server = AgentServer::new(ServerDeps {
            harness: harness.clone(),
            providers: resolver.clone(),
            shells: Rc::new(NoShells),
            stores,
            clock,
            ids: Rc::new(SequentialIds::new("id")),
            config,
        });
        Self {
            server,
            store,
            resolver,
            harness,
        }
    }

    pub fn client(&self) -> TestClient<TestHarness> {
        TestClient::connect(&self.server, &conversation(), "/workspace")
    }

    /// A client that opened the conversation and read its handshake.
    pub async fn opened(&self) -> TestClient<TestHarness> {
        let mut client = self.client();
        client.send(open(test_model())).await;
        let handshake = client.next_until(is_pending_steers).await;
        assert_eq!(
            handshake.first(),
            Some(&ServerFrame::Opened),
            "{handshake:?}"
        );
        client
    }
}

/// The frames until and with the first that `done` accepts; the wait fails
/// the test after ten seconds.
pub async fn frames_until(
    client: &mut TestClient<TestHarness>,
    done: impl Fn(&ServerFrame) -> bool,
) -> Vec<ServerFrame> {
    async fn collect(
        client: &mut TestClient<TestHarness>,
        done: &impl Fn(&ServerFrame) -> bool,
        frames: &mut Vec<ServerFrame>,
    ) {
        while let Some(frame) = client.next().await {
            let last = done(&frame);
            frames.push(frame);
            if last {
                return;
            }
        }
        panic!("the connection closed");
    }
    let mut frames = Vec::new();
    let waited = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        collect(client, &done, &mut frames),
    )
    .await;
    assert!(waited.is_ok(), "no frame ended the wait: {frames:#?}");
    frames
}

/// Lets the other tasks run until `done` holds, such as a worker reaching
/// its provider request.
pub async fn until(done: impl Fn() -> bool) {
    for _ in 0..1_000 {
        if done() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("the condition never held");
}

/// The wall-clock time the scenarios start at.
fn start() -> Timestamp {
    "2026-09-24T12:00:00.000Z"
        .parse()
        .expect("the time is RFC 3339")
}

/// The root's session.
pub fn session_of(fixture: &Fixture) -> demi_agent::AgentSession {
    fixture
        .server
        .tree(&conversation())
        .expect("the tree is live")
        .root()
        .session()
        .clone()
}

pub fn conversation() -> NodeId {
    NodeId::try_from("conversation").unwrap()
}

pub fn turn(id: &str) -> TurnId {
    TurnId::try_from(id).unwrap()
}

pub fn open(model: ModelSelection) -> ClientFrame {
    ClientFrame::Open { model }
}

pub fn send(id: &str, message: &str) -> ClientFrame {
    ClientFrame::Send {
        message_id: turn(id),
        content: demi_agent::testing::client_text(message),
    }
}

pub fn is_pending_steers(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::PendingSteers { .. })
}

pub fn is_idle(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::Phase { phase } if *phase == demi_core::SessionPhase::Idle)
}

/// Each block's type, and a tool call's status beside it.
pub fn kinds(blocks: &[Block]) -> Vec<String> {
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

/// The frame's `type`.
pub fn frame_type(frame: &ServerFrame) -> String {
    serde_json::to_value(frame).unwrap()["type"]
        .as_str()
        .unwrap()
        .to_owned()
}
