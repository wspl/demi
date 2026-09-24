//! What the scenarios share: a harness, a provider resolver over scripted
//! runtimes, a server over an in-memory tree store, and frame builders.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

use demi_agent::{
    AgentHarness, AgentServer, AgentTreeStore, PromptContext, ProviderResolver, ResolveError,
    ServerConfig, ServerDeps,
    testing::{MemoryTreeStore, NoHost, NoShells, SequentialIds, TestClient, test_model},
};
use demi_agent_protocol::{ClientFrame, ServerFrame};
use demi_core::{Block, ModelSelection, NodeId, TurnId};
use demi_provider::{
    ProviderRuntime,
    testing::{FixedClock, ScriptedRuntime},
};
use demi_shell::{
    CommandSet, GroupBuilder, LeafBuilder, RpcError, RpcHandler, RpcInvocation, RpcPort,
};
use futures_util::future::LocalBoxFuture;

/// The harness the scenarios run: fixed texts, one command, and a context
/// text the test can set for the next request.
#[derive(Default)]
pub struct TestHarness {
    pub preamble: Option<String>,
    /// Handed out once, before the next request.
    pub context: RefCell<VecDeque<String>>,
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

    async fn system_prompt(&self, _context: PromptContext<'_>, commands: &str) -> String {
        self.prompts.borrow_mut().push(commands.to_owned());
        "system prompt".to_owned()
    }

    async fn preamble(&self, _context: PromptContext<'_>) -> Option<String> {
        self.preamble.clone()
    }

    async fn context(&self, _context: PromptContext<'_>) -> Option<String> {
        self.context.borrow_mut().pop_front()
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

/// Runtimes by provider id: every runtime of one provider plays that
/// provider's one script.
#[derive(Default)]
pub struct Resolver {
    runtimes: RefCell<HashMap<String, ScriptedRuntime>>,
    /// Every resolution asked for: the conversation and the provider.
    pub calls: RefCell<Vec<(NodeId, String)>>,
}

impl Resolver {
    pub fn provide(&self, provider: &str, runtime: &ScriptedRuntime) {
        self.runtimes
            .borrow_mut()
            .insert(provider.to_owned(), runtime.clone());
    }
}

impl ProviderResolver for Resolver {
    fn runtime<'a>(
        &'a self,
        root: &'a NodeId,
        model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<Box<dyn ProviderRuntime>, ResolveError>> {
        self.calls
            .borrow_mut()
            .push((root.clone(), model.provider_id.clone()));
        let runtime = self.runtimes.borrow().get(&model.provider_id).cloned();
        let provider = model.provider_id.clone();
        Box::pin(async move {
            let runtime = runtime.ok_or(ResolveError::Unknown(provider))?;
            Ok(Box::new(runtime) as Box<dyn ProviderRuntime>)
        })
    }
}

/// A server with its store, resolver and harness in reach.
pub struct Fixture {
    pub server: Rc<AgentServer<TestHarness>>,
    pub store: Rc<MemoryTreeStore>,
    pub resolver: Rc<Resolver>,
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
        let resolver = Rc::new(Resolver::default());
        resolver.provide("stub", script);
        let harness = Rc::new(TestHarness::default());
        let stores = {
            let store = store.clone();
            Rc::new(move |_: &NodeId| store.clone() as Rc<dyn AgentTreeStore>)
        };
        let server = AgentServer::new(ServerDeps {
            harness: harness.clone(),
            providers: resolver.clone(),
            shells: Rc::new(NoShells),
            stores,
            clock: Arc::new(FixedClock(
                "2026-09-24T12:00:00.000Z"
                    .parse()
                    .expect("the time is RFC 3339"),
            )),
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
