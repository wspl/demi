//! A scripted Claude Code CLI: a placement whose every start hands the test
//! the other end of a process that speaks stream-json as the test says. The
//! test reads what the provider writes, line by line, writes the CLI's
//! output, and ends the process; the process ends as a real one does on
//! SIGTERM and SIGKILL, and when its handle is dropped.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use bytes::Bytes;
use demi_core::{Clock, StreamKind, UserContentBlock};
use demi_provider::credentials::{AddAccount, MemoryCredentialPool};
use demi_provider::models_dev::ModelsDevClient;
use demi_provider::quota::MemorySnapshots;
use demi_provider::testing::{FixedClock, inference_request};
use demi_provider::{
    InferenceItem, InferenceRequest, Provider, ProviderEvent, ProviderRuntime, Secret,
    ToolDefinition,
};
use demi_provider_claude_code::{
    ClaudeCodeConfig, ClaudeCodeProvider, CliSite, Placement, StartError,
};
use demi_shell::{
    HostError, Process, ProcessControl, ProcessEnd, ProcessOutput, Signal, SpawnRequest,
};
use futures_channel::mpsc;
use futures_util::future::LocalBoxFuture;
use futures_util::{Stream, StreamExt as _};
use serde_json::{Value, json};
use tokio::sync::watch;

/// Where the scripted placement says every process runs.
pub fn site() -> CliSite {
    CliSite {
        executable: "/home/demi/.demi/claude/2.1.3/claude".into(),
        run_dir: "/home/demi/.demi/claude/run".into(),
        config_dir: "/home/demi/.demi/claude/config".into(),
    }
}

/// The setup token of the test account.
pub const TOKEN: &str = "sk-ant-oat01-test-token";

pub const NOW: &str = "2026-09-24T08:00:00Z";

pub fn clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock(NOW.parse().unwrap()))
}

/// A provider for an entry whose one account holds `token`, and the
/// entry's pool; the models.dev document is at `models_dev`.
pub async fn provider_with(
    token: &str,
    models_dev: &str,
    usage: &str,
) -> (ClaudeCodeProvider, MemoryCredentialPool) {
    let pool = MemoryCredentialPool::new();
    let staged = ClaudeCodeProvider::new(
        ClaudeCodeConfig::new("entry-1", "Claude", None),
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        models_dev_client(models_dev),
        reqwest::Client::new(),
        clock(),
    );
    let account = staged
        .accounts()
        .unwrap()
        .add(AddAccount::SetupToken(
            Secret::try_from(token.to_owned()).unwrap(),
        ))
        .await
        .unwrap();
    let mut config = ClaudeCodeConfig::new("entry-1", "Claude", Some(account.id));
    config.usage_url = usage.parse().unwrap();
    let provider = ClaudeCodeProvider::new(
        config,
        Arc::new(pool.clone()),
        Arc::new(MemorySnapshots::new()),
        models_dev_client(models_dev),
        reqwest::Client::new(),
        clock(),
    );
    (provider, pool)
}

/// A provider of the test account whose catalog and quota nobody reads.
pub async fn provider() -> ClaudeCodeProvider {
    provider_with(
        TOKEN,
        "http://127.0.0.1:9/api.json",
        "http://127.0.0.1:9/usage",
    )
    .await
    .0
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
        content: vec![UserContentBlock::Text { text: text.into() }],
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
        output: vec![demi_core::ToolResultContentBlock::Text { text: text.into() }],
        is_error: false,
    }
}

/// Every event of a run.
pub async fn all_events(run: impl Stream<Item = ProviderEvent>) -> Vec<ProviderEvent> {
    run.collect().await
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
        self.0
            .next()
            .await
            .expect("the placement lives while the test does")
    }
}

impl Placement for ScriptedPlacement {
    fn start<'a>(
        &'a self,
        spawn: &'a dyn Fn(&CliSite) -> SpawnRequest,
    ) -> LocalBoxFuture<'a, Result<Process, StartError>> {
        Box::pin(async move {
            if let Some(failure) = self.failure.borrow_mut().take() {
                return Err(failure);
            }
            self.starts.set(self.starts.get() + 1);
            let (process, cli) = scripted(spawn(&site()));
            // A test that dropped its end of the placement starts nothing more.
            let _ = self.started.unbounded_send(cli);
            Ok(process)
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

fn scripted(spawn: SpawnRequest) -> (Process, Cli) {
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
        self.inputs
            .next()
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
}
