//! A CLI process a runtime keeps across the turns of a session
//! (`claude-code.md` § Process lifetime): its input and output, its SDK MCP
//! server, what it has received of the transcript, and the tool calls it
//! holds. Its standard error is drained all the time, keeping the last
//! 64 KiB. Dropping it asks the Host to kill the process without waiting;
//! [`LiveCli::close`] ends it and waits.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use demi_core::{StreamKind, ThinkingConfig, UserContentBlock};
use demi_provider::quota::Observation;
use demi_provider::wire::Tagged;
use demi_provider::{
    InferenceItem, InferenceRequest, ProviderFailure, ToolCall, UnloadedMedia, encode_body,
};
use demi_shell::{Process, ProcessControl, ProcessEnd, ProcessOutput, Signal};
use futures_channel::mpsc;
use futures_util::future::{LocalBoxFuture, Shared as SharedFuture};
use futures_util::stream::LocalBoxStream;
use futures_util::{FutureExt as _, StreamExt as _};
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use serde::Deserialize as _;
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};
use tokio_util::io::StreamReader;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::input::{self, ControlRequest, ControlResponse, Input, MCP_SERVER, McpReply};
use crate::mcp::{self, Mcp, McpEvent};
use crate::output::{ControlRequestLine, Line};
use crate::placement::{CliSite, Placement};
use crate::{FAMILY, Shared, cli};

/// The `tracing` target of the raw stream-json exchange, at trace level.
pub(crate) const WIRE: &str = "demi::provider::claude_code::wire";

/// How much of the process's standard error is kept.
const STDERR_TAIL_BYTES: usize = 64 * 1024;

/// How long a closed process has to end after SIGTERM before SIGKILL.
const CLOSE_GRACE: Duration = Duration::from_secs(5);

/// The longest line Demi reads; a longer one fails the run like a line that
/// cannot be read.
const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;

/// The CLI's output, line by line.
type Lines =
    FramedRead<StreamReader<mpsc::UnboundedReceiver<io::Result<Bytes>>, Bytes>, LinesCodec>;

/// What a process was started for: another session, model or thinking
/// setting needs a new process, since each is fixed per process.
#[derive(Debug, Clone, PartialEq)]
struct ProcessKey {
    session_id: String,
    model_id: String,
    thinking: Option<ThinkingConfig>,
}

impl ProcessKey {
    fn of(request: &InferenceRequest) -> Self {
        Self {
            session_id: request.session_id.clone(),
            model_id: request.model_id.clone(),
            thinking: request.thinking.clone(),
        }
    }
}

/// The user messages a process received: how many, steers included, and
/// the first one, which an edit, a fork or a compaction changes.
#[derive(Default)]
struct Sent {
    count: usize,
    first: Option<Vec<UserContentBlock>>,
}

impl Sent {
    fn of(items: &[InferenceItem]) -> Self {
        Self {
            count: input::user_messages(items).count(),
            first: first_user_message(items).cloned(),
        }
    }

    /// Whether `items` no longer continue what the process received: they
    /// hold fewer user messages, or another first one.
    fn diverged(&self, items: &[InferenceItem]) -> bool {
        if input::user_messages(items).count() < self.count {
            return true;
        }
        self.first
            .as_ref()
            .is_some_and(|first| first_user_message(items) != Some(first))
    }
}

fn first_user_message(items: &[InferenceItem]) -> Option<&Vec<UserContentBlock>> {
    items.iter().find_map(|item| match item {
        InferenceItem::UserMessage { content } => Some(content),
        _ => None,
    })
}

/// A line of output as it was read.
enum Read {
    Line(String, Option<Line>),
    Blank,
    End,
    Broken(ProviderFailure),
}

/// What the process did next, as the run reads it.
pub(crate) enum Next {
    /// The run's token was cancelled.
    Cancelled,
    /// A line, with its text, which is the record of a failure it causes;
    /// `None` for a line of a type Demi does not read.
    Line(String, Option<Line>),
    /// The MCP server opened a call.
    Opened(ToolCall),
    /// The output ended.
    End,
    /// The output cannot be read on, or Demi's answer cannot be written.
    Broken(ProviderFailure),
}

pub(crate) struct LiveCli {
    shared: Arc<Shared>,
    key: ProcessKey,
    control: Box<dyn ProcessControl>,
    /// How the process ended, awaited as often as needed.
    exit: SharedFuture<LocalBoxFuture<'static, ProcessEnd>>,
    lines: Lines,
    /// Lines read while the process was being set up, which come first.
    pending: VecDeque<(String, Option<Line>)>,
    stderr: Rc<RefCell<Tail>>,
    _drain: AbortOnDropHandle<()>,
    sent: Sent,
    /// Whether the process streamed a message, after which the model's
    /// tool calls come in batches that `message_stop` ends.
    pub(crate) streamed: bool,
    /// The tool uses of the message being streamed.
    pub(crate) collecting: Vec<ToolCall>,
    /// The batch the last run yielded, whose results the next run delivers.
    pub(crate) held: Vec<ToolCall>,
    pub(crate) mcp: Option<Mcp>,
}

impl LiveCli {
    /// Starts a new process for `request` through `placement`, with the
    /// account's token; the CLI is never started without one.
    pub(crate) async fn start(
        shared: &Arc<Shared>,
        placement: &dyn Placement,
        request: &InferenceRequest,
    ) -> Result<Self, ProviderFailure> {
        let secret = shared
            .auth
            .stored()
            .await
            .map_err(|failure| failure.failure())?;
        let spawn = |site: &CliSite| {
            let spawn = cli::spawn_request(site, request, &secret.access_token);
            // The environment holds the token, so the record leaves it out.
            tracing::trace!(
                target: WIRE,
                direction = "spawn",
                command = %spawn.command,
                cwd = ?spawn.cwd,
                args = ?spawn.args,
            );
            spawn
        };
        let Process {
            output,
            control,
            exit,
        } = placement
            .start(&spawn)
            .await
            .map_err(|error| failure(error.0))?;
        let (stdout, lines) = mpsc::unbounded();
        let stderr = Rc::new(RefCell::new(Tail::default()));
        let drain = tokio::task::spawn_local(drain(output, stdout, stderr.clone()));
        let mcp = (!request.tools.is_empty()).then(|| Mcp::start(request.tools.clone()));
        Ok(Self {
            shared: shared.clone(),
            key: ProcessKey::of(request),
            control,
            exit: exit.shared(),
            lines: FramedRead::new(
                StreamReader::new(lines),
                LinesCodec::new_with_max_length(MAX_LINE_BYTES),
            ),
            pending: VecDeque::new(),
            stderr,
            _drain: AbortOnDropHandle::new(drain),
            sent: Sent::default(),
            streamed: false,
            collecting: Vec::new(),
            held: Vec::new(),
            mcp,
        })
    }

    /// Whether the process can go on with `request`: it still runs, for the
    /// same session, model and thinking setting, it has an MCP server when
    /// the request offers tools, and the transcript continues what it
    /// received.
    pub(crate) fn serves(&self, request: &InferenceRequest) -> bool {
        let running = self.exit.clone().now_or_never().is_none();
        let offers_tools = self.mcp.is_some() || request.tools.is_empty();
        running
            && offers_tools
            && self.key == ProcessKey::of(request)
            && !self.sent.diverged(&request.items)
    }

    /// Sets a new process up for `request`: when the request offers tools,
    /// the `initialize` control request that declares the SDK MCP server,
    /// and its success; then the history as the first messages.
    pub(crate) async fn prepare(
        &mut self,
        request: &InferenceRequest,
    ) -> Result<(), ProviderFailure> {
        if self.mcp.is_some() {
            self.initialize(&request.system_prompt).await?;
        }
        let items = request.items.clone();
        let history = encode_body(FAMILY, move || input::history(&items)).await?;
        self.write(history).await?;
        self.sent = Sent::of(&request.items);
        Ok(())
    }

    /// Sends `initialize` and reads until the CLI's answer. The CLI may start
    /// its MCP handshake meanwhile, so control requests are answered as they
    /// come; any other line is kept for the run.
    async fn initialize(&mut self, system_prompt: &str) -> Result<(), ProviderFailure> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let request = Input::ControlRequest {
            request_id: &request_id,
            request: ControlRequest::Initialize {
                sdk_mcp_servers: [MCP_SERVER],
                system_prompt,
            },
        };
        self.write(input::line(&request)).await?;
        loop {
            match self.advance().await {
                Next::Line(_, Some(Line::ControlResponse(line)))
                    if line.response.as_ref().is_some_and(|response| {
                        response.request_id.as_deref() == Some(request_id.as_str())
                    }) =>
                {
                    let response = line.response.expect("the answer was matched");
                    if response.subtype.as_deref() == Some("success") {
                        return Ok(());
                    }
                    let reason = response
                        .error
                        .into_inner()
                        .unwrap_or_else(|| "no reason given".into());
                    return Err(failure(format!(
                        "Claude Code refused the SDK MCP server: {reason}"
                    )));
                }
                Next::Line(_, Some(Line::ControlRequest(line))) => {
                    self.control_request(line).await?
                }
                Next::Line(text, line) => self.pending.push_back((text, line)),
                // The model has no message to call a tool in before the
                // history is written, which waits for this answer.
                Next::Opened(call) => {
                    return Err(failure(format!(
                        "Claude Code called the tool {} before its initialization completed",
                        call.tool_name
                    )));
                }
                Next::End => {
                    let reason = self.exit_message().await;
                    return Err(failure(format!(
                        "Claude Code exited before SDK MCP initialization completed: {reason}"
                    )));
                }
                Next::Broken(failure) => return Err(failure),
                // Only the run's reading watches its token.
                Next::Cancelled => unreachable!("advancing watches no token"),
            }
        }
    }

    /// What the process does next, unless `cancel` is cancelled first.
    pub(crate) async fn next(&mut self, cancel: &CancellationToken) -> Next {
        cancel
            .run_until_cancelled(self.advance())
            .await
            .unwrap_or(Next::Cancelled)
    }

    /// What the process does next: a line, a call its MCP server opened, or
    /// the end of its output. The server's replies are written to the CLI
    /// meanwhile.
    async fn advance(&mut self) -> Next {
        loop {
            if let Some((text, line)) = self.pending.pop_front() {
                return Next::Line(text, line);
            }
            let event = tokio::select! {
                biased;
                event = next_mcp(&mut self.mcp) => Ok(event),
                line = self.lines.next() => Err(line),
            };
            match event {
                Ok(McpEvent::Reply(request_id, reply)) => {
                    if let Err(failure) = self.reply(request_id, reply).await {
                        return Next::Broken(failure);
                    }
                }
                Ok(McpEvent::Opened(call)) => return Next::Opened(call),
                Err(line) => match self.read(line) {
                    Read::Line(text, line) => return Next::Line(text, line),
                    Read::Blank => {}
                    Read::End => return Next::End,
                    Read::Broken(failure) => return Next::Broken(failure),
                },
            }
        }
    }

    /// Reads what the CLI printed after the run before this one ended: that
    /// output belongs to no request, as when the CLI answered a steer in a
    /// turn of its own. Its control requests are answered and every other
    /// line is skipped, a `result` included.
    pub(crate) async fn skip_leftovers(&mut self) -> Result<(), ProviderFailure> {
        loop {
            let (text, line) = match self.pending.pop_front() {
                Some(pending) => pending,
                None => {
                    let Some(line) = self.lines.next().now_or_never() else {
                        return Ok(());
                    };
                    match self.read(line) {
                        Read::Line(text, line) => (text, line),
                        Read::Blank => continue,
                        // The end is the run's to meet.
                        Read::End => return Ok(()),
                        Read::Broken(failure) => return Err(failure),
                    }
                }
            };
            match line {
                Some(Line::ControlRequest(line)) => self.control_request(line).await?,
                _ => tracing::debug!(line = %text, "skipped a line that belongs to no request"),
            }
        }
    }

    /// One line of output as the run reads it: observed for its quota, then
    /// decoded.
    fn read(&mut self, line: Option<Result<String, LinesCodecError>>) -> Read {
        let text = match line {
            None => return Read::End,
            Some(Err(error)) => {
                let message = format!("Claude Code's output cannot be read: {error}");
                return Read::Broken(failure(message));
            }
            Some(Ok(text)) => text,
        };
        if text.trim().is_empty() {
            return Read::Blank;
        }
        tracing::trace!(target: WIRE, direction = "out", line = %text);
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(error) => return Read::Broken(undecodable(&error, text)),
        };
        self.shared.quota.observe(Observation::CliLine(&value));
        match Tagged::<Line>::deserialize(value) {
            Ok(Tagged(line)) => Read::Line(text, line),
            Err(error) => Read::Broken(undecodable(&error, text)),
        }
    }

    /// Answers a control request of the CLI's: an MCP message goes to the
    /// SDK MCP server, and any other request is refused.
    pub(crate) async fn control_request(
        &mut self,
        line: ControlRequestLine,
    ) -> Result<(), ProviderFailure> {
        let ControlRequestLine {
            request_id,
            request,
        } = line;
        let answer = if request.subtype != "mcp_message" {
            Err(format!(
                "Demi does not serve the control request {}",
                request.subtype
            ))
        } else {
            match (
                &mut self.mcp,
                request.server_name.as_deref(),
                request.message,
            ) {
                (Some(mcp), Some(MCP_SERVER), Some(message)) => {
                    match serde_json::from_value::<ClientJsonRpcMessage>(message) {
                        Ok(message) => mcp.pass(&request_id, message),
                        Err(error) => Err(format!("The SDK MCP message cannot be read: {error}")),
                    }
                }
                (_, server, _) => Err(format!(
                    "No SDK MCP server {} serves this process",
                    server.unwrap_or("(unnamed)")
                )),
            }
        };
        match answer {
            Ok(None) => Ok(()),
            Ok(Some(acknowledgment)) => self.reply(request_id, acknowledgment).await,
            Err(error) => {
                let response = Input::ControlResponse {
                    response: ControlResponse::Error {
                        request_id: &request_id,
                        error,
                    },
                };
                self.write(input::line(&response)).await
            }
        }
    }

    /// Writes the SDK MCP server's `reply` as the answer to control request
    /// `request_id`. A reply may carry a tool's images, so it is serialized
    /// off the shard.
    async fn reply(
        &mut self,
        request_id: String,
        reply: ServerJsonRpcMessage,
    ) -> Result<(), ProviderFailure> {
        let line = encode_body(FAMILY, move || {
            let response = Input::ControlResponse {
                response: ControlResponse::Success {
                    request_id: &request_id,
                    response: McpReply {
                        mcp_response: &reply,
                    },
                },
            };
            Ok::<_, UnloadedMedia>(input::line(&response))
        })
        .await?;
        self.write(line).await
    }

    /// Hands the held batch's results to their calls: each call of the
    /// batch must have a result in `request`.
    pub(crate) async fn deliver(
        &mut self,
        request: &InferenceRequest,
    ) -> Result<(), ProviderFailure> {
        let held = std::mem::take(&mut self.held);
        let missing: Vec<&str> = held
            .iter()
            .map(|call| call.tool_use_id.as_str())
            .filter(|id| tool_result(&request.items, id).is_none())
            .collect();
        if !missing.is_empty() {
            return Err(failure(format!(
                "Claude Code provider missing tool_result for SDK MCP tool_use {}",
                missing.join(", ")
            )));
        }
        let items = request.items.clone();
        let results = encode_body(FAMILY, move || {
            held.iter()
                .map(|call| {
                    let (output, is_error) = tool_result(&items, &call.tool_use_id)
                        .expect("every call of the batch has a result");
                    Ok((
                        call.tool_use_id.clone(),
                        mcp::tool_result(output, is_error)?,
                    ))
                })
                .collect::<Result<Vec<_>, UnloadedMedia>>()
        })
        .await?;
        if let Some(mcp) = &self.mcp {
            mcp.deliver(results);
        }
        Ok(())
    }

    /// Writes the user messages `request` gained since the process last
    /// received one, steers included.
    pub(crate) async fn send_new_user_messages(
        &mut self,
        request: &InferenceRequest,
    ) -> Result<(), ProviderFailure> {
        let count = input::user_messages(&request.items).count();
        if count <= self.sent.count {
            return Ok(());
        }
        let items = request.items.clone();
        let sent = self.sent.count;
        let lines = encode_body(FAMILY, move || input::new_user_messages(&items, sent)).await?;
        self.write(lines).await?;
        self.sent.count = count;
        if self.sent.first.is_none() {
            self.sent.first = first_user_message(&request.items).cloned();
        }
        Ok(())
    }

    /// Adds a tool use to the batch being collected; a tool use the CLI
    /// repeats is kept once, where it first came.
    pub(crate) fn collect(&mut self, call: ToolCall) {
        match self
            .collecting
            .iter_mut()
            .find(|collected| collected.tool_use_id == call.tool_use_id)
        {
            Some(collected) => *collected = call,
            None => self.collecting.push(call),
        }
    }

    async fn write(&mut self, bytes: Vec<u8>) -> Result<(), ProviderFailure> {
        if tracing::enabled!(target: WIRE, tracing::Level::TRACE) {
            for line in bytes
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
            {
                tracing::trace!(target: WIRE, direction = "in", line = %String::from_utf8_lossy(line));
            }
        }
        self.control
            .write_stdin(Bytes::from(bytes))
            .await
            .map_err(|error| failure(format!("Claude Code's input could not be written: {error}")))
    }

    /// The failure of a process whose output ended: results it was given and
    /// never asked for, or an exit other than success, told by the tail of
    /// its standard error when it wrote one.
    pub(crate) async fn end(self) -> Option<ProviderFailure> {
        let unasked = self.mcp.as_ref().map(Mcp::unasked).unwrap_or_default();
        let stderr = self.stderr.borrow().text();
        let exit = self.exit.clone().await;
        tracing::trace!(target: WIRE, direction = "exit", end = ?exit);
        if !unasked.is_empty() {
            return Some(failure(format!(
                "Claude Code exited before requesting SDK MCP tool result for {}",
                unasked.join(", ")
            )));
        }
        if exit == ProcessEnd::Exited(0) {
            return None;
        }
        Some(failure(if stderr.is_empty() {
            exit_message(&exit)
        } else {
            stderr
        }))
    }

    /// Why the process ended, by the tail of its standard error, else by its
    /// end, for a process that failed while it was set up.
    async fn exit_message(&self) -> String {
        let stderr = self.stderr.borrow().text();
        if !stderr.is_empty() {
            return stderr;
        }
        exit_message(&self.exit.clone().await)
    }

    /// Ends the process: its MCP server stops, SIGTERM asks it to end, and
    /// SIGKILL follows after five seconds; either way its end is awaited.
    pub(crate) async fn close(self) {
        let LiveCli {
            control, exit, mcp, ..
        } = self;
        drop(mcp);
        // A process that already ended, or whose machine went away, takes no
        // signal; its end is known either way.
        let _ = control.kill(Signal::Terminate).await;
        let end = match tokio::time::timeout(CLOSE_GRACE, exit.clone()).await {
            Ok(end) => end,
            Err(_) => {
                let _ = control.kill(Signal::Kill).await;
                exit.await
            }
        };
        tracing::trace!(target: WIRE, direction = "exit", end = ?end);
    }
}

/// The server's next event; a process without tools has none.
async fn next_mcp(mcp: &mut Option<Mcp>) -> McpEvent {
    match mcp {
        Some(mcp) => mcp.next().await,
        None => std::future::pending().await,
    }
}

/// The latest result the transcript holds for `tool_use_id`.
fn tool_result<'a>(
    items: &'a [InferenceItem],
    tool_use_id: &str,
) -> Option<(&'a [demi_core::ToolResultContentBlock], bool)> {
    items.iter().rev().find_map(|item| match item {
        InferenceItem::ToolResult {
            tool_use_id: id,
            output,
            is_error,
        } if id == tool_use_id => Some((output.as_slice(), *is_error)),
        _ => None,
    })
}

/// Forwards the process's standard output to the run and keeps the tail of
/// its standard error, until the output ends.
async fn drain(
    mut output: LocalBoxStream<'static, ProcessOutput>,
    stdout: mpsc::UnboundedSender<io::Result<Bytes>>,
    stderr: Rc<RefCell<Tail>>,
) {
    while let Some(chunk) = output.next().await {
        match chunk.stream {
            // The receiver goes with the process, which stops this task too.
            StreamKind::Stdout => {
                let _ = stdout.unbounded_send(Ok(chunk.bytes));
            }
            StreamKind::Stderr => {
                tracing::trace!(target: WIRE, direction = "err", text = %String::from_utf8_lossy(&chunk.bytes));
                stderr.borrow_mut().push(&chunk.bytes);
            }
        }
    }
}

/// The last bytes of a stream.
#[derive(Default)]
struct Tail(Vec<u8>);

impl Tail {
    fn push(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
        let excess = self.0.len().saturating_sub(STDERR_TAIL_BYTES);
        self.0.drain(..excess);
    }

    /// The tail as text, trimmed; a character the cut split is left out.
    fn text(&self) -> String {
        let start = self
            .0
            .iter()
            .position(|byte| byte & 0b1100_0000 != 0b1000_0000)
            .unwrap_or(self.0.len());
        String::from_utf8_lossy(&self.0[start..]).trim().to_owned()
    }
}

/// A failure Demi composes, which is never retried by itself.
fn failure(message: String) -> ProviderFailure {
    ProviderFailure {
        message,
        code: None,
        diagnostics: None,
        retry_after: None,
    }
}

/// The failure of a line Demi cannot read, with the line as its record.
fn undecodable(error: &dyn std::fmt::Display, text: String) -> ProviderFailure {
    ProviderFailure::protocol(
        format!("Claude Code sent a line Demi cannot read: {error}"),
        text,
    )
}

fn exit_message(end: &ProcessEnd) -> String {
    match end {
        ProcessEnd::Exited(code) => format!("Claude Code exited with code {code}"),
        ProcessEnd::Signalled(signal) => format!("Claude Code was ended by {signal}"),
        ProcessEnd::NotStarted(error) => match &error.detail {
            Some(detail) => format!("Claude Code did not start ({}): {detail}", error.kind),
            None => format!("Claude Code did not start ({})", error.kind),
        },
        ProcessEnd::Lost(reason) => format!("Claude Code's machine went away: {reason}"),
    }
}
