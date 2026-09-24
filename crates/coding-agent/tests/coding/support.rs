//! What the scenarios share: a real runner for one device, the `demi.builtin`
//! package the workspace built, and an agent server whose conversations run
//! the coding harness on that device, their shells real runner jobs and
//! their model a script.

use std::{
    cell::RefCell, collections::BTreeMap, future::Future, rc::Rc, sync::Arc, time::Duration,
};

use demi_agent::{
    AgentServer, AgentTreeStore, EnvironmentScope, PromptContext, ServerConfig, ServerDeps,
    ShellEnvironmentFactory,
    testing::{
        MemoryTreeStore, ScriptedProviders, SequentialIds, TestClient, TokioClock, client_text,
        test_model,
    },
};
use demi_agent_protocol::{ClientFrame, ServerFrame};
use demi_builtin_protocol::Operation;
use demi_coding_agent::{BUILTIN_PACKAGE, CodingHarness, DemiOptions, HostResolver, demi_root};
use demi_core::{Block, CommandId, NodeId, SessionPhase, ToolResultContentBlock, ToolView, TurnId};
use demi_host_remote::{
    CommandCatalog, ContextSource, EnvironmentOptions, RemoteHost, RemoteShellEnvironmentFactory,
    testing::{FixtureOptions, NativeFixture, RunnerFixture, built_program},
};
use demi_provider::{
    InferenceItem, InferenceRequest, ProviderEvent,
    testing::{ScriptedRuntime, Turn, event},
};
use demi_shell::{
    CommandSet, HostError, HostErrorKind, ShellEnvironment, testing::test_command_context,
};
use futures_util::future::LocalBoxFuture;
use serde_json::json;

pub type Harness = CodingHarness<DeviceHost>;

/// The conversation's Host: the fixture's device, working in the directory
/// the test chose last.
pub struct DeviceHost(Rc<RefCell<Rc<RemoteHost>>>);

impl HostResolver for DeviceHost {
    type Host = RemoteHost;

    async fn host(&self, _context: PromptContext<'_>) -> Result<Rc<RemoteHost>, HostError> {
        Ok(self.0.borrow().clone())
    }
}

/// The conversations' shells: runner jobs on the device that start with a
/// plain `PATH` and may run the node's commands.
struct RunnerShells(RemoteShellEnvironmentFactory);

impl ShellEnvironmentFactory<RemoteHost> for RunnerShells {
    fn create<'a>(
        &'a self,
        scope: EnvironmentScope<'a>,
        host: Rc<RemoteHost>,
    ) -> LocalBoxFuture<'a, Result<Rc<dyn ShellEnvironment>, HostError>> {
        let context: ContextSource = Rc::new(|| Box::pin(async { Ok(test_command_context()) }));
        let mut options = EnvironmentOptions::new(RemoteHost::clone(&host), context);
        options.initial_env = BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]);
        let made = self
            .0
            .create(scope.commands, options)
            .map(|environment| Rc::new(environment) as Rc<dyn ShellEnvironment>)
            .map_err(|error| {
                HostError::new(HostErrorKind::Failed { code: None }, error.to_string())
            });
        Box::pin(async move { made })
    }
}

/// A device's runner and an agent server whose conversations work on it.
pub struct Fixture {
    pub runner: RunnerFixture,
    pub server: Rc<AgentServer<Harness>>,
    /// The conversation's working directory on the device, inside the
    /// runner's home.
    pub workspace: String,
    /// The Host the conversation's nodes reach now.
    host: Rc<RefCell<Rc<RemoteHost>>>,
}

impl Fixture {
    /// A fixture whose model plays `script`.
    pub async fn start(script: &ScriptedRuntime) -> Self {
        let runner = RunnerFixture::start(FixtureOptions {
            commands: demi_commands(),
            ..FixtureOptions::default()
        })
        .await;
        let workspace = format!("{}/workspace", runner.home());
        std::fs::create_dir_all(&workspace).unwrap();
        let host = Rc::new(RefCell::new(Rc::new(runner.host_at(&workspace))));
        let harness = CodingHarness::new(DeviceHost(host.clone()), DemiOptions::default()).unwrap();
        let providers = Rc::new(ScriptedProviders::default());
        providers.provide("stub", script);
        let builtin = NativeFixture::package(
            BUILTIN_PACKAGE,
            built_program("demi-commands"),
            Operation::names(),
        );
        let catalog =
            CommandCatalog::new(vec![builtin.descriptor.clone()], builtin.resolver()).unwrap();
        let store = MemoryTreeStore::new();
        let server = AgentServer::new(ServerDeps {
            harness: Rc::new(harness),
            providers,
            shells: Rc::new(RunnerShells(RemoteShellEnvironmentFactory::new(catalog))),
            stores: Rc::new(move |_: &NodeId| store.clone() as Rc<dyn AgentTreeStore>),
            clock: Arc::new(TokioClock::new("2026-09-24T12:00:00Z".parse().unwrap())),
            ids: Rc::new(SequentialIds::new("id")),
            config: ServerConfig::default(),
        });
        Self {
            runner,
            server,
            workspace,
            host,
        }
    }

    /// Makes the conversation's Host the device working in `directory`, a
    /// new directory in the runner's home: another Host, as after a target
    /// switch.
    pub fn work_in(&self, directory: &str) -> String {
        let path = format!("{}/{directory}", self.runner.home());
        std::fs::create_dir_all(&path).unwrap();
        self.host.replace(Rc::new(self.runner.host_at(&path)));
        path
    }

    /// A client that opened the conversation; its handshake waits unread.
    pub async fn attach(&self) -> TestClient<Harness> {
        let client = TestClient::connect(&self.server, &conversation(), &self.workspace);
        client
            .send(ClientFrame::Open {
                model: test_model(),
            })
            .await;
        client
    }

    /// A client that opened the conversation and read its handshake.
    pub async fn opened(&self) -> TestClient<Harness> {
        let mut client = self.attach().await;
        let handshake = client
            .next_until(|frame| matches!(frame, ServerFrame::PendingSteers { .. }))
            .await;
        assert_eq!(
            handshake.first(),
            Some(&ServerFrame::Opened),
            "{handshake:?}"
        );
        client
    }

    /// The root's transcript: each block's type, and a tool call's status
    /// beside it.
    pub fn kinds(&self) -> Vec<String> {
        let root = conversation();
        let node = self.server.node(&root, &root).unwrap();
        node.session()
            .transcript()
            .blocks
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

    /// The command of each of the root's shell tool calls, in order, as the
    /// transcript's views name them for the user.
    pub fn shell_commands(&self) -> Vec<CommandId> {
        let root = conversation();
        let node = self.server.node(&root, &root).unwrap();
        node.session()
            .transcript()
            .blocks
            .iter()
            .filter_map(|block| match block {
                Block::ToolCall(call) => match &call.view {
                    Some(ToolView::Shell(view)) => Some(view.command_id.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// Ends the conversations, then the runner.
    pub async fn stop(self) {
        self.server.shutdown().await;
        self.runner.stop().await;
    }
}

/// The `demi` commands the device's jobs call back into.
fn demi_commands() -> CommandSet {
    let mut commands = CommandSet::new();
    commands
        .register(demi_root(DemiOptions::default()))
        .unwrap();
    commands
}

/// Runs a scenario, failing it when it does not end within a minute.
pub async fn within<T>(scenario: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), scenario)
        .await
        .expect("the scenario ends within a minute")
}

pub fn conversation() -> NodeId {
    NodeId::try_from("conversation").unwrap()
}

/// Sends the user message `text` as `id` and returns the frames up to the
/// idle phase that ends its turn.
pub async fn turn(client: &mut TestClient<Harness>, id: &str, text: &str) -> Vec<ServerFrame> {
    client
        .send(ClientFrame::Send {
            message_id: TurnId::try_from(id).unwrap(),
            content: client_text(text),
        })
        .await;
    client.next_until(is_idle).await
}

pub fn is_idle(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::Phase { phase } if *phase == SessionPhase::Idle)
}

/// A `shell_exec` call of `script` that watches it for up to `timeout_ms`.
pub fn exec(id: &str, script: &str, timeout_ms: u32) -> ProviderEvent {
    event::tool_call(
        id,
        "shell_exec",
        json!({"description": "Workspace state", "script": script, "timeoutMs": timeout_ms}),
    )
}

/// The end of a turn: a text and a response.
pub fn reply(text: &str) -> Vec<ProviderEvent> {
    vec![event::text(text), event::response(10, 5)]
}

/// A model's runs for messages that each run scripts with `shell_exec`, one
/// call per request in one shell, and end the message's turn after its last
/// script; every call's result text lands in the returned list.
pub fn scripts(messages: &[&[&str]]) -> (Vec<Turn>, Rc<RefCell<Vec<String>>>) {
    let results = Rc::new(RefCell::new(Vec::new()));
    let mut turns = Vec::new();
    let mut step = 0;
    for scripts in messages {
        let (first, rest) = scripts.split_first().expect("a message runs a script");
        turns.push(Turn::Events(vec![exec(
            &format!("step-{step}"),
            first,
            30_000,
        )]));
        for script in rest {
            step += 1;
            let results = results.clone();
            let call = exec(&format!("step-{step}"), script, 30_000);
            turns.push(Turn::Respond(Box::new(move |request| {
                results.borrow_mut().push(last_result(request));
                vec![call]
            })));
        }
        step += 1;
        let results = results.clone();
        turns.push(Turn::Respond(Box::new(move |request| {
            results.borrow_mut().push(last_result(request));
            reply("done")
        })));
    }
    (turns, results)
}

/// The text of the newest tool result a request carries.
pub fn last_result(request: &InferenceRequest) -> String {
    let output = request
        .items
        .iter()
        .rev()
        .find_map(|item| match item {
            InferenceItem::ToolResult { output, .. } => Some(output),
            _ => None,
        })
        .expect("the request carries a tool result");
    output
        .iter()
        .filter_map(|block| match block {
            ToolResultContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The value of a result's `name: value` line.
pub fn field<'a>(result: &'a str, name: &str) -> &'a str {
    result
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(": "))
        .unwrap_or_else(|| panic!("the result has no {name}:\n{result}"))
}

/// The output preview of a result, empty when it shows none.
pub fn preview(result: &str) -> &str {
    let Some((_, preview)) = result.split_once("\npreview:\n") else {
        return "";
    };
    preview.split("\nnext: ").next().unwrap_or(preview)
}
