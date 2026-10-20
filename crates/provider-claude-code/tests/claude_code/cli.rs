//! A scripted Claude Code CLI: a placement whose every start hands the test the
//! other end of a process that speaks stream-json as the test says, in its
//! account's directory with a directory of its own whose removal the test sees.
//!
//! The test reads what the provider writes, line by line, writes the CLI's
//! output, and ends the process. The process ends as a real one does on SIGTERM
//! and SIGKILL, and when its handle is dropped. A scripted machine runs a
//! sign-in's or a probe's work on the same placement.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use bytes::Bytes;
use demi_host_interface::{
    HostError, Process, ProcessControl, ProcessEnd, ProcessOutput, Signal, SpawnRequest,
};
use demi_provider_claude_code::{
    AccountMachine, AccountWork, ClaudeCodeConfig, ClaudeCodeProvider, CliSite, CliStart,
    CliStarter, CliSystem, CliWork, ConfigDir, Placed, Placement, PreparedProcesses, StartError,
    StartStage, StartSteps, WorkLease,
};
use demi_provider_common::credentials::{
    AccountMeta, CredentialPool, MemoryCredentialPool, credential_id_for,
};
use demi_provider_common::models_dev::ModelsDevClient;
use demi_provider_common::quota::MemorySnapshots;
use demi_provider_common::testing::{FixedClock, guarded, inference_request, titled_tool};
pub use demi_provider_common::testing::{all_events, next_event};
use demi_provider_common::{
    InferenceItem, InferenceRequest, ProviderRuntime, ResultPart, ToolDefinition, UserPart,
};
use demi_shared_types::{Clock, StreamKind, Timestamp};
use futures_channel::mpsc;
use futures_util::StreamExt as _;
use futures_util::future::{BoxFuture, LocalBoxFuture};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// Where the scripted machine's accounts' directories are.
pub const ACCOUNTS: &str = "/home/demi/accounts";

/// The executable, run directory and machine the scripted placement says
/// every process has; each start names its own configuration directory.
pub fn site() -> CliSite {
    CliSite {
        executable: "/home/demi/.demi/claude/2.1.3/claude".into(),
        run_dir: "/home/demi/run".into(),
        config_dir: String::new(),
        process_dir: String::new(),
        system: CliSystem {
            kernel: "Linux".into(),
            release: "6.8.0-test".into(),
            shell: Some("/bin/bash".into()),
            working_directory: "/home/demi/run".into(),
        },
    }
}

/// The access token an earlier release kept in the test account's record.
pub const TOKEN: &str = "sk-ant-oat01-test-access";
/// Its refresh token.
pub const REFRESH: &str = "sk-ant-ort01-test-refresh";

pub const NOW: &str = "2026-09-24T08:00:00Z";

pub fn clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock(NOW.parse().unwrap()))
}

/// The secret document an earlier release kept of a Claude sign-in whose
/// access token is `token` and expires at `expires_at`.
pub fn earlier_sign_in(token: &str, refresh: &str, expires_at: &str) -> String {
    json!({
        "accessToken": token,
        "refreshToken": refresh,
        "expiresAt": expires_at.parse::<Timestamp>().unwrap(),
        "scopes": ["user:inference", "user:profile"],
        "subscriptionType": "max",
        "accountId": "account-1",
        "email": "alex@example.test",
    })
    .to_string()
}

/// Stores the test account as the one account of `pool`, with `secret`, an
/// earlier release's sign-in, or none, as a sign-in now stores it, and
/// answers its id.
pub async fn seed(pool: &MemoryCredentialPool, secret: Option<String>) -> String {
    let id = credential_id_for(Some("account:account-1"), "alex@example.test");
    let meta = AccountMeta {
        id: id.clone(),
        label: "alex@example.test".into(),
        detail: None,
        updated_at: NOW.parse().unwrap(),
        source: "login:code".into(),
        identity_key: Some("account:account-1".into()),
        signed_out: false,
    };
    pool.write(meta, secret).await.unwrap();
    pool.set_active(&id).await.unwrap();
    id
}

/// The models.dev address nothing answers at.
pub const NOWHERE: &str = "http://127.0.0.1:9/api.json";

/// A provider for `account` of `pool`, without a machine to probe on.
pub fn provider_of(pool: &MemoryCredentialPool, account: &str) -> ClaudeCodeProvider {
    provider_on(pool, account, None)
}

/// A provider for `account` of `pool` whose usage probe runs on `machine`.
pub fn provider_on(
    pool: &MemoryCredentialPool,
    account: &str,
    machine: Option<Arc<dyn AccountMachine>>,
) -> ClaudeCodeProvider {
    let config = ClaudeCodeConfig::new("entry-1", "Claude", Some(account.to_owned()), machine);
    ClaudeCodeProvider::new(
        config,
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        models_dev_client(NOWHERE),
        clock(),
    )
}

/// A provider for an entry whose one account is signed in, and the entry's
/// pool.
pub async fn provider_with_pool() -> (ClaudeCodeProvider, MemoryCredentialPool) {
    let pool = MemoryCredentialPool::new();
    let account = seed(&pool, None).await;
    (provider_of(&pool, &account), pool)
}

/// A provider of the test account whose catalog and quota nobody reads.
pub async fn provider() -> ClaudeCodeProvider {
    provider_with_pool().await.0
}

pub fn models_dev_client(url: &str) -> ModelsDevClient {
    ModelsDevClient::new(reqwest::Client::new(), url.parse().unwrap(), clock())
}

/// One account work as the test's machine runs it: the work, the token
/// that fires once nobody waits for it, and where its end goes.
type MachineWork = (AccountWork, CancellationToken, tokio::sync::oneshot::Sender<()>);

/// A machine whose account work runs on `placement`, as a task of the
/// test's thread, with a token that fires once nobody waits for the work,
/// as the backend's does.
pub fn machine(placement: &ScriptedPlacement) -> Arc<dyn AccountMachine> {
    let (works, mut received) = tokio::sync::mpsc::unbounded_channel::<MachineWork>();
    let placement = placement.clone();
    tokio::task::spawn_local(async move {
        while let Some((work, abandoned, done)) = received.recv().await {
            work(Rc::new(placement.clone()), abandoned).await;
            // A test that stopped waiting wants no answer.
            let _ = done.send(());
        }
    });
    Arc::new(ScriptedMachine(works))
}

/// A machine that hands each work to the test's thread.
struct ScriptedMachine(tokio::sync::mpsc::UnboundedSender<MachineWork>);

impl AccountMachine for ScriptedMachine {
    fn run(&self, _: String, work: AccountWork) -> BoxFuture<'static, Result<(), StartError>> {
        let (done, ended) = tokio::sync::oneshot::channel();
        let abandoned = CancellationToken::new();
        let sent = self.0.send((work, abandoned.clone(), done));
        Box::pin(async move {
            // Dropping the answer gives the work up.
            let _abandoned = abandoned.drop_guard();
            sent.map_err(|_| StartError("the test's machine is gone".into()))?;
            ended.await.map_err(|_| StartError("the work ended without an answer".into()))
        })
    }

    fn remove_account(&self, _: String) -> BoxFuture<'static, Result<(), StartError>> {
        Box::pin(async { Ok(()) })
    }
}

/// The tool a request offers, as the agent builds it.
pub fn shell_exec() -> ToolDefinition {
    ToolDefinition {
        name: "shell_exec".into(),
        ..titled_tool()
    }
}

/// A request of session `session-1` with `items`, the model `claude-test`,
/// the system prompt `system` and the shell tool.
pub fn request(items: Vec<InferenceItem>) -> InferenceRequest {
    InferenceRequest {
        model_id: "claude-test".into(),
        system_prompt: "system".into(),
        items: items.into(),
        tools: Arc::new([shell_exec()]),
        ..inference_request()
    }
}

/// The same, without tools.
pub fn request_without_tools(items: Vec<InferenceItem>) -> InferenceRequest {
    InferenceRequest {
        tools: Arc::new([]),
        ..request(items)
    }
}

pub fn user(text: &str) -> InferenceItem {
    InferenceItem::UserMessage {
        content: vec![UserPart::Text(text.into())],
    }
}

pub fn tool_use(id: &str, script: &str) -> InferenceItem {
    InferenceItem::ToolUse {
        model_id: "claude-test".into(),
        tool_use_id: id.into(),
        tool_name: "shell_exec".into(),
        input: json!({ "script": script }),
    }
}

pub fn tool_result(id: &str, text: &str) -> InferenceItem {
    InferenceItem::ToolResult {
        tool_use_id: id.into(),
        output: vec![ResultPart::Text(text.into())],
        is_error: false,
    }
}

/// A runtime of `provider` over `placement`.
pub fn runtime_of(
    provider: &ClaudeCodeProvider,
    placement: &ScriptedPlacement,
) -> Box<dyn ProviderRuntime> {
    provider.process_runtime(Rc::new(placement.clone()))
}

/// A placement whose starts the test takes the other end of.
#[derive(Clone)]
pub struct ScriptedPlacement {
    started: mpsc::UnboundedSender<Cli>,
    /// The failure of the next start.
    failure: Rc<RefCell<Option<StartError>>>,
    starts: Rc<Cell<usize>>,
    /// The processes prepared ahead on the one machine every clone stands
    /// for.
    prepared: Rc<PreparedProcesses>,
    /// How many work leases of the machine are held.
    leases: Rc<Cell<usize>>,
    /// The accounts whose directory a sign-in's became.
    adopted: Rc<RefCell<Vec<String>>>,
    /// Told when an adoption begins, which then waits for the account's
    /// processes for good while `holds_adoption` is set.
    adopting: Rc<tokio::sync::Notify>,
    holds_adoption: Rc<Cell<bool>>,
}

impl ScriptedPlacement {
    pub fn new() -> (Self, Starts) {
        let (started, starts) = mpsc::unbounded();
        let placement = Self {
            started,
            failure: Rc::default(),
            starts: Rc::default(),
            prepared: Rc::default(),
            leases: Rc::default(),
            adopted: Rc::default(),
            adopting: Rc::default(),
            holds_adoption: Rc::default(),
        };
        (placement, Starts(starts))
    }

    pub fn fail_next(&self, message: &str) {
        *self.failure.borrow_mut() = Some(StartError(message.into()));
    }

    /// How many processes were started.
    pub fn starts(&self) -> usize {
        self.starts.get()
    }

    /// How many processes prepared ahead the machine holds unused.
    pub fn prepared_count(&self) -> usize {
        self.prepared.len()
    }

    /// How many work leases of the machine are held now.
    pub fn leases(&self) -> usize {
        self.leases.get()
    }

    /// The accounts whose directory a sign-in's became, in order.
    pub fn adopted(&self) -> Vec<String> {
        self.adopted.borrow().clone()
    }

    /// From now on an adoption waits for the account's processes, which
    /// never end; the future completes once one begins.
    pub fn hold_adoptions(&self) -> impl Future<Output = ()> + use<> {
        self.holds_adoption.set(true);
        let adopting = self.adopting.clone();
        async move { adopting.notified().await }
    }
}

/// A held work lease, counted while it lives.
struct Lease(Rc<Cell<usize>>);

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

/// The processes the placement started, as the test takes them.
pub struct Starts(mpsc::UnboundedReceiver<Cli>);

impl Starts {
    /// The next process started.
    pub async fn next(&mut self) -> Cli {
        guarded("the next process start", self.0.next())
            .await
            .expect("the placement lives while the test does")
    }
}

impl Placement for ScriptedPlacement {
    fn start<'a>(
        &'a self,
        work: CliWork,
        start: CliStarter,
        stages: &'a dyn Fn(StartStage),
    ) -> LocalBoxFuture<'a, Result<Placed, StartError>> {
        Box::pin(async move {
            stages(StartStage::Machine);
            if let Some(failure) = self.failure.borrow_mut().take() {
                return Err(failure);
            }
            stages(StartStage::Process);
            let started = self.starts.get() + 1;
            self.starts.set(started);
            let config_dir = match &work {
                CliWork::Account { account, .. } => format!("{ACCOUNTS}/{account}"),
                CliWork::Login => format!("{ACCOUNTS}/.login-{started}"),
            };
            let site = CliSite {
                config_dir,
                process_dir: format!("process-{started}"),
                ..site()
            };
            let (process, cli) = scripted(start(&site), work, site);
            let config = Box::new(ScriptedConfig {
                state: cli.state.clone(),
                adopted: self.adopted.clone(),
                adopting: self.adopting.clone(),
                holds_adoption: self.holds_adoption.get(),
            });
            // A test that dropped its end of the placement starts nothing more.
            let _ = self.started.unbounded_send(cli);
            Ok(Placed {
                process,
                config,
                steps: StartSteps::default(),
            })
        })
    }

    fn work(&self) -> WorkLease {
        self.leases.set(self.leases.get() + 1);
        WorkLease::new(Lease(self.leases.clone()))
    }

    fn prepared(&self) -> Rc<PreparedProcesses> {
        self.prepared.clone()
    }
}

/// A scripted process's own directory, whose removal the test sees, and
/// for a sign-in the files its CLI wrote, which the test gives it.
struct ScriptedConfig {
    state: Rc<State>,
    adopted: Rc<RefCell<Vec<String>>>,
    adopting: Rc<tokio::sync::Notify>,
    holds_adoption: bool,
}

impl ConfigDir for ScriptedConfig {
    fn read<'a>(&'a self, name: &'a str) -> LocalBoxFuture<'a, Result<Bytes, HostError>> {
        Box::pin(async move {
            self.state.written.borrow().iter().find(|(file, _)| file == name).map(|(_, bytes)| bytes.clone()).ok_or_else(|| {
                HostError::failed(
                    Some("ENOENT".into()),
                    format!("no file {name} in a scripted directory"),
                )
            })
        })
    }

    fn remove(self: Box<Self>) -> LocalBoxFuture<'static, ()> {
        let state = self.state.clone();
        Box::pin(async move {
            assert!(state.ended(), "a directory is removed once its process ended");
            state.removed.set(true);
        })
    }

    fn adopt<'a>(self: Box<Self>, account: &'a str) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            assert!(self.state.ended(), "a sign-in's directory is adopted once its process ended");
            if self.holds_adoption {
                self.adopting.notify_one();
                std::future::pending::<()>().await;
            }
            self.adopted.borrow_mut().push(account.to_owned());
            Ok(())
        })
    }
}

/// What the process and the test share.
struct State {
    output: RefCell<Option<mpsc::UnboundedSender<ProcessOutput>>>,
    end: watch::Sender<Option<ProcessEnd>>,
    signals: RefCell<Vec<Signal>>,
    ignores_terminate: Cell<bool>,
    dropped: Cell<bool>,
    unread: RefCell<Vec<u8>>,
    input: mpsc::UnboundedSender<Value>,
    /// What the process was started at, and whether its own directory was
    /// removed.
    site: CliSite,
    removed: Cell<bool>,
    /// The files the process wrote into its configuration directory.
    written: RefCell<Vec<(String, Bytes)>>,
}

impl State {
    fn ended(&self) -> bool {
        self.end.borrow().is_some()
    }

    /// Ends the process: its output ends, and its end is known.
    fn finish(&self, end: ProcessEnd) {
        if self.ended() {
            return;
        }
        self.output.borrow_mut().take();
        self.end.send_replace(Some(end));
    }
}

fn scripted(start: CliStart, work: CliWork, site: CliSite) -> (Process, Cli) {
    let CliStart { spawn, files } = start;
    let (output, outputs) = mpsc::unbounded();
    let (input, inputs) = mpsc::unbounded();
    let state = Rc::new(State {
        output: RefCell::new(Some(output)),
        end: watch::Sender::new(None),
        signals: RefCell::default(),
        ignores_terminate: Cell::new(false),
        dropped: Cell::new(false),
        unread: RefCell::default(),
        input,
        site,
        removed: Cell::new(false),
        written: RefCell::default(),
    });
    let exit = {
        let mut end = state.end.subscribe();
        Box::pin(async move {
            let ended = end
                .wait_for(Option::is_some)
                .await
                .expect("the state lives");
            ended.clone().expect("the end is set")
        })
    };
    let process = Process {
        output: outputs.boxed_local(),
        control: Box::new(Control(state.clone())),
        exit,
    };
    let cli = Cli {
        spawn,
        files,
        work,
        state,
        inputs,
    };
    (process, cli)
}

/// The provider's end of the process's control.
struct Control(Rc<State>);

impl ProcessControl for Control {
    fn write_stdin(&self, bytes: Bytes) -> LocalBoxFuture<'_, Result<(), HostError>> {
        Box::pin(async move {
            if self.0.ended() {
                return Ok(());
            }
            let mut unread = self.0.unread.borrow_mut();
            unread.extend_from_slice(&bytes);
            while let Some(end) = unread.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = unread.drain(..=end).collect();
                // A line that is not JSON, such as a sign-in's code, reads as
                // its text, without its newline.
                let value = serde_json::from_slice(&line).unwrap_or_else(|_| {
                    Value::String(String::from_utf8_lossy(&line[..line.len() - 1]).into_owned())
                });
                // A test that stopped reading loses nothing it asserts.
                let _ = self.0.input.unbounded_send(value);
            }
            Ok(())
        })
    }

    fn close_stdin(&self) -> LocalBoxFuture<'_, Result<(), HostError>> {
        Box::pin(async { Ok(()) })
    }

    fn kill(&self, signal: Signal) -> LocalBoxFuture<'_, Result<(), HostError>> {
        Box::pin(async move {
            self.0.signals.borrow_mut().push(signal);
            match signal {
                Signal::Terminate if self.0.ignores_terminate.get() => {}
                Signal::Terminate => self.0.finish(ProcessEnd::Signalled("SIGTERM".into())),
                Signal::Kill => self.0.finish(ProcessEnd::Signalled("SIGKILL".into())),
            }
            Ok(())
        })
    }
}

impl Drop for Control {
    /// A handle dropped before its process ended kills it, as a Host does.
    fn drop(&mut self) {
        if !self.0.ended() {
            self.0.dropped.set(true);
            self.0.finish(ProcessEnd::Signalled("SIGKILL".into()));
        }
    }
}

/// The test's end of a scripted process.
pub struct Cli {
    /// What the provider asked the placement to start.
    pub spawn: SpawnRequest,
    /// The files the provider put in the process's own directory, by their
    /// path there.
    pub files: Vec<(String, Bytes)>,
    /// What the process was started for.
    pub work: CliWork,
    state: Rc<State>,
    inputs: mpsc::UnboundedReceiver<Value>,
}

impl Cli {
    /// The next line the provider writes.
    pub async fn read(&mut self) -> Value {
        guarded("the provider's next line", self.inputs.next())
            .await
            .expect("the process's input stays open while the test reads it")
    }

    /// The next line the provider writes, a user message, which the CLI
    /// takes into a turn and prints back, as `--replay-user-messages` has it
    /// do.
    pub async fn take(&mut self) -> Value {
        let message = self.read().await;
        assert_eq!(message["type"], "user", "{message}");
        let mut replay = message.clone();
        replay["isReplay"] = json!(true);
        self.say(replay);
        message
    }

    /// Every line the provider wrote and the test did not read yet.
    pub fn unread(&mut self) -> Vec<Value> {
        std::iter::from_fn(|| self.inputs.try_recv().ok()).collect()
    }

    /// The session the process resumes: the one file of its own directory,
    /// which the argument after `--resume` names, and its entries.
    pub fn session(&self) -> Vec<Value> {
        let [(path, bytes)] = self.files.as_slice() else {
            panic!("one file: {:?}", self.files);
        };
        let resumed = self
            .spawn
            .args
            .windows(2)
            .find(|pair| pair[0] == "--resume")
            .map(|pair| pair[1].clone())
            .expect("the process resumes a session");
        assert_eq!(path, &format!("{resumed}.jsonl"));
        let text = std::str::from_utf8(bytes).expect("the session is text");
        text.lines()
            .map(|line| serde_json::from_str(line).expect("each line is an entry"))
            .collect()
    }

    /// The messages of the session's entries, in order.
    pub fn messages(&self) -> Vec<Value> {
        self.session()
            .into_iter()
            .filter_map(|entry| entry.get("message").cloned())
            .collect()
    }

    /// A batch of `entries` the CLI wrote to its session.
    pub fn mirror(&self, entries: Vec<Value>) {
        self.say(json!({ "type": "transcript_mirror", "filePath": "/session.jsonl", "entries": entries }));
    }

    /// Writes `line` to the process's standard output.
    pub fn say(&self, line: Value) {
        self.write(StreamKind::Stdout, format!("{line}\n"));
    }

    /// Writes `text` to the process's standard output as it is.
    pub fn say_text(&self, text: &str) {
        self.write(StreamKind::Stdout, text.to_owned());
    }

    pub fn stderr(&self, text: &str) {
        self.write(StreamKind::Stderr, text.to_owned());
    }

    fn write(&self, stream: StreamKind, text: String) {
        let output = self.state.output.borrow();
        let output = output.as_ref().expect("the process still runs");
        output
            .unbounded_send(ProcessOutput {
                stream,
                bytes: Bytes::from(text),
            })
            .expect("the provider reads the output");
    }

    /// Ends the process with `end`.
    pub fn exit(&self, end: ProcessEnd) {
        self.state.finish(end);
    }

    /// The process ignores SIGTERM from now on.
    pub fn ignore_terminate(&self) {
        self.state.ignores_terminate.set(true);
    }

    /// The signals the provider sent.
    pub fn signals(&self) -> Vec<Signal> {
        self.state.signals.borrow().clone()
    }

    /// Whether the provider dropped the process's handle while it ran.
    pub fn dropped(&self) -> bool {
        self.state.dropped.get()
    }

    /// How the process ended, once it did.
    pub fn end(&self) -> Option<ProcessEnd> {
        self.state.end.borrow().clone()
    }

    /// How the process ended, once it does.
    pub async fn ended(&self) -> ProcessEnd {
        let mut end = self.state.end.subscribe();
        let ended = guarded("the process's end", end.wait_for(Option::is_some))
            .await
            .expect("the state lives");
        ended.clone().expect("the end is set")
    }

    /// The process's configuration directory: its account's, or a
    /// sign-in's own.
    pub fn config_dir(&self) -> &str {
        &self.state.site.config_dir
    }

    /// The name of the process's own directory under it.
    pub fn process_dir(&self) -> &str {
        &self.state.site.process_dir
    }

    /// Whether the process's own directory was removed.
    pub fn config_removed(&self) -> bool {
        self.state.removed.get()
    }

    /// The process writes `bytes` as the file `name` of its configuration
    /// directory, as the CLI's login writes its sign-in.
    pub fn write_file(&self, name: &str, bytes: &str) {
        self.state
            .written
            .borrow_mut()
            .push((name.to_owned(), Bytes::from(bytes.to_owned())));
    }

    /// Reads the `initialize` control request, which declares the SDK MCP
    /// server, and answers it.
    pub async fn initialized(&mut self) -> Value {
        let initialize = self.read().await;
        assert_eq!(initialize["type"], "control_request", "{initialize}");
        assert_eq!(
            initialize["request"]["subtype"], "initialize",
            "{initialize}"
        );
        self.say(json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": initialize["request_id"] },
        }));
        initialize
    }

    /// Sends an MCP message in control request `request_id`.
    pub fn mcp(&self, request_id: &str, message: Value) {
        self.say(json!({
            "type": "control_request",
            "request_id": request_id,
            "request": { "subtype": "mcp_message", "server_name": "main", "message": message },
        }));
    }

    /// Reads the answer to control request `request_id`: the MCP reply.
    pub async fn mcp_reply(&mut self, request_id: &str) -> Value {
        let answer = self.read().await;
        assert_eq!(answer["type"], "control_response", "{answer}");
        assert_eq!(answer["response"]["request_id"], request_id, "{answer}");
        assert_eq!(answer["response"]["subtype"], "success", "{answer}");
        answer["response"]["response"]["mcp_response"].clone()
    }

    /// The MCP handshake, as the CLI's client makes it.
    pub async fn handshake(&mut self) {
        self.mcp(
            "mcp-init",
            json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "claude-code", "version": "2.1.3" },
                },
            }),
        );
        let initialize = self.mcp_reply("mcp-init").await;
        assert_eq!(
            initialize["result"]["serverInfo"]["name"], "demi",
            "{initialize}"
        );
        self.mcp(
            "mcp-initialized",
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        );
        let acknowledged = self.mcp_reply("mcp-initialized").await;
        assert_eq!(
            acknowledged,
            json!({ "jsonrpc": "2.0", "id": 0, "result": {} })
        );
    }

    /// A `tools/call` of `id` in control request `request_id`.
    pub fn call(&self, request_id: &str, id: i64, tool_use_id: &str, script: &str) {
        self.mcp(
            request_id,
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {
                    "name": "shell_exec",
                    "arguments": { "script": script },
                    "_meta": { "claudecode/toolUseId": tool_use_id },
                },
            }),
        );
    }

    /// A streamed text piece.
    pub fn text(&self, text: &str) {
        self.say(json!({
            "type": "stream_event",
            "event": { "type": "content_block_delta", "delta": { "type": "text_delta", "text": text } },
        }));
    }

    /// A whole assistant message with a tool use.
    pub fn tool_use(&self, id: &str, script: &str) {
        self.say(json!({
            "type": "assistant",
            "message": { "content": [{
                "type": "tool_use",
                "id": id,
                "name": "mcp__main__shell_exec",
                "input": { "script": script },
            }] },
        }));
    }

    /// A tool use streamed as the model writes it: its block's start at
    /// `index`, then each piece of its input.
    pub fn streamed_tool_use(&self, index: u64, id: &str, pieces: &[&str]) {
        self.say(json!({ "type": "stream_event", "event": {
            "type": "content_block_start",
            "index": index,
            "content_block": { "type": "tool_use", "id": id, "name": "mcp__main__shell_exec", "input": {} },
        } }));
        for piece in pieces {
            self.say(json!({ "type": "stream_event", "event": {
                "type": "content_block_delta",
                "index": index,
                "delta": { "type": "input_json_delta", "partial_json": piece },
            } }));
        }
    }

    pub fn message_start(&self) {
        self.say(
            json!({ "type": "stream_event", "event": { "type": "message_start", "message": {} } }),
        );
    }

    pub fn message_stop(&self) {
        self.say(json!({ "type": "stream_event", "event": { "type": "message_stop" } }));
    }

    /// A call's `message_delta`, which states its `input` and `output`
    /// tokens.
    pub fn message_delta(&self, input: u64, output: u64) {
        self.say(json!({
            "type": "stream_event",
            "event": {
                "type": "message_delta",
                "usage": { "input_tokens": input, "output_tokens": output },
            },
        }));
    }

    /// The turn's end after a call of `input` and `output` tokens: the
    /// call's `message_delta`, then the result.
    pub fn result(&self, input: u64, output: u64) {
        self.message_delta(input, output);
        self.say(json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "usage": { "input_tokens": input, "output_tokens": output },
        }));
    }
}
