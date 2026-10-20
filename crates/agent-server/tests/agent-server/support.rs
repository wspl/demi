//! What the scenarios share: a product, a provider runtime that answers each
//! node of a tree from its own script, a server over an in-memory tree
//! store, and frame builders.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    rc::{Rc, Weak},
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;

use demi_agent_server::{
    AgentServer, ServerConfig, ServerDeps,
    testing::{ScriptedProviders, TestClient},
};
use demi_agent_store::{AgentTreeStore, Placement, testing::MemoryTreeStore};
use demi_agent_tools::{
    ContextAnswer, ContextSource, ConversationProfiles, EnvironmentScope, HostResolver, NodeContext,
    WorkflowListing,
    ShellEnvironmentFactory, SubagentSettings, SubagentSource, Toolset,
    testing::{NoHost, NoShells},
};
use demi_agent_transcript::testing::SequentialIds;
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_host_interface::{
    CommandRecord, CommandSet, CommandStatus, DEFAULT_OUTPUT_LIMIT_BYTES, Ending, ExecRequest,
    GroupBuilder, Host, HostError, HostFs, HostIdentity, HostKey, HostProcess, LeafBuilder,
    PageFeed, PageView, PortError, PortRequest, PortResponse, PortTransport, RpcError, RpcHandler,
    RpcInvocation, RpcPort, ShellEnvironment, ShellError, WholeOutput,
};
use demi_provider_common::{
    InferenceItem, InferenceRequest, ProviderRun, ProviderRuntime, RequestLimits, UserPart,
    testing::{FixedClock, ManualClock, ScriptedRuntime, Turn},
};
use demi_shared_types::{
    Block, Clock, CommandId, ModelSelection, NodeId, ProviderModel, ServiceTier, StreamKind,
    Timestamp, TurnId,
};
use futures_util::{StreamExt, future::LocalBoxFuture, stream};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// What the scenarios' product supplies: one command, the user's subagent
/// settings, which the test can change between two spawns, and a context
/// source, the conversation's execution context, which the test can set.
pub struct TestProduct {
    /// Subagents on, Unlimited, and no profile until the test sets them.
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
    /// Holds the next read of where the conversation works until the gate
    /// opens, as a Host that is slow to answer; the read takes it.
    pub placement_hold: RefCell<Option<Gate>>,
    /// The conversation's Workflow menu choice as its spawns read it.
    pub workflow: RefCell<ConversationProfiles>,
    /// What `demi agent workflows list` reads.
    pub workflows: RefCell<Option<WorkflowListing>>,
    /// The Host's files `demi agent workflows check` reads, by path.
    pub files: RefCell<HashMap<String, String>>,
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
    ) -> LocalBoxFuture<'a, Result<Option<ContextAnswer>, String>> {
        self.asked.set(true);
        let news = (!seen.contains(&self.text.as_str())).then(|| self.text.clone());
        Box::pin(async move { Ok(news.map(ContextAnswer::from)) })
    }
}

impl Default for TestProduct {
    fn default() -> Self {
        Self {
            subagents: RefCell::new(SubagentSettings {
                enabled: true,
                max_live: None,
                max_depth: None,
                profiles: Vec::new(),
            }),
            context: RefCell::default(),
            seen: RefCell::default(),
            placement_hold: RefCell::default(),
            workflow: RefCell::new(ConversationProfiles::Settings),
            workflows: RefCell::default(),
            files: RefCell::default(),
            plugin: None,
            plugin_asked_meanwhile: RefCell::default(),
        }
    }
}

/// No Host: the conversation's directory is `/workspace`.
impl HostResolver for TestProduct {
    type Host = NoHost;

    async fn placement(&self, _context: NodeContext<'_>) -> Result<Placement, HostError> {
        let hold = self.placement_hold.borrow_mut().take();
        if let Some(hold) = hold {
            let mut opened = hold.0.subscribe();
            // The gate lives in the test that holds the read.
            let _ = opened.wait_for(|open| *open).await;
        }
        Ok(Placement {
            device: None,
            directory: "/workspace".to_owned(),
        })
    }
}

impl SubagentSource for TestProduct {
    fn current(&self) -> LocalBoxFuture<'_, Result<SubagentSettings, String>> {
        let settings = self.subagents.borrow().clone();
        Box::pin(async move { Ok(settings) })
    }

    fn conversation_profiles<'a>(
        &'a self,
        _root: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<ConversationProfiles, String>> {
        let profiles = self.workflow.borrow().clone();
        Box::pin(async move { Ok(profiles) })
    }

    fn workflows<'a>(&'a self, _root: &'a NodeId) -> LocalBoxFuture<'a, Result<WorkflowListing, String>> {
        let listing = self.workflows.borrow().clone();
        Box::pin(async move { listing.ok_or_else(|| "no listing".to_owned()) })
    }

    fn read_workflow_file<'a>(
        &'a self,
        _root: &'a NodeId,
        directory: &'a str,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        let text = self.files.borrow().get(&format!("{directory}/{path}")).cloned();
        Box::pin(async move { text.ok_or_else(|| format!("no file {path}")) })
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
    ) -> LocalBoxFuture<'a, Result<Option<ContextAnswer>, String>> {
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
            Ok(news.map(ContextAnswer::from))
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
/// from the root's script, a child's from the script whose key the first
/// user message of its first request contains, in order, from then on by
/// its session, whatever a compaction made of that message. A run beyond
/// its script panics: the test did not script it.
#[derive(Clone, Default)]
pub struct Model(Rc<RefCell<Scripts>>);

#[derive(Default)]
struct Scripts {
    root: VecDeque<Turn>,
    children: Vec<(String, VecDeque<Turn>)>,
    /// The key of each child's script, by the child's session.
    claimed: HashMap<String, String>,
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
        let scripts = self.0.borrow();
        scripts
            .requests
            .iter()
            .filter(|request| {
                scripts.claimed.get(&request.session_id).map(String::as_str) == Some(key)
            })
            .cloned()
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
                let Scripts {
                    children, claimed, ..
                } = &mut *scripts;
                let key = match claimed.get(&request.session_id) {
                    Some(key) => Some(key.clone()),
                    None => {
                        let text = brief(&request);
                        let key = children
                            .iter()
                            .map(|(key, _)| key)
                            .find(|key| text.contains(key.as_str()))
                            .cloned();
                        if let Some(key) = &key {
                            claimed.insert(request.session_id.clone(), key.clone());
                        }
                        key
                    }
                };
                children
                    .iter_mut()
                    .find(|(known, _)| Some(known) == key.as_ref())
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
    /// Whether the calling process reads its stdout; when not, each write
    /// fails, as a relayed pipe that broke does.
    read: bool,
}

impl PortTransport for NodePort {
    fn request(&self, request: PortRequest) -> LocalBoxFuture<'_, Result<PortResponse, PortError>> {
        Box::pin(async move {
            match request {
                PortRequest::Stdout { .. } if !self.read => {
                    Err(PortError::Ended("the stdout pipe broke".into()))
                }
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
                // A test's job never waits for the user.
                PortRequest::AwaitsUser { .. } => Ok(PortResponse::Written {}),
            }
        })
    }
}

/// The `shell` call whose job runs every `demi agent` command a test runs.
pub const AGENT_CALL: &str = "call";

/// Runs `demi agent <verb>` with `args` as a job of `node`'s call
/// [`AGENT_CALL`] would, through the node's commands; `cancel` is the
/// call's cancellation. A verb of a subgroup is its path, such as
/// `profiles list`.
pub async fn agent_call<H: HostResolver>(
    server: &Rc<AgentServer<H>>,
    node: &NodeId,
    verb: &str,
    args: Value,
    json: bool,
    cancel: CancellationToken,
) -> Result<CommandRun, RpcError> {
    let path: Vec<&str> = std::iter::once("agent").chain(verb.split(' ')).collect();
    demi_call(server, &conversation(), node, &path, args, json, cancel, true).await
}

/// `demi agent <verb>` run to its end as a job of `node` of the
/// conversation `root`, another than the scenarios' own.
pub async fn agent_in<H: HostResolver>(
    server: &Rc<AgentServer<H>>,
    root: &NodeId,
    node: &NodeId,
    verb: &str,
    args: Value,
) -> CommandRun {
    let path: Vec<&str> = std::iter::once("agent").chain(verb.split(' ')).collect();
    demi_call(server, root, node, &path, args, false, CancellationToken::new(), true)
        .await
        .expect("the call is dispatched")
}

/// Runs `demi shell <verb>` as a job of `node`, whose call `cancel`
/// cancels; `read` says whether the calling process reads its stdout.
pub async fn shell_call<H: HostResolver>(
    server: &Rc<AgentServer<H>>,
    node: &NodeId,
    verb: &str,
    args: Value,
    cancel: CancellationToken,
    read: bool,
) -> Result<CommandRun, RpcError> {
    demi_call(server, &conversation(), node, &["shell", verb], args, false, cancel, read).await
}

/// `demi shell <verb>` run to its end as a job of `node`.
pub async fn shell_command<H: HostResolver>(
    server: &Rc<AgentServer<H>>,
    node: &NodeId,
    verb: &str,
    args: Value,
) -> CommandRun {
    shell_call(server, node, verb, args, CancellationToken::new(), true)
        .await
        .expect("the call is dispatched")
}

/// Runs `demi <path>` as [`agent_call`] does, as a job of `node` of the
/// conversation `root`.
async fn demi_call<H: HostResolver>(
    server: &Rc<AgentServer<H>>,
    root: &NodeId,
    node: &NodeId,
    path: &[&str],
    args: Value,
    json: bool,
    cancel: CancellationToken,
    read: bool,
) -> Result<CommandRun, RpcError> {
    let live = server.node(root, node).expect("the calling node is live");
    let mut invocation: RpcInvocation = serde_json::from_value(json!({
        "path": std::iter::once("demi").chain(path.iter().copied()).collect::<Vec<_>>(),
        "argv": [],
        "args": args,
        "json": json,
        "host": "laptop",
        "cwd": "/workspace",
        "env": {},
        "context": {
            "conversation": root,
            "caller": { "kind": "agent", "number": live.record().number },
            "locale": { "timeZone": "UTC", "languages": ["en"] },
            "colorScheme": "light",
        },
        "stdin": true,
    }))
    .expect("the invocation is well formed");
    // The backend's record of the job names the node and call that started it.
    invocation.caller = Some(live.job_caller(AGENT_CALL, None));
    let port = Rc::new(NodePort {
        stdout: RefCell::new(Vec::new()),
        stderr: RefCell::new(Vec::new()),
        read,
    });
    let path = invocation.path.join(" ");
    // A failure is told as the backend's dispatcher tells it: a runtime
    // error after the command's path, a usage error whole
    // (`crates/host-interface/src/rpc.rs`).
    let (code, told) = match live
        .commands()
        .dispatch(invocation, RpcPort::new(port.clone(), cancel))
        .await
    {
        Ok(code) => (code, String::new()),
        Err(RpcError::Failed(reason)) => (1, format!("{path}: {reason}\n")),
        Err(RpcError::Usage(text)) => (2, format!("{text}\n")),
        Err(error) => return Err(error),
    };
    let stdout = String::from_utf8(port.stdout.borrow().clone()).expect("stdout is text");
    let stderr = String::from_utf8(port.stderr.borrow().clone()).expect("stderr is text") + &told;
    Ok(CommandRun {
        code,
        stdout,
        stderr,
    })
}

/// `demi agent <verb>` run to its end as a job of `node`.
pub async fn agent<H: HostResolver>(
    server: &Rc<AgentServer<H>>,
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

    /// A server of the provider `stub` playing `script`, on a clock the
    /// test moves.
    pub fn with_clock(script: &ScriptedRuntime) -> (Self, Arc<ManualClock>) {
        let resolver = Rc::new(ScriptedProviders::default());
        resolver.provide("stub", script);
        let clock = Arc::new(ManualClock::new(start()));
        let fixture = Self::build(
            resolver,
            TestProduct::default(),
            MemoryTreeStore::new(),
            ServerConfig::default(),
            clock.clone(),
        );
        (fixture, clock)
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

    fn build(
        resolver: Rc<ScriptedProviders>,
        product: TestProduct,
        store: Rc<MemoryTreeStore>,
        config: ServerConfig,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let product = Rc::new(product);
        // The product's order: its own context, the side questions, then a plugin.
        let told = Rc::new(SideQuestionsSource::default());
        let mut context = vec![product.clone() as Rc<dyn ContextSource>, told.clone()];
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
        told.server.replace(Rc::downgrade(&server));
        Self {
            server,
            store,
            resolver,
            product,
        }
    }

    pub fn client(&self) -> TestClient<TestProduct> {
        TestClient::connect(&self.server, &conversation())
    }

    /// A client that opened the conversation and read its handshake.
    pub async fn opened(&self) -> TestClient<TestProduct> {
        let mut client = self.client();
        client.send(open()).await;
        let handshake = client.next_until(is_waiting).await;
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
pub fn start() -> Timestamp {
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
    ClientFrame::Open {
        from: None,
        edge: None,
    }
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
        comments: Vec::new(),
        message_id: turn(id),
        content: demi_agent_server::testing::client_text(message),
    }
}

pub fn is_pending_steers(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::PendingSteers { .. })
}

/// The handshake's last frame before the usage.
pub fn is_waiting(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::Waiting { .. })
}

pub fn is_context_usage(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::ContextUsage { .. })
}

pub fn is_idle(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::Phase { subagent_id: None, phase } if *phase == demi_shared_types::SessionPhase::Idle)
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

/// The Host the scripted environment runs on; nothing reaches its files or
/// processes.
#[derive(Debug)]
pub struct ScriptedHost;

impl Host for ScriptedHost {
    fn key(&self) -> HostKey {
        HostKey::new("scripted")
    }

    fn default_cwd(&self) -> &str {
        "/workspace"
    }

    fn identity(&self) -> HostIdentity {
        HostIdentity {
            uid: 1000,
            gid: 1000,
            hostname: "scripted".into(),
            home_dir: "/home/demi".into(),
            scratch_root: "/demi/scratch".into(),
            paths: demi_host_interface::HostPaths::Posix,
        }
    }

    fn fs(&self) -> &dyn HostFs {
        unreachable!("the scripted environment reads no file")
    }

    fn process(&self) -> &dyn HostProcess {
        unreachable!("the scripted environment starts no process")
    }
}

/// Where the nodes' shell tools run: the scripted Host, where the
/// conversation works in `/workspace` until the test moves it, as a change
/// of its target in the store does.
#[derive(Default)]
pub struct LiveHosts {
    moved: RefCell<Option<String>>,
}

impl LiveHosts {
    /// The conversation works in `directory` of the scripted Host from now
    /// on.
    pub fn move_to(&self, directory: &str) {
        self.moved.replace(Some(directory.to_owned()));
    }
}

impl HostResolver for LiveHosts {
    type Host = ScriptedHost;

    async fn host(&self, _context: NodeContext<'_>) -> Result<Rc<ScriptedHost>, HostError> {
        Ok(Rc::new(ScriptedHost))
    }

    async fn placement(&self, _context: NodeContext<'_>) -> Result<Placement, HostError> {
        Ok(Placement {
            device: Some("scripted".to_owned()),
            directory: self.moved.borrow().clone().unwrap_or_else(|| "/workspace".to_owned()),
        })
    }
}

/// One node's environment: its command runs until the test ends it, and
/// prints what the test hands it, which it reports to the node's feed. Like
/// a runner that has not answered yet, a stop only marks the command
/// stopping: it ends when the test says (`crates/runner-shell/src/job.rs`). Commands started while one runs, as the parallel calls of
/// one response, run beside it, numbered after it.
pub struct ScriptedShell {
    /// The directory each of its jobs started in, in order; none for one
    /// that starts where the Host starts its work.
    pub started_in: RefCell<Vec<Option<String>>>,
    feed: Rc<dyn PageFeed>,
    /// Its commands, the first being the one the helpers below act on.
    commands: RefCell<Vec<Rc<RefCell<CommandRecord>>>>,
    /// Changes when the command ends.
    ends: watch::Sender<()>,
    /// Set once the environment let go of its command, which then runs on
    /// without it: it learns of no end.
    detached: Cell<bool>,
}

impl ScriptedShell {
    fn record(&self, command: &CommandId) -> Result<Rc<RefCell<CommandRecord>>, ShellError> {
        self.commands
            .borrow()
            .iter()
            .find(|record| record.borrow().command_id() == command)
            .cloned()
            .ok_or_else(|| ShellError::UnknownCommand(command.clone()))
    }

    /// The command the helpers act on: the first one started.
    fn first(&self) -> Rc<RefCell<CommandRecord>> {
        self.commands.borrow().first().cloned().expect("the command started")
    }

    /// Holds `record`: beside the commands that run, or in place of those
    /// that ended.
    fn hold(&self, record: &Rc<RefCell<CommandRecord>>) {
        let running = self.commands.borrow().iter().any(|command| command.borrow().is_running());
        if !running {
            self.commands.borrow_mut().clear();
        }
        self.commands.borrow_mut().push(record.clone());
        self.feed.changed(record);
    }

    /// The command `command`'s start waits for its Host, the device `host`
    /// names, whose runner is away; with none, it no longer waits.
    pub fn wait_for(&self, command: &str, host: Option<&str>) {
        let record = self
            .record(&CommandId::try_from(command).unwrap())
            .expect("the command started");
        if record.borrow_mut().mark_waiting(host.map(str::to_owned)) {
            self.feed.changed(&record);
        }
    }

    /// The command `command` exits.
    pub fn exit(&self, command: &str) {
        let record = self
            .record(&CommandId::try_from(command).unwrap())
            .expect("the command started");
        record
            .borrow_mut()
            .settle(Ending::Exited(0), Arc::new(WholeOutput::default()), None, Vec::new(), "");
        self.feed.changed(&record);
        self.ends.send_replace(());
    }

    /// The running command prints `text`.
    pub fn print(&self, text: &str) {
        let record = self.first();
        record.borrow_mut().append_output(StreamKind::Stdout, text);
        self.feed.changed(&record);
    }

    /// The running command exits, having changed `files`, as its runner
    /// reports a job's exit with its edit report.
    pub fn exit_with(&self, files: Vec<demi_shared_types::EditedFile>) {
        let record = self.first();
        let mut command = record.borrow_mut();
        command.set_files(demi_host_interface::EditedFiles {
            files,
            path_changes: Vec::new(),
            truncated: false,
        });
        command.settle(Ending::Exited(0), Arc::new(WholeOutput::default()), None, Vec::new(), "");
        drop(command);
        self.feed.changed(&record);
        self.ends.send_replace(());
    }

    /// The running command is lost, as when its runner started anew.
    pub fn lose(&self) {
        let record = self.first();
        record.borrow_mut().settle(
            Ending::Lost("the Host's runner started anew".to_owned()),
            Arc::new(WholeOutput::default()),
            None,
            Vec::new(),
            "",
        );
        self.feed.changed(&record);
        self.ends.send_replace(());
    }

    /// The running command ends, as its runner reports a stop's end.
    pub fn end(&self) {
        assert!(!self.detached.get(), "the environment let go of its command");
        let record = self.first();
        record.borrow_mut().settle(
            Ending::Aborted,
            Arc::new(WholeOutput::default()),
            None,
            Vec::new(),
            "",
        );
        self.feed.changed(&record);
        self.ends.send_replace(());
    }
}

impl ShellEnvironment for ScriptedShell {
    fn start(
        &self,
        request: ExecRequest,
        _cancel: CancellationToken,
    ) -> LocalBoxFuture<'_, Result<CommandId, ShellError>> {
        self.started_in.borrow_mut().push(request.cwd.clone());
        let running = self.commands.borrow().iter().filter(|command| command.borrow().is_running()).count();
        let command = CommandId::try_from((running + 1).to_string()).unwrap();
        let record = Rc::new(RefCell::new(CommandRecord::new(
            command.clone(),
            request.caller.tool_use_id,
            request.title,
            &request.script,
        )));
        self.hold(&record);
        Box::pin(async move { Ok(command) })
    }

    /// Waits for the end the test gives.
    fn ended<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<Ending, ShellError>> {
        let record = self.record(command);
        let mut ends = self.ends.subscribe();
        Box::pin(async move {
            let record = record?;
            loop {
                if let Some(ending) = record.borrow().ending() {
                    return Ok(ending);
                }
                // The sender lives as long as this environment, which the
                // caller holds.
                let _ = ends.changed().await;
            }
        })
    }

    fn title(&self, command: &CommandId) -> Option<String> {
        Some(self.record(command).ok()?.borrow().title().to_owned())
    }

    fn status(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        let record = self.record(command)?;
        let status = record.borrow_mut().status(DEFAULT_OUTPUT_LIMIT_BYTES, None, Vec::new());
        Ok(status)
    }

    fn quiet(&self, command: &CommandId) -> Result<Duration, ShellError> {
        Ok(self.record(command)?.borrow().quiet())
    }

    fn awaits_user(&self, _: &CommandId) -> Option<tokio::sync::watch::Receiver<bool>> {
        None
    }

    fn outliving(&self, _: &CommandId) -> Vec<String> {
        Vec::new()
    }

    fn unreachable(&self, _: &CommandId) -> Option<demi_host_interface::Unreachable> {
        None
    }

    fn media(&self, command: &CommandId) -> Result<Vec<demi_host_interface::CommandMedium>, ShellError> {
        Ok(self.record(command)?.borrow().media().to_vec())
    }

    /// Its Host keeps none of the running command's output.
    fn read_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<WholeOutput, ShellError>> {
        let kept = self.record(command).and_then(|record| {
            if record.borrow().is_running() {
                Ok(WholeOutput::default())
            } else {
                Err(ShellError::NotRunning(command.clone()))
            }
        });
        Box::pin(std::future::ready(kept))
    }

    /// Takes input while the command runs, and keeps none of it.
    fn write<'a>(
        &'a self,
        command: &'a CommandId,
        _stdin: Bytes,
    ) -> LocalBoxFuture<'a, Result<(), ShellError>> {
        let taken = self.record(command).and_then(|record| {
            if record.borrow().is_running() {
                Ok(())
            } else {
                Err(ShellError::NotRunning(command.clone()))
            }
        });
        Box::pin(std::future::ready(taken))
    }

    /// Marks the command stopping, as a stop that its runner has not
    /// answered yet.
    fn abort<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<(), ShellError>> {
        let asked = self.record(command).map(|record| {
            if record.borrow_mut().mark_stopping() {
                self.feed.changed(&record);
            }
        });
        Box::pin(std::future::ready(asked))
    }

    fn page_views(&self) -> Vec<PageView> {
        self.commands
            .borrow()
            .iter()
            .map(|record| record.borrow().page_view())
            .collect()
    }

    fn release_command<'a>(&'a self, _command: &'a CommandId) -> LocalBoxFuture<'a, bool> {
        Box::pin(async { false })
    }

    /// Ends its commands whose start waits, never sent.
    fn stop_waiting(&self) -> LocalBoxFuture<'_, ()> {
        let waiting: Vec<Rc<RefCell<CommandRecord>>> = self
            .commands
            .borrow()
            .iter()
            .filter(|record| record.borrow().is_waiting())
            .cloned()
            .collect();
        for record in waiting {
            record
                .borrow_mut()
                .settle(Ending::Aborted, Arc::new(WholeOutput::default()), None, Vec::new(), "");
            self.feed.changed(&record);
            self.ends.send_replace(());
        }
        Box::pin(async {})
    }

    /// Takes up the command a restore names, as a runner that kept it.
    fn adopt(&self, taken: demi_host_interface::TakenUp) {
        let record = Rc::new(RefCell::new(CommandRecord::taken_up(
            taken.command,
            taken.caller.tool_use_id,
            taken.title,
            &taken.script,
            taken.running,
            taken.seen,
        )));
        self.hold(&record);
    }

    fn detach_all(&self) -> LocalBoxFuture<'_, ()> {
        self.detached.set(true);
        Box::pin(async {})
    }

    /// Stops the command and waits for its end, as a real environment does.
    fn dispose_all(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async move {
            let commands: Vec<CommandId> = self
                .commands
                .borrow()
                .iter()
                .map(|record| record.borrow().command_id().clone())
                .collect();
            for command in commands {
                // It holds the commands it names.
                let _ = self.abort(&command).await;
                let _ = self.ended(&command).await;
            }
        })
    }

    fn owns_command(&self, command: &CommandId) -> bool {
        self.record(command).is_ok()
    }
}

/// A server whose nodes' shells run on the scripted Host, each in a
/// [`ScriptedShell`] that `shells` keeps, with subagents on and Unlimited,
/// answered by the provider entry `stub` of `providers`.
pub fn scripted_host_server(
    providers: Rc<ScriptedProviders>,
    shells: Rc<ScriptedShells>,
    store: Rc<MemoryTreeStore>,
) -> Rc<AgentServer<LiveHosts>> {
    scripted_host_server_in(providers, shells, store, Rc::default())
}

/// [`scripted_host_server`], whose conversation works where `hosts` says.
pub fn scripted_host_server_in(
    providers: Rc<ScriptedProviders>,
    shells: Rc<ScriptedShells>,
    store: Rc<MemoryTreeStore>,
    hosts: Rc<LiveHosts>,
) -> Rc<AgentServer<LiveHosts>> {
    scripted_host_server_telling(providers, shells, store, hosts, Rc::new(|_| {}))
}

/// [`scripted_host_server_in`], which tells `status_changed` of each change
/// the product hears of a tree's work.
pub fn scripted_host_server_telling(
    providers: Rc<ScriptedProviders>,
    shells: Rc<ScriptedShells>,
    store: Rc<MemoryTreeStore>,
    hosts: Rc<LiveHosts>,
    status_changed: Rc<dyn Fn(&NodeId)>,
) -> Rc<AgentServer<LiveHosts>> {
    AgentServer::new(ServerDeps {
        toolsets: Rc::new(Toolset {
            commands: Rc::new(CommandSet::new()),
            revision: Rc::from("none"),
        }),
        subagents: Rc::new(SubagentSettings {
            enabled: true,
            max_live: None,
            max_depth: None,
            profiles: Vec::new(),
        }),
        instructions: Rc::from("system prompt"),
        guide: Rc::from("harness guide"),
        hosts,
        context: Rc::new([]),
        providers,
        shells,
        stores: Rc::new(move |_: &NodeId| store.clone() as Rc<dyn AgentTreeStore>),
        clock: Arc::new(FixedClock(start())),
        ids: Rc::new(SequentialIds::new("id")),
        config: ServerConfig::default(),
        status_changed,
    })
}

/// Makes each node's environment and keeps each one made, in order, for the
/// test to print through and end; while the test sets a gate, each waits
/// until it opens, as a Host that is slow to answer a take-up.
#[derive(Default)]
pub struct ScriptedShells(pub RefCell<Vec<Rc<ScriptedShell>>>, pub RefCell<Option<Gate>>);

impl ScriptedShells {
    /// The environment made last.
    pub fn last(&self) -> Rc<ScriptedShell> {
        self.0.borrow().last().cloned().expect("a call made an environment")
    }
}

impl ShellEnvironmentFactory<ScriptedHost> for ScriptedShells {
    fn create<'a>(
        &'a self,
        scope: EnvironmentScope<'a>,
        _host: Rc<ScriptedHost>,
    ) -> LocalBoxFuture<'a, Result<Rc<dyn ShellEnvironment>, HostError>> {
        let shell = Rc::new(ScriptedShell {
            started_in: RefCell::default(),
            feed: scope.feed.clone(),
            commands: RefCell::default(),
            ends: watch::Sender::new(()),
            detached: Cell::new(false),
        });
        self.0.borrow_mut().push(shell.clone());
        let gate = self.1.borrow().as_ref().map(|gate| gate.0.subscribe());
        Box::pin(async move {
            if let Some(mut opened) = gate {
                // The gate lives in the test that holds the take-up.
                let _ = opened.wait_for(|open| *open).await;
            }
            Ok(shell as Rc<dyn ShellEnvironment>)
        })
    }
}

/// The side questions source as the product's: what the root's tree tells
/// of its side questions.
#[derive(Default)]
pub struct SideQuestionsSource {
    server: RefCell<Weak<AgentServer<TestProduct>>>,
}

impl ContextSource for SideQuestionsSource {
    fn name(&self) -> &str {
        demi_shared_types::SIDE_QUESTIONS_SOURCE
    }

    fn context<'a>(
        &'a self,
        node: demi_agent_tools::NodeContext<'a>,
        _: &'a TurnId,
        seen: &'a [&'a str],
    ) -> LocalBoxFuture<'a, Result<Option<demi_agent_tools::ContextAnswer>, String>> {
        let tree = self.server.borrow().upgrade().and_then(|server| server.tree(node.root));
        Box::pin(async move {
            let Some(tree) = tree else {
                return Ok(None);
            };
            let told = tree.side_questions_context(node.node, seen).await.map_err(|error| error.to_string())?;
            Ok(told.map(demi_agent_tools::ContextAnswer::from))
        })
    }
}
