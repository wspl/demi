//! A scripted Claude Code CLI: a placement whose every start hands the test
//! the other end of a process that speaks stream-json as the test says, in
//! a configuration directory of its own whose removal the test sees. The
//! test reads what the provider writes, line by line, writes the CLI's
//! output, and ends the process; the process ends as a real one does on
//! SIGTERM and SIGKILL, and when its handle is dropped.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use bytes::Bytes;
use demi_host_interface::{
    HostError, Process, ProcessControl, ProcessEnd, ProcessOutput, Signal, SpawnRequest,
};
use demi_provider_claude_code::{
    ClaudeCodeConfig, ClaudeCodeProvider, CliSite, ConfigDir, Placed, Placement, StartError,
};
use demi_provider_common::credentials::{
    AccountMeta, CredentialPool, MemoryCredentialPool, credential_id_for,
};
use demi_provider_common::models_dev::ModelsDevClient;
use demi_provider_common::quota::MemorySnapshots;
use demi_provider_common::testing::{FixedClock, guarded, inference_request};
pub use demi_provider_common::testing::{all_events, next_event};
use demi_provider_common::{
    InferenceItem, InferenceRequest, ProviderRuntime, ResultPart, RuntimeEnv, ToolDefinition,
    UserPart,
};
use demi_shared_types::{Clock, StreamKind, Timestamp};
use futures_channel::mpsc;
use futures_util::StreamExt as _;
use futures_util::future::LocalBoxFuture;
use serde_json::{Value, json};
use tokio::sync::watch;

/// The executable and run directory the scripted placement says every
/// process has; each process's configuration directory is its own.
pub fn site() -> CliSite {
    CliSite {
        executable: "/home/demi/.demi/claude/2.1.3/claude".into(),
        run_dir: "/home/demi/.demi/claude/run".into(),
        config_dir: String::new(),
    }
}

/// The access token of the test account.
pub const TOKEN: &str = "sk-ant-oat01-test-access";
/// Its refresh token.
pub const REFRESH: &str = "sk-ant-ort01-test-refresh";

pub const NOW: &str = "2026-09-24T08:00:00Z";

pub fn clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock(NOW.parse().unwrap()))
}

/// The secret document of a Claude sign-in whose access token is `token`
/// and expires at `expires_at`.
pub fn sign_in(token: &str, refresh: &str, expires_at: &str) -> String {
    json!({
        "accessToken": token,
        "refreshToken": refresh,
        "expiresAt": expires_at.parse::<Timestamp>().unwrap(),
        "scopes": ["user:inference", "user:profile"],
        "subscriptionType": "max",
        "accountId": "account-1",
        "email": "zan@example.test",
    })
    .to_string()
}

/// Stores `secret` as the one account of `pool`, and answers its id.
pub async fn seed(pool: &MemoryCredentialPool, secret: String) -> String {
    let id = credential_id_for(Some("account:account-1"), "zan@example.test");
    let meta = AccountMeta {
        id: id.clone(),
        label: "zan@example.test".into(),
        detail: Some("max".into()),
        updated_at: NOW.parse().unwrap(),
        source: "login:code".into(),
        identity_key: Some("account:account-1".into()),
    };
    pool.write(meta, secret).await.unwrap();
    pool.set_active(&id).await.unwrap();
    id
}

/// The vendor's addresses a provider reads.
pub struct Urls<'a> {
    pub models_dev: &'a str,
    pub usage: &'a str,
    pub token: &'a str,
}

/// Addresses nothing answers at.
pub const NOWHERE: Urls<'static> = Urls {
    models_dev: "http://127.0.0.1:9/api.json",
    usage: "http://127.0.0.1:9/usage",
    token: "http://127.0.0.1:9/v1/oauth/token",
};

/// A provider for `account` of `pool`, reading the vendor at `urls`.
pub fn provider_of(pool: &MemoryCredentialPool, account: &str, urls: &Urls<'_>) -> ClaudeCodeProvider {
    provider_at(pool, account, urls, clock())
}

/// The same, on `clock`.
pub fn provider_at(
    pool: &MemoryCredentialPool,
    account: &str,
    urls: &Urls<'_>,
    clock: Arc<dyn Clock>,
) -> ClaudeCodeProvider {
    let mut config = ClaudeCodeConfig::new("entry-1", "Claude", Some(account.to_owned()));
    config.usage_url = urls.usage.parse().unwrap();
    config.token_url = urls.token.parse().unwrap();
    ClaudeCodeProvider::new(
        config,
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        models_dev_client(urls.models_dev),
        reqwest::Client::new(),
        clock,
    )
}

/// A provider for an entry whose one account's access token is `token`,
/// valid for hours, and the entry's pool; the vendor is at `urls`.
pub async fn provider_with(token: &str, urls: &Urls<'_>) -> (ClaudeCodeProvider, MemoryCredentialPool) {
    let pool = MemoryCredentialPool::new();
    let account = seed(&pool, sign_in(token, REFRESH, "2026-09-24T16:00:00Z")).await;
    (provider_of(&pool, &account, urls), pool)
}

/// A provider of the test account whose catalog and quota nobody reads.
pub async fn provider() -> ClaudeCodeProvider {
    provider_with(TOKEN, &NOWHERE).await.0
}

pub fn models_dev_client(url: &str) -> ModelsDevClient {
    ModelsDevClient::new(reqwest::Client::new(), url.parse().unwrap(), clock())
}

/// The tool a request offers, as the agent builds it.
pub fn shell_exec() -> ToolDefinition {
    let schema = json!({
        "type": "object",
        "properties": { "script": { "type": "string" } },
        "required": ["script"],
        "additionalProperties": false,
    });
    ToolDefinition {
        name: "shell_exec".into(),
        description: "Execute a shell script".into(),
        input_schema: schema.as_object().unwrap().clone(),
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
    let env = RuntimeEnv {
        http: reqwest::Client::new(),
    };
    provider.process_runtime(env, Rc::new(placement.clone()))
}

/// A placement whose starts the test takes the other end of.
#[derive(Clone)]
pub struct ScriptedPlacement {
    started: mpsc::UnboundedSender<Cli>,
    /// The failure of the next start.
    failure: Rc<RefCell<Option<StartError>>>,
    starts: Rc<Cell<usize>>,
}

impl ScriptedPlacement {
    pub fn new() -> (Self, Starts) {
        let (started, starts) = mpsc::unbounded();
        let placement = Self {
            started,
            failure: Rc::default(),
            starts: Rc::default(),
        };
        (placement, Starts(starts))
    }

    /// The next start fails with `message`.
    pub fn fail_next(&self, message: &str) {
        *self.failure.borrow_mut() = Some(StartError(message.into()));
    }

    /// How many processes were started.
    pub fn starts(&self) -> usize {
        self.starts.get()
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
        spawn: &'a dyn Fn(&CliSite) -> SpawnRequest,
    ) -> LocalBoxFuture<'a, Result<Placed, StartError>> {
        Box::pin(async move {
            if let Some(failure) = self.failure.borrow_mut().take() {
                return Err(failure);
            }
            let started = self.starts.get() + 1;
            self.starts.set(started);
            let site = CliSite {
                config_dir: format!("/tmp/demi-claude-{started}"),
                ..site()
            };
            let (process, cli) = scripted(spawn(&site), site.config_dir);
            let config = Box::new(ScriptedConfig(cli.state.clone()));
            // A test that dropped its end of the placement starts nothing more.
            let _ = self.started.unbounded_send(cli);
            Ok(Placed { process, config })
        })
    }
}

/// A scripted process's configuration directory, whose removal the test
/// sees.
struct ScriptedConfig(Rc<State>);

impl ConfigDir for ScriptedConfig {
    fn read<'a>(&'a self, name: &'a str) -> LocalBoxFuture<'a, Result<Bytes, HostError>> {
        Box::pin(async move {
            Err(HostError::failed(
                Some("ENOENT".into()),
                format!("no file {name} in a scripted directory"),
            ))
        })
    }

    fn remove(self: Box<Self>) -> LocalBoxFuture<'static, ()> {
        let state = self.0.clone();
        Box::pin(async move {
            assert!(state.ended(), "a directory is removed once its process ended");
            state.removed.set(true);
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
    /// The process's configuration directory, and whether it was removed.
    config_dir: String,
    removed: Cell<bool>,
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

fn scripted(spawn: SpawnRequest, config_dir: String) -> (Process, Cli) {
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
        config_dir,
        removed: Cell::new(false),
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
                let value = serde_json::from_slice(&line).expect("the provider writes JSON lines");
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

    /// Every line the provider wrote and the test did not read yet.
    pub fn unread(&mut self) -> Vec<Value> {
        std::iter::from_fn(|| self.inputs.try_recv().ok()).collect()
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

    /// The process's configuration directory.
    pub fn config_dir(&self) -> &str {
        &self.state.config_dir
    }

    /// Whether the process's configuration directory was removed.
    pub fn config_removed(&self) -> bool {
        self.state.removed.get()
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

    /// The turn's end, with `input` and `output` tokens.
    pub fn result(&self, input: u64, output: u64) {
        self.say(json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "usage": { "input_tokens": input, "output_tokens": output },
        }));
    }

    /// The turn's end after the vendor refused the token, as Claude Code
    /// 2.1.294 prints it: its made-up assistant line, then a failed result
    /// that names HTTP 401.
    pub fn refused(&self) {
        let words = "Failed to authenticate. API Error: 401 OAuth token has expired";
        self.say(json!({
            "type": "assistant",
            "error": "authentication_failed",
            "is_api_error_message": true,
            "message": { "model": "<synthetic>", "content": [{ "type": "text", "text": words }] },
        }));
        self.say(json!({
            "type": "result",
            "subtype": "success",
            "is_error": true,
            "api_error_status": 401,
            "terminal_reason": "api_error",
            "result": words,
            "usage": { "input_tokens": 0, "output_tokens": 0 },
        }));
    }
}
