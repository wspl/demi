//! What the scenarios share: a product, a provider runtime that answers each
//! node of a tree from its own script, a server over an in-memory tree
//! store, and frame builders.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
    sync::Arc,
};

use demi_agent_server::{
    AgentServer, ServerConfig, ServerDeps,
    testing::{ScriptedProviders, TestClient},
};
use demi_agent_store::{AgentTreeStore, testing::MemoryTreeStore};
use demi_agent_tools::{
    ContextSource, HostResolver, NodeContext, SubagentSettings, SubagentSource, Toolset,
    testing::{NoHost, NoShells},
};
use demi_agent_transcript::testing::SequentialIds;
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_host_interface::{
    CommandSet, GroupBuilder, LeafBuilder, PortError, PortRequest, PortResponse, PortTransport,
    RpcError, RpcHandler, RpcInvocation, RpcPort,
};
use demi_provider_common::{
    InferenceItem, InferenceRequest, ProviderRun, ProviderRuntime, RequestLimits, UserPart,
    testing::{FixedClock, ScriptedRuntime, TokioClock, Turn},
};
use demi_shared_types::{
    Block, Clock, ModelSelection, NodeId, ProviderModel, ServiceTier, Timestamp, TurnId,
};
use futures_util::{StreamExt, future::LocalBoxFuture, stream};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// What the scenarios' product supplies: one command, the user's subagent
/// settings, which the test can change between two spawns, and a context
/// source, the conversation's execution context, which the test can set.
pub struct TestProduct {
    /// Subagents on and no profile until the test sets them.
    pub subagents: RefCell<SubagentSettings>,
    /// The execution context: each node whose transcript does not hold it
    /// yet is given it before its next request.
    pub context: RefCell<Option<String>>,
    /// The context texts each request's source was given.
    pub seen: RefCell<Vec<Vec<String>>>,
    /// A plugin's source, asked after the execution context, when the
    /// scenario has one.
    pub plugin: Option<Rc<PluginSource>>,
    /// For each request, whether the plugin had been asked by the time the
    /// execution context, which takes a moment, answered.
    pub plugin_asked_meanwhile: RefCell<Vec<bool>>,
}

/// A plugin's context source: it answers its text once per node.
pub struct PluginSource {
    pub text: String,
    pub asked: Cell<bool>,
}

impl ContextSource for PluginSource {
    fn name(&self) -> &str {
        "plugin"
    }

    fn context<'a>(
        &'a self,
        _node: NodeContext<'a>,
        _turn: &'a TurnId,
        seen: &'a [&'a str],
    ) -> LocalBoxFuture<'a, Result<Option<String>, String>> {
        self.asked.set(true);
        let news = (!seen.contains(&self.text.as_str())).then(|| self.text.clone());
        Box::pin(async move { Ok(news) })
    }
}

impl Default for TestProduct {
    fn default() -> Self {
        Self {
            subagents: RefCell::new(SubagentSettings {
                enabled: true,
                profiles: Vec::new(),
            }),
            context: RefCell::default(),
            seen: RefCell::default(),
            plugin: None,
            plugin_asked_meanwhile: RefCell::default(),
        }
    }
}

impl HostResolver for TestProduct {
    type Host = NoHost;
}

impl SubagentSource for TestProduct {
    fn current(&self) -> LocalBoxFuture<'_, Result<SubagentSettings, String>> {
        let settings = self.subagents.borrow().clone();
        Box::pin(async move { Ok(settings) })
    }
}

impl ContextSource for TestProduct {
    fn name(&self) -> &str {
        "execution"
    }

    fn context<'a>(
        &'a self,
        _node: NodeContext<'a>,
        _turn: &'a TurnId,
        seen: &'a [&'a str],
    ) -> LocalBoxFuture<'a, Result<Option<String>, String>> {
        self.seen
            .borrow_mut()
            .push(seen.iter().map(|text| (*text).to_owned()).collect());
        let current = self.context.borrow().clone();
        let news = current.filter(|current| !seen.contains(&current.as_str()));
        Box::pin(async move {
            if let Some(plugin) = &self.plugin {
                // The execution context reads a Host, which takes a moment.
                tokio::task::yield_now().await;
                self.plugin_asked_meanwhile
                    .borrow_mut()
                    .push(plugin.asked.get());
            }
            Ok(news)
        })
    }
}

/// The scenarios' one command, `greet hello`.
fn greet() -> CommandSet {
    let mut commands = CommandSet::new();
    commands
        .register(
            GroupBuilder::new("greet", "Greets the caller.")
                .index_entry("Greets the caller by name.")
                .leaf(LeafBuilder::rpc("hello", "Say hello.").bind(Hello)),
        )
        .expect("the test command registers");
    commands
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

/// Every text of `content` as a request carries it, one per line.
pub fn texts(content: &[UserPart]) -> String {
    content
        .iter()
        .filter_map(|part| match part {
            UserPart::Text(text) => Some(text.as_str()),
            UserPart::Image(_) | UserPart::Video(_) | UserPart::Document { .. } => None,
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

    fn request_limits(&self, _model: &demi_shared_types::Model) -> RequestLimits {
        RequestLimits::default()
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
pub fn held(gate: &Gate, events: Vec<demi_provider_common::ProviderEvent>) -> Turn {
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

/// A run that streams `before`, then waits until the gate opens and plays
/// `after`.
pub fn streamed_then_held(
    gate: &Gate,
    before: Vec<demi_provider_common::ProviderEvent>,
    after: Vec<demi_provider_common::ProviderEvent>,
) -> Turn {
    let Turn::Stream(rest) = held(gate, after) else {
        unreachable!("a held run is a stream")
    };
    Turn::Stream(Box::new(move |request| {
        stream::iter(before).chain(rest(request)).boxed_local()
    }))
}

/// What a `demi agent` call wrote and how it exited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRun {
    pub code: u8,
    pub stdout: String,
    pub stderr: String,
}

/// The node of the agent a `spawn` or `resume` named on stdout by its
/// number.
pub fn named_node(store: &MemoryTreeStore, run: &CommandRun) -> NodeId {
    let number: u64 = run
        .stdout
        .strip_prefix("subagentId: ")
        .and_then(|rest| rest.strip_suffix('\n'))
        .and_then(|number| number.parse().ok())
        .unwrap_or_else(|| panic!("{run:?} names no child"));
    store
        .numbered(number)
        .unwrap_or_else(|| panic!("no agent {number} in the store"))
}

/// A job's port: its output kept.
struct NodePort {
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
                PortRequest::Medium { .. } => Err(PortError::Ended(
                    "an agent command returns no media".into(),
                )),
            }
        })
    }
}

/// Runs `demi agent <verb>` with `args` as a job of `node` would, through
/// the node's commands; `cancel` is the call's cancellation.
pub async fn agent_call(
    server: &Rc<AgentServer<TestProduct>>,
    node: &NodeId,
    verb: &str,
    args: Value,
    json: bool,
    cancel: CancellationToken,
) -> Result<CommandRun, RpcError> {
    let live = server
        .node(&conversation(), node)
        .expect("the calling node is live");
    let mut invocation: RpcInvocation = serde_json::from_value(json!({
        "path": ["demi", "agent", verb],
        "argv": [],
        "args": args,
        "json": json,
        "host": "laptop",
        "cwd": "/workspace",
        "env": {},
        "context": {
            "conversation": conversation(),
            "caller": { "kind": "agent", "number": live.record().number },
            "locale": { "timeZone": "UTC", "languages": ["en"] },
            "colorScheme": "light",
        },
        "stdin": true,
    }))
    .expect("the invocation is well formed");
    // The backend's record of the job names the node that started it.
    invocation.caller = Some(live.job_caller());
    let port = Rc::new(NodePort {
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

/// `demi agent <verb>` run to its end as a job of `node`.
pub async fn agent(
    server: &Rc<AgentServer<TestProduct>>,
    node: &NodeId,
    verb: &str,
    args: Value,
) -> CommandRun {
    agent_call(server, node, verb, args, false, CancellationToken::new())
        .await
        .expect("the call is dispatched")
}

/// A server with its store, resolver and product in reach.
pub struct Fixture {
    pub server: Rc<AgentServer<TestProduct>>,
    pub store: Rc<MemoryTreeStore>,
    pub resolver: Rc<ScriptedProviders>,
    pub product: Rc<TestProduct>,
}

impl Fixture {
    /// The number the model knows the agent `node` by.
    pub fn number(&self, node: &NodeId) -> u64 {
        self.store
            .record(node)
            .unwrap_or_else(|| panic!("no node {node} in the store"))
            .number
    }

    /// A server of the provider `stub` playing `script`.
    pub fn new(script: &ScriptedRuntime) -> Self {
        Self::with(script, MemoryTreeStore::new(), ServerConfig::default())
    }

    pub fn with(
        script: &ScriptedRuntime,
        store: Rc<MemoryTreeStore>,
        config: ServerConfig,
    ) -> Self {
        Self::with_product(script, TestProduct::default(), store, config)
    }

    /// As [`with`](Self::with), with `product`.
    pub fn with_product(
        script: &ScriptedRuntime,
        product: TestProduct,
        store: Rc<MemoryTreeStore>,
        config: ServerConfig,
    ) -> Self {
        let resolver = Rc::new(ScriptedProviders::default());
        resolver.provide("stub", script);
        let clock = Arc::new(FixedClock(start()));
        Self::build(resolver, product, store, config, clock)
    }

    /// A server of the provider `stub` answering from `model`, with
    /// `product`.
    pub fn with_model(
        model: &Model,
        product: TestProduct,
        store: Rc<MemoryTreeStore>,
        config: ServerConfig,
    ) -> Self {
        let resolver = Rc::new(ScriptedProviders::default());
        resolver.provide_runtime("stub", model.clone());
        Self::build(
            resolver,
            product,
            store,
            config,
            Arc::new(FixedClock(start())),
        )
    }

    /// As [`with_model`](Self::with_model), on a wall clock that moves with
    /// Tokio's, so that yield wakeups fall due.
    pub fn with_model_on_tokio_time(model: &Model, product: TestProduct) -> Self {
        let resolver = Rc::new(ScriptedProviders::default());
        resolver.provide_runtime("stub", model.clone());
        Self::build(
            resolver,
            product,
            MemoryTreeStore::new(),
            ServerConfig::default(),
            Arc::new(TokioClock::new(start())),
        )
    }

    fn build(
        resolver: Rc<ScriptedProviders>,
        product: TestProduct,
        store: Rc<MemoryTreeStore>,
        config: ServerConfig,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let product = Rc::new(product);
        let mut context = vec![product.clone() as Rc<dyn ContextSource>];
        if let Some(plugin) = &product.plugin {
            context.push(plugin.clone());
        }
        let stores = {
            let store = store.clone();
            Rc::new(move |_: &NodeId| store.clone() as Rc<dyn AgentTreeStore>)
        };
        let server = AgentServer::new(ServerDeps {
            toolsets: Rc::new(Toolset {
                commands: Rc::new(greet()),
                revision: Rc::from("test"),
            }),
            subagents: product.clone(),
            instructions: Rc::from("system prompt"),
            guide: Rc::from("harness guide"),
            hosts: product.clone(),
            context: context.into(),
            providers: resolver.clone(),
            shells: Rc::new(NoShells),
            stores,
            clock,
            ids: Rc::new(SequentialIds::new("id")),
            config,
            // No product shows the conversations' statuses.
            status_changed: Rc::new(|_| {}),
        });
        Self {
            server,
            store,
            resolver,
            product,
        }
    }

    pub fn client(&self) -> TestClient<TestProduct> {
        TestClient::connect(&self.server, &conversation(), "/workspace")
    }

    /// A client that opened the conversation and read its handshake.
    pub async fn opened(&self) -> TestClient<TestProduct> {
        let mut client = self.client();
        client.send(open()).await;
        let handshake = client.next_until(is_pending_calls).await;
        assert_eq!(
            handshake.first(),
            Some(&ServerFrame::Opened),
            "{handshake:?}"
        );
        client
    }
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
pub fn session_of(fixture: &Fixture) -> demi_agent_session::AgentSession {
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

pub fn open() -> ClientFrame {
    ClientFrame::Open {}
}

/// Switches the conversation's live tree to `model`, as the backend does
/// once the conversation's record holds it.
pub async fn switch(fixture: &Fixture, model: ModelSelection) {
    let prepared = fixture
        .server
        .prepare_switch(&conversation(), model)
        .await
        .expect("the model's provider resolves")
        .expect("the tree is live");
    fixture.server.switch_model(&conversation(), prepared).await;
}

pub fn send(id: &str, message: &str) -> ClientFrame {
    ClientFrame::Send {
        message_id: turn(id),
        content: demi_agent_server::testing::client_text(message),
    }
}

pub fn is_pending_steers(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::PendingSteers { .. })
}

/// The handshake's last frame before the usage.
pub fn is_pending_calls(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::PendingCalls { .. })
}

pub fn is_context_usage(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::ContextUsage { .. })
}

pub fn is_idle(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::Phase { phase } if *phase == demi_shared_types::SessionPhase::Idle)
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

/// A catalog's model `id` that lists `efforts` and `tiers`.
pub fn listed_model(id: &str, efforts: &[&str], tiers: &[&str]) -> ProviderModel {
    ProviderModel {
        id: id.into(),
        display_name: id.into(),
        description: None,
        context_window: Some(200_000),
        output_limit: None,
        supports_tools: Some(true),
        supports_attachments: None,
        supports_video: None,
        accepted_extensions: None,
        supports_reasoning: None,
        supported_thinking_efforts: (!efforts.is_empty())
            .then(|| efforts.iter().map(|effort| (*effort).to_owned()).collect()),
        can_disable_thinking: None,
        service_tiers: tiers
            .iter()
            .map(|tier| ServiceTier {
                id: (*tier).into(),
                label: (*tier).into(),
                description: None,
                fast: false,
            })
            .collect(),
        default_service_tier_id: None,
        cost: None,
    }
}
