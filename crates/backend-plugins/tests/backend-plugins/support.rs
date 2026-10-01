//! What the scenarios share: a real runner for one device, the `demi.file`
//! package the workspace built, and an agent server whose conversations run
//! on that device with the command set the plugin host composes from the
//! `file` and `todo` plugins, their shells real runner jobs and their model a
//! script.

use std::{
    cell::RefCell, collections::BTreeMap, future::Future, rc::Rc, sync::Arc, time::Duration,
};

use demi_agent_server::{
    AgentServer, ServerConfig, ServerDeps,
    testing::{ScriptedProviders, TestClient, client_text},
};
use demi_agent_store::{AgentTreeStore, testing::MemoryTreeStore};
use demi_agent_tools::{EnvironmentScope, HostResolver, NodeContext, ShellEnvironmentFactory};
use demi_agent_transcript::testing::SequentialIds;
use demi_backend_plugins::Registry;
use demi_backend_remote_host::{
    CommandCatalog, ContextSource, EnvironmentOptions, RemoteHost, RemoteShellEnvironmentFactory,
    testing::{FixtureOptions, NativeFixture, RunnerFixture},
};
use demi_command_protocol::testing::built_program;
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_host_interface::{
    CommandSet, HostError, HostErrorKind, ShellEnvironment, testing::test_command_context,
};
use demi_plugin_file::File;
use demi_plugin_interface::PluginFactory;
use demi_plugin_todo::Todo;
use demi_provider_common::{
    InferenceItem, InferenceRequest, ProviderEvent, ResultPart,
    testing::{ScriptedRuntime, TokioClock, Turn, event},
};
use demi_shared_types::{Block, CommandId, NodeId, SessionPhase, ToolView, TurnId};
use futures_util::future::LocalBoxFuture;
use serde_json::json;

/// The conversation's Host: the fixture's device, working in the directory
/// the test chose last.
pub struct DeviceHost(Rc<RefCell<Rc<RemoteHost>>>);

impl HostResolver for DeviceHost {
    type Host = RemoteHost;

    async fn host(&self, _context: NodeContext<'_>) -> Result<Rc<RemoteHost>, HostError> {
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
        let mut options = EnvironmentOptions::new(
            RemoteHost::clone(&host),
            context,
            scope.feed.clone(),
            scope.numbers.clone(),
        );
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
    pub server: Rc<AgentServer<DeviceHost>>,
    /// The conversation's working directory on the device, inside the
    /// runner's home.
    pub workspace: String,
    /// The Host the conversation's nodes reach now.
    host: Rc<RefCell<Rc<RemoteHost>>>,
}

impl Fixture {
    /// A fixture whose model plays `script`.
    pub async fn start(script: &ScriptedRuntime) -> Self {
        Self::start_with(script, ServerConfig::default()).await
    }

    /// A fixture whose model plays `script`, with the server's `config`.
    pub async fn start_with(script: &ScriptedRuntime, config: ServerConfig) -> Self {
        let commands = demi_commands();
        let runner = RunnerFixture::start(FixtureOptions {
            commands: commands.clone(),
            ..FixtureOptions::default()
        })
        .await;
        let workspace = format!("{}/workspace", runner.home());
        std::fs::create_dir_all(&workspace).unwrap();
        let host = Rc::new(RefCell::new(Rc::new(runner.host_at(&workspace))));
        let providers = Rc::new(ScriptedProviders::default());
        providers.provide("stub", script);
        let file = NativeFixture::package(
            demi_command_package_file_protocol::PACKAGE,
            built_program("demi-file"),
            demi_command_package_file_protocol::OPERATIONS
                .iter()
                .copied(),
        );
        let catalog = CommandCatalog::new(vec![file.descriptor.clone()], file.resolver()).unwrap();
        let store = MemoryTreeStore::new();
        let server = AgentServer::new(ServerDeps {
            commands: Rc::new(commands),
            instructions: Rc::from("You are a coding agent."),
            profiles: Rc::new([]),
            hosts: Rc::new(DeviceHost(host.clone())),
            context: Rc::new([]),
            providers,
            shells: Rc::new(RunnerShells(RemoteShellEnvironmentFactory::new(catalog))),
            stores: Rc::new(move |_: &NodeId| store.clone() as Rc<dyn AgentTreeStore>),
            clock: Arc::new(TokioClock::new("2026-09-24T12:00:00Z".parse().unwrap())),
            ids: Rc::new(SequentialIds::new("id")),
            config,
            // No product shows the conversations' statuses.
            status_changed: Rc::new(|_| {}),
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
    pub async fn attach(&self) -> TestClient<DeviceHost> {
        let client = TestClient::connect(&self.server, &conversation(), &self.workspace);
        client.send(ClientFrame::Open {}).await;
        client
    }

    /// A client that opened the conversation and read its handshake.
    pub async fn opened(&self) -> TestClient<DeviceHost> {
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

/// The `demi` commands the plugin host composes from the `file` and `todo`
/// plugins, which the device's jobs call back into.
fn demi_commands() -> CommandSet {
    let plugins: Vec<Box<dyn PluginFactory>> = vec![Box::new(File::new()), Box::new(Todo::new())];
    Registry::new(plugins, |_| true)
        .unwrap()
        .instances("u1")
        .commands(Vec::new())
        .unwrap()
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
pub async fn turn(client: &mut TestClient<DeviceHost>, id: &str, text: &str) -> Vec<ServerFrame> {
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
        .filter_map(|part| match part {
            ResultPart::Text(text) => Some(text.as_str()),
            ResultPart::Image(_) | ResultPart::Video(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
