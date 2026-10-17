//! What the scenarios share: a real runner for one device, the `demi.file`
//! package the workspace built, and an agent server whose conversations run
//! on that device with the command set the plugin host composes from the
//! `file` plugin, their shells real runner jobs and their model a script.

use std::{
    cell::RefCell, collections::BTreeMap, future::Future, rc::Rc, sync::Arc, time::Duration,
};

use demi_agent_server::{
    AgentServer, ServerConfig, ServerDeps,
    testing::{ScriptedProviders, TestClient, client_text},
};
use demi_agent_store::{AgentTreeStore, testing::MemoryTreeStore};
use demi_agent_tools::{
    EnvironmentScope, HostResolver, NodeContext, ShellEnvironmentFactory, SubagentSettings, Toolset,
};
use demi_agent_transcript::testing::SequentialIds;
use demi_backend_database::control::ControlService;
use demi_backend_providers::vault::seal::{Row, VaultKey};
use demi_backend_page_sync::SyncRegistry;
use demi_backend_plugins::{PageCall, PageCallError, ProductPort, Registry, UserPlugins};
use demi_backend_remote_host::{
    CommandCatalog, ContextSource, EnvironmentOptions, RemoteHost, RemoteShellEnvironmentFactory,
    testing::{FixtureOptions, NativeFixture, RunnerFixture},
};
use demi_command_declarations::NativeOperation;
use demi_command_protocol::testing::built_program;
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_host_interface::{
    CommandSet, HostError, HostErrorKind, PortError, ShellEnvironment,
    testing::test_command_context,
};
use demi_plugin_file::File;
use demi_plugin_interface::testing::loopback;
use demi_plugin_interface::{
    CallKind, HostFile, HostRead, Manifest, Method, OAuth, Page, Plugin, PluginError,
    PluginFactory, PluginId, PluginPort, PortFailure, PortRefusal, Reply, Request, Scope,
    SecretName, WebMethod, WebRequest,
};
use demi_provider_common::{
    InferenceItem, InferenceRequest, ProviderEvent, ResultPart, UserPart,
    testing::{ManualClock, ScriptedRuntime, TokioClock, Turn, event},
};
use demi_shared_types::SystemClock;
use demi_shared_types::{B64Bytes, BlobRef};
use demi_shared_types::{Block, CommandId, NodeId, SessionPhase, ToolView, TurnId};
use demi_web_api_protocol::ids::{ConversationId, UserId};
use futures_util::future::LocalBoxFuture;
use serde_json::json;
use serde_json::{Map, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

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
    /// The model's provider, whose selection a test may choose before the
    /// conversation opens.
    pub providers: Rc<ScriptedProviders>,
    /// Holds the plugins' control database.
    _data: TempDir,
}

impl Fixture {
    /// A fixture whose model plays `script`.
    pub async fn start(script: &ScriptedRuntime) -> Self {
        Self::start_with(script, ServerConfig::default()).await
    }

    /// A fixture whose model plays `script`, with the server's `config`.
    pub async fn start_with(script: &ScriptedRuntime, config: ServerConfig) -> Self {
        let (commands, data) = demi_commands().await;
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
            toolsets: Rc::new(Toolset {
                commands: Rc::new(commands),
                revision: Rc::from("file"),
            }),
            subagents: Rc::new(SubagentSettings {
                enabled: true,
                max_live: None,
                max_depth: None,
                profiles: Vec::new(),
            }),
            instructions: Rc::from("You are a coding agent."),
            guide: Rc::from("How Demi works."),
            hosts: Rc::new(DeviceHost(host.clone())),
            context: Rc::new([]),
            providers: providers.clone(),
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
            providers,
            _data: data,
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
        let client = TestClient::connect(&self.server, &conversation());
        client.send(ClientFrame::Open { from: None, edge: None }).await;
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

/// The `demi` commands the plugin host composes from the `file` plugin,
/// which the device's jobs call back into.
async fn demi_commands() -> (CommandSet, TempDir) {
    let plugins: Vec<Box<dyn PluginFactory>> = vec![Box::new(File::new())];
    let (plugins, data) = user_plugins(Registry::new(plugins, |_| true).unwrap()).await;
    let toolset = plugins.toolset(Vec::new()).await.unwrap();
    (toolset.commands, data)
}

/// The plugins of `registry` for the user `u1`, over a control database of
/// their own, which the returned directory holds, and a product that
/// reaches no Host.
pub async fn user_plugins(registry: Registry) -> (UserPlugins, TempDir) {
    let data = tempfile::tempdir().unwrap();
    let control = ControlService::open(&data.path().join("control.sqlite"), Arc::new(SystemClock), std::sync::Arc::new(demi_backend_database::providers::NoSealedConfigs))
        .await
        .unwrap();
    let user = UserId::try_from("u1").unwrap();
    let marks = SyncRegistry::default().of(&user);
    let product = Rc::new(TestProduct::new(Vec::new()));
    let plugins = UserPlugins::new(Arc::new(registry), user, product, control, marks);
    (plugins, data)
}

/// The backend's public URL in the scenarios.
pub const PUBLIC_URL: &str = "https://demi.example.test";

/// What several users' plugins share, as one backend's shards do: the
/// registry, a control database with each user in it, the product with
/// its sealing key, its client of outbound requests and its clock, and the
/// users' change marks.
pub struct Backend {
    pub registry: Arc<Registry>,
    pub control: ControlService,
    pub product: Rc<TestProduct>,
    pub sync: SyncRegistry,
    _data: TempDir,
}

impl Backend {
    /// The plugins of `registry` over a new control database, whose
    /// outbound requests trust `roots` besides the system's authorities.
    pub async fn start(registry: Registry, roots: Vec<reqwest::Certificate>) -> Self {
        let data = tempfile::tempdir().unwrap();
        let control =
            ControlService::open(&data.path().join("control.sqlite"), Arc::new(SystemClock), std::sync::Arc::new(demi_backend_database::providers::NoSealedConfigs))
                .await
                .unwrap();
        Self {
            registry: Arc::new(registry),
            control,
            product: Rc::new(TestProduct::new(roots)),
            sync: SyncRegistry::default(),
            _data: data,
        }
    }

    /// The user `id`, created in the control database.
    pub async fn user(&self, id: &str) -> UserId {
        demi_backend_database::control::testing::execute(
            &self.control,
            "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
             VALUES (?1, ?1 || '@example.test', '', 'hash', 'user', 0)",
            vec![id.to_owned()],
        )
        .await;
        UserId::try_from(id).unwrap()
    }

    /// An instance of every plugin for `user`, as a shard makes them: two
    /// calls are two shards, as two workers would have.
    pub fn plugins(&self, user: &UserId) -> UserPlugins {
        UserPlugins::new(
            self.registry.clone(),
            user.clone(),
            self.product.clone(),
            self.control.clone(),
            self.sync.of(user),
        )
    }
}

/// A product with no conversation Host and no blobs, which seals with a
/// key of its own, sends outbound requests through the plugins' client and
/// reads the time from a clock the scenario moves.
pub struct TestProduct {
    key: VaultKey,
    web: reqwest::Client,
    pub clock: ManualClock,
}

impl TestProduct {
    fn new(roots: Vec<reqwest::Certificate>) -> Self {
        Self {
            key: VaultKey::new([5; 32]),
            web: demi_backend_plugins::web_client(&roots),
            clock: ManualClock::new(
                demi_shared_types::Timestamp::from_millisecond(1_790_000_000_000).unwrap(),
            ),
        }
    }
}

fn none<T>() -> LocalBoxFuture<'static, Result<T, PortFailure>> {
    Box::pin(async {
        Err(PortFailure::Port(PortError::Failed(
            "the scenarios' product reaches no Host".into(),
        )))
    })
}

impl ProductPort for TestProduct {
    fn package_call<'a>(
        &'a self,
        _: &'a ConversationId,
        _: &'a NativeOperation,
        _: Map<String, Value>,
        _: CallKind,
        _: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<Value, PortFailure>> {
        none()
    }

    fn read_host_files<'a>(
        &'a self,
        _: &'a ConversationId,
        _: &'a demi_backend_plugins::ReadOn,
        _: Vec<HostRead>,
        _: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<Vec<HostFile>, PortFailure>> {
        none()
    }

    fn media_models(
        &self,
        _: demi_shared_types::MediaKind,
    ) -> LocalBoxFuture<'_, Result<demi_plugin_interface::MediaListing, PortFailure>> {
        none()
    }

    fn make_image<'a>(
        &'a self,
        _: &'a ConversationId,
        _: Option<&'a demi_host_interface::JobCaller>,
        _: demi_plugin_interface::ImageAsk,
        _: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<demi_plugin_interface::MadeImage, PortFailure>> {
        none()
    }

    fn make_video<'a>(
        &'a self,
        _: &'a ConversationId,
        _: Option<&'a demi_host_interface::JobCaller>,
        _: demi_plugin_interface::VideoAsk,
        _: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<(demi_plugin_interface::AskedVideo, demi_backend_plugins::VideoJob), PortFailure>> {
        none()
    }

    fn put_blob(&self, _: B64Bytes) -> LocalBoxFuture<'_, Result<BlobRef, PortFailure>> {
        none()
    }

    fn get_blob(&self, _: BlobRef) -> LocalBoxFuture<'_, Result<Option<B64Bytes>, PortFailure>> {
        none()
    }

    fn seal(&self, user: &UserId, plugin: &str, name: &str, plaintext: &[u8]) -> Vec<u8> {
        self.key.seal(Row::PluginSecret { plugin, user, name }, plaintext)
    }

    fn open(&self, user: &UserId, plugin: &str, name: &str, sealed: &[u8]) -> Option<Vec<u8>> {
        self.key
            .open(Row::PluginSecret { plugin, user, name }, sealed)
            .ok()
    }

    fn web_client(&self) -> reqwest::Client {
        self.web.clone()
    }

    fn public_url(&self) -> Option<url::Url> {
        Some(url::Url::parse(PUBLIC_URL).unwrap())
    }

    fn now(&self) -> demi_shared_types::Timestamp {
        demi_shared_types::Clock::now(&self.clock)
    }

    fn spawn(&self, work: LocalBoxFuture<'static, ()>) {
        // The scenarios run on a local runtime, which owns the work.
        tokio::task::spawn_local(work);
    }
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
    matches!(frame, ServerFrame::Phase { subagent_id: None, phase } if *phase == SessionPhase::Idle)
}

/// A `shell` call of `script` that watches it for up to `interval_ms`, and
/// then reports every `interval_ms` while it runs.
pub fn exec(id: &str, script: &str, interval_ms: u32) -> ProviderEvent {
    event::tool_call(
        id,
        "shell",
        json!({"description": "Run the test script", "script": script, "intervalMs": interval_ms}),
    )
}

/// A `shell` call of `script` for a command that runs until it ends or is
/// stopped: the call returns once its output has been quiet for two
/// seconds, and the command reports only its end.
pub fn resident(id: &str, script: &str) -> ProviderEvent {
    event::tool_call(
        id,
        "shell",
        json!({"description": "Run the test script", "script": script, "intervalMs": null}),
    )
}

/// A model that answers each request with the next of `steps`, but notes a
/// request whose input is commands' reports alone, which a command left
/// running opens when it reports (`shell-tool.md` § Command reports): how many
/// such requests come depends on when the commands end.
pub fn tolerant(steps: Vec<Vec<ProviderEvent>>) -> ScriptedRuntime {
    let steps = Rc::new(RefCell::new(std::collections::VecDeque::from(steps)));
    let turns = (0..200).map(move |_| {
        let steps = steps.clone();
        Turn::Respond(Box::new(move |request: &InferenceRequest| {
            let reported = matches!(
                request.items.last(),
                Some(InferenceItem::UserMessage { content })
                    if matches!(content.first(), Some(UserPart::Text(text)) if text.starts_with("Command "))
            );
            if reported {
                return reply("noted");
            }
            steps
                .borrow_mut()
                .pop_front()
                .expect("the scenario scripted this request")
        }))
    });
    ScriptedRuntime::new(turns)
}

/// The end of a turn: a text and a response.
pub fn reply(text: &str) -> Vec<ProviderEvent> {
    vec![event::text(text), event::response(10, 5)]
}

/// A model's runs for messages that each run scripts with `shell`, one
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

/// The text of the newest tool result a request carries, each medium as a
/// line that names it: `<image image/png>`, `<document document-1.pdf>`.
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
        .map(|part| match part {
            ResultPart::Text(text) => text.clone(),
            ResultPart::Image(bytes) => format!("<image {}>", bytes.media_type),
            ResultPart::Video(bytes) => format!("<video {}>", bytes.media_type),
            ResultPart::Document { file_name, .. } => format!("<document {file_name}>"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A plugin whose user methods each make one port operation of the
/// secrets, outbound requests and sign-in services, with what it answered
/// as their result, behind the JSON loopback. A refusal is the call's
/// failure.
pub struct PortProbe(Manifest);

/// The vendor's name the probes sign in to.
pub const VENDOR: &str = "Example";

impl PortProbe {
    /// The probe `id`, signing in at `issuer` when it names one.
    pub fn new(id: &str, issuer: Option<url::Url>) -> Self {
        let mut page = Page::new(format!("@demicodes/plugin-{id}"));
        for method in [
            "read_secret",
            "write_secret",
            "remove_secret",
            "send",
            "sign_in",
            "token",
            "sign_out",
        ] {
            page = page.method(Method::new::<Map<String, Value>, Value>(method, Scope::User));
        }
        let mut manifest = Manifest::new(PluginId::try_from(id).unwrap(), id, "A port probe.");
        manifest.page = Some(page);
        manifest.oauth = issuer.map(|issuer| OAuth {
            vendor: VENDOR.into(),
            issuer,
            resource: "https://mcp.example.test/mcp".into(),
            scopes: vec!["tools".into()],
        });
        Self(manifest)
    }

    /// Its sign-in names `vendor`.
    pub fn vendor(&mut self, vendor: &str) {
        self.0.oauth.as_mut().expect("the probe signs in").vendor = vendor.into();
    }

    pub fn boxed(self) -> Box<dyn PluginFactory> {
        Box::new(self)
    }
}

impl PluginFactory for PortProbe {
    fn manifest(&self) -> &Manifest {
        &self.0
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        loopback(Rc::new(Operations))
    }
}

struct Operations;

impl Plugin for Operations {
    fn call(&self, request: Request, port: PluginPort) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            let Request::PageCall { method, params, .. } = request else {
                unreachable!("the probe declares only user methods")
            };
            let text = |field: &str| params.get(field).and_then(Value::as_str).map(str::to_owned);
            let number = |field: &str| params.get(field).and_then(Value::as_u64);
            let name = || SecretName::try_from(text("name").unwrap()).unwrap();
            let result = match method.as_str() {
                "read_secret" => match port.secret(name()).await? {
                    Some(secret) => json!({ "value": secret.value, "revision": secret.revision }),
                    None => Value::Null,
                },
                "write_secret" => {
                    let revision = port
                        .write_secret(name(), text("value").unwrap(), number("revision"))
                        .await?;
                    json!({ "revision": revision })
                }
                "remove_secret" => {
                    port.remove_secret(name(), number("revision").unwrap()).await?;
                    Value::Null
                }
                "send" => {
                    let mut request = WebRequest::new(
                        WebMethod::Get,
                        url::Url::parse(&text("url").unwrap()).unwrap(),
                        Duration::from_millis(number("deadlineMs").unwrap()),
                        number("limitBytes").unwrap(),
                    );
                    if let Some(key) = text("key") {
                        request = request.header("x-api-key", key);
                    }
                    let response = port.send(request).await?;
                    json!({
                        "status": response.status,
                        "body": String::from_utf8_lossy(response.body.as_bytes()),
                    })
                }
                "sign_in" => json!({ "url": port.oauth_start().await? }),
                "token" => json!({ "token": port.oauth_token().await? }),
                "sign_out" => {
                    port.oauth_sign_out().await?;
                    Value::Null
                }
                method => unreachable!("the probe declares no method {method}"),
            };
            Ok(Reply::Result { result })
        })
    }
}

/// Calls the user method `method` of `plugin` with `params`.
pub async fn call(
    plugins: &UserPlugins,
    plugin: &str,
    method: &str,
    params: Value,
) -> Result<Value, PageCallError> {
    let call = PageCall {
        plugin: plugin.into(),
        method: method.into(),
        params,
        conversation: None,
    };
    plugins.page_call(call, CancellationToken::new()).await
}

/// The port's refusal a page call failed with.
pub fn refusal(answer: Result<Value, PageCallError>) -> PortRefusal {
    match answer {
        Err(PageCallError::Plugin(PluginError::Port { refusal })) => refusal,
        answer => panic!("expected a refusal, got {answer:?}"),
    }
}
