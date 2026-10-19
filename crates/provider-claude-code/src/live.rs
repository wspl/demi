//! A CLI process a runtime keeps across the turns of a session
//! (`claude-code.md` § Process lifetime): its input and output, its SDK MCP
//! server, what it has received of the transcript, the tool calls it holds,
//! and the blocks its session's entries belong to. Its standard error is
//! drained all the time, keeping the last 64 KiB. A process is closed,
//! never killed outright: [`LiveCli::close`] sends SIGTERM, waits up to five
//! seconds, then sends SIGKILL, removes the process's own directory and
//! waits; dropping it closes it the same way, as a task nobody waits for,
//! since a process killed while it renews the account's sign-in stops the
//! account for a minute (`claude-code.md` § Process lifetime).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use demi_host_interface::{Process, ProcessControl, ProcessEnd, ProcessOutput, Signal};
use demi_provider_common::quota::Observation;
use demi_provider_common::wire::Tagged;
use demi_provider_common::{
    CallEvent, InferenceItem, InferenceRequest, ProviderEvent, ProviderFailure, ResultPart,
    ToolCall, UserPart, encode_body,
};
use demi_shared_types::{StreamKind, ThinkingConfig, Timestamp};
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
use crate::mirror::Mirror;
use crate::output::{ControlRequestLine, LastCall, Line, Replaced, Streaming};
use crate::placement::{CliSite, ConfigDir, Placed, Placement};
use crate::session::SessionFile;
use crate::timing::StartTiming;
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
    FramedRead<StreamReader<mpsc::UnboundedReceiver<io::Result<Bytes>>, Bytes>, CliLines>;

/// Lines as `LinesCodec` reads them, but output that ends in the middle of a
/// line, as when the process's machine went away while it wrote, is not a
/// line: the rest is dropped and `cut` says so, so the end is read by how
/// the process ended rather than as a line Demi cannot read.
struct CliLines {
    lines: LinesCodec,
    cut: Rc<Cell<bool>>,
}

impl tokio_util::codec::Decoder for CliLines {
    type Item = String;
    type Error = LinesCodecError;

    fn decode(&mut self, buf: &mut bytes::BytesMut) -> Result<Option<String>, LinesCodecError> {
        self.lines.decode(buf)
    }

    fn decode_eof(&mut self, buf: &mut bytes::BytesMut) -> Result<Option<String>, LinesCodecError> {
        if let Some(line) = self.lines.decode(buf)? {
            return Ok(Some(line));
        }
        if !buf.is_empty() {
            buf.clear();
            self.cut.set(true);
        }
        Ok(None)
    }
}

/// What a process was started for: another session, model or thinking
/// setting needs a new process, since each is fixed per process.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProcessKey {
    session_id: String,
    model_id: String,
    thinking: Option<ThinkingConfig>,
}

impl ProcessKey {
    pub(crate) fn of(request: &InferenceRequest) -> Self {
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
    first: Option<Vec<UserPart>>,
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

fn first_user_message(items: &[InferenceItem]) -> Option<&Vec<UserPart>> {
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

/// What a process did while it waited for a request to take it.
pub(crate) enum Waited {
    /// The MCP server's reply to a control request.
    Reply(String, Box<ServerJsonRpcMessage>),
    /// The MCP server opened a call.
    Opened(ToolCall),
    /// The process printed a line, or its output ended.
    Output(Option<Result<String, LinesCodecError>>),
}

pub(crate) struct LiveCli {
    shared: Arc<Shared>,
    key: ProcessKey,
    /// The process's control and its own directory, which go with it.
    handle: Handle,
    /// How the process ended, awaited as often as needed.
    exit: SharedFuture<LocalBoxFuture<'static, ProcessEnd>>,
    lines: Lines,
    /// The output ended in the middle of a line (`CliLines`).
    cut: Rc<Cell<bool>>,
    /// Lines read while the process was being set up, which come first.
    pending: VecDeque<(String, Option<Line>)>,
    stderr: Rc<RefCell<Tail>>,
    _drain: AbortOnDropHandle<()>,
    sent: Sent,
    /// The user messages written to the process that it has not printed
    /// back yet: a `result` ends a run only once there are none
    /// (`claude-code.md` § Which `result` ends a run).
    pub(crate) unreplayed: usize,
    /// Whether the CLI printed a message back or started a model call
    /// since its last `result`: a turn that did neither was one it answered
    /// itself, such as a `/cost` it does not offer.
    pub(crate) turn_began: bool,
    /// The response being streamed, from its first stream event until its
    /// `message_stop`: its tool calls come in the batch that ends it. A
    /// response that comes whole hands its batch on at once.
    pub(crate) streaming: Option<Streaming>,
    /// The tool uses of the response being read.
    pub(crate) collecting: Vec<ToolCall>,
    /// The tool uses the message being streamed opened, by their block's
    /// index, whose input pieces name only that index.
    pub(crate) writing: HashMap<u64, String>,
    /// The batch the last run yielded, whose results the next run delivers.
    pub(crate) held: Vec<ToolCall>,
    /// The usage of the latest API call, which the turn's response carries.
    pub(crate) last_call: LastCall,
    pub(crate) mcp: Option<Mcp>,
    /// Which block each entry the process mirrors belongs to.
    pub(crate) mirror: Mirror,
    /// How the process's start went, until the first run that takes it
    /// reports it.
    pub(crate) started: Option<StartTiming>,
    /// When the call the output belongs to now started, as the usage ledger
    /// counts calls: a run's first call with the run, each later one when
    /// its first line is read. A `rate_limit_event` follows the
    /// `message_stop` of the call it reports on, so its windows carry this
    /// time (`usage-and-quota.md` § Probe and observation).
    call_requested_at: Timestamp,
    /// Whether the run's first call, which started with the run, is yet to
    /// show its first line.
    first_call_unseen: bool,
}

impl LiveCli {
    /// Starts a new process for `request` through `placement`, in the
    /// account's directory, which the CLI reads its sign-in from; an
    /// account the vendor signed out starts none. The process resumes the
    /// session written from the request's blocks (`claude-code.md` § The
    /// session a process resumes). A sign-in an earlier release kept in the
    /// account's record goes to the directory with the start, and the
    /// record's secret is cleared once the process started.
    pub(crate) async fn start(
        shared: &Arc<Shared>,
        placement: &dyn Placement,
        request: &InferenceRequest,
    ) -> Result<Self, ProviderFailure> {
        let account = shared
            .auth
            .work()
            .await
            .map_err(|failure| failure.failure())?;
        let building = tokio::time::Instant::now();
        let file = {
            let session = request.session_id.clone();
            let items = request.items.clone();
            let blocks = request.blocks.clone();
            let timestamp = shared.clock.now().to_jiff().to_string();
            encode_body(FAMILY, move || {
                SessionFile::new(&session, &items, &blocks, timestamp)
            })
            .await?
        };
        let session_build = building.elapsed();
        let args = cli::args(request, &file.id.to_string());
        let start = Arc::new(move |site: &CliSite| {
            let start = cli::start(site, args.clone(), &file);
            // The environment is left out: the machine's own may hold
            // anything.
            tracing::trace!(
                target: WIRE,
                direction = "spawn",
                command = %start.spawn.command,
                cwd = ?start.spawn.cwd,
                args = ?start.spawn.args,
            );
            start
        });
        let Placed {
            process,
            config,
            steps,
        } = placement
            .start(account.work, start, &|_| {})
            .await
            // The machine could not be reached or readied, as when its
            // connection is being replaced or its network fails a lookup:
            // the agent's retry starts the process again.
            .map_err(|error| ProviderFailure::no_answer(error.0))?;
        if let Some(version) = account.handed_over {
            shared.auth.handed_over(version).await;
        }
        let Process {
            output,
            control,
            exit,
        } = process;
        let (stdout, lines) = mpsc::unbounded();
        let stderr = Rc::new(RefCell::new(Tail::default()));
        let drain = tokio::task::spawn_local(drain(output, stdout, stderr.clone()));
        let mcp = (!request.tools.is_empty()).then(|| Mcp::start(request.tools.clone()));
        let cut = Rc::new(Cell::new(false));
        let exit = exit.shared();
        Ok(Self {
            shared: shared.clone(),
            key: ProcessKey::of(request),
            handle: Handle(Some(Parts {
                control,
                exit: exit.clone(),
                config: Some(config),
            })),
            exit,
            lines: FramedRead::new(
                StreamReader::new(lines),
                CliLines {
                    lines: LinesCodec::new_with_max_length(MAX_LINE_BYTES),
                    cut: cut.clone(),
                },
            ),
            cut,
            pending: VecDeque::new(),
            stderr,
            _drain: AbortOnDropHandle::new(drain),
            sent: Sent::default(),
            unreplayed: 0,
            turn_began: false,
            streaming: None,
            collecting: Vec::new(),
            writing: HashMap::new(),
            held: Vec::new(),
            last_call: LastCall::default(),
            mcp,
            mirror: Mirror::default(),
            started: Some(StartTiming {
                steps,
                session_build,
                initialize: None,
            }),
            // No call starts before a run gives the process input.
            call_requested_at: shared.clock.now(),
            first_call_unseen: false,
        })
    }

    /// A run that started at `at` takes the process: its first call
    /// started with it.
    pub(crate) fn run_started(&mut self, at: Timestamp) {
        self.call_requested_at = at;
        self.first_call_unseen = true;
    }

    /// The process showed the first line of a call: the run's first, which
    /// started with the run, or a later one, which starts now.
    pub(crate) fn call_started(&mut self) -> ProviderEvent {
        if !std::mem::take(&mut self.first_call_unseen) {
            self.call_requested_at = self.shared.clock.now();
        }
        ProviderEvent::Call(CallEvent::Started)
    }

    /// Whether the process can go on with `request`: it still runs, for the
    /// same session, model and thinking setting, it has an MCP server when
    /// the request offers tools, the transcript continues what it received,
    /// and the request gives it something to answer: results for the calls
    /// it holds, or a new user message. A retry of a failed turn gives it
    /// neither, and the process would wait for input that never comes.
    pub(crate) fn serves(&self, request: &InferenceRequest) -> bool {
        let gains_input =
            !self.held.is_empty() || input::user_messages(&request.items).count() > self.sent.count;
        self.continues(request) && gains_input
    }

    /// Whether the process could go on with `request`, whatever its input:
    /// it still runs, for the same session, model and
    /// thinking setting, it has an MCP server when the request offers
    /// tools, and the transcript continues what it received.
    pub(crate) fn continues(&self, request: &InferenceRequest) -> bool {
        let running = self.exit.clone().now_or_never().is_none();
        let offers_tools = self.mcp.is_some() || request.tools.is_empty();
        running
            && offers_tools
            && self.key == ProcessKey::of(request)
            && !self.sent.diverged(&request.items)
    }

    /// Sets a new process up for `request`: when the request offers tools,
    /// the `initialize` control request that declares the SDK MCP server,
    /// and its success. The process holds the whole transcript in the
    /// session it resumed, and answers the input it ends with.
    pub(crate) async fn prepare(
        &mut self,
        request: &InferenceRequest,
    ) -> Result<(), ProviderFailure> {
        if self.mcp.is_some() {
            let initializing = tokio::time::Instant::now();
            self.initialize(&request.system_prompt).await?;
            if let Some(started) = &mut self.started {
                started.initialize = Some(initializing.elapsed());
            }
        }
        self.sent = Sent::of(&request.items);
        Ok(())
    }

    /// Sends `initialize` and reads until the CLI's answer. The CLI may start
    /// its MCP handshake meanwhile, so control requests are answered as they
    /// come; any other line, such as the `system` line the CLI prints first,
    /// is kept for the run.
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
        // The run's lines wait here until the answer: `advance` reads the
        // kept lines first, so a line kept among them now would come back at
        // once, and again, without the process ever being read.
        let mut kept = VecDeque::new();
        loop {
            match self.advance().await {
                Next::Line(_, Some(Line::ControlResponse(line)))
                    if line.response.as_ref().is_some_and(|response| {
                        response.request_id.as_deref() == Some(request_id.as_str())
                    }) =>
                {
                    let response = line.response.expect("the answer was matched");
                    if response.subtype.as_deref() == Some("success") {
                        self.pending.append(&mut kept);
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
                Next::Line(text, line) => kept.push_back((text, line)),
                // The model has no message to call a tool in before the
                // transcript is written, which waits for this answer.
                Next::Opened(call) => {
                    return Err(failure(format!(
                        "Claude Code called the tool {} before its initialization completed",
                        call.tool_name
                    )));
                }
                Next::End => {
                    let reason = self.exit_message().await;
                    return Err(ended(
                        &self.exit.clone().await,
                        format!("Claude Code exited before SDK MCP initialization completed: {reason}"),
                    ));
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
                    if let Err(failure) = self.reply(request_id, *reply).await {
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

    /// What the process does next while it waits for a request; dropping
    /// the wait loses nothing.
    pub(crate) async fn wait(&mut self) -> Waited {
        tokio::select! {
            biased;
            event = next_mcp(&mut self.mcp) => match event {
                McpEvent::Reply(request_id, reply) => Waited::Reply(request_id, reply),
                McpEvent::Opened(call) => Waited::Opened(call),
            },
            line = self.lines.next() => Waited::Output(line),
        }
    }

    /// Answers what a waiting process did: its control requests are
    /// answered and the MCP server's replies written, and any other line is
    /// kept for the request that takes it. A process that ends, breaks or
    /// calls a tool before it was given any input fails.
    pub(crate) async fn answer_waiting(&mut self, waited: Waited) -> Result<(), ProviderFailure> {
        match waited {
            Waited::Reply(request_id, reply) => self.reply(request_id, *reply).await,
            Waited::Opened(call) => Err(failure(format!(
                "Claude Code called the tool {} before it was given any input",
                call.tool_name
            ))),
            Waited::Output(line) => match self.read(line) {
                Read::Line(_, Some(Line::ControlRequest(line))) => self.control_request(line).await,
                Read::Line(text, line) => {
                    self.pending.push_back((text, line));
                    Ok(())
                }
                Read::Blank => Ok(()),
                Read::End => {
                    let reason = self.exit_message().await;
                    Err(ended(&self.exit.clone().await, reason))
                }
                Read::Broken(failure) => Err(failure),
            },
        }
    }

    /// Whether the process printed nothing that answers a request since the
    /// `result` that ended the last run, as a process in step with its
    /// requests never does (`claude-code.md` § Which `result` ends a run).
    /// Its control requests are answered, since they are no turn's output,
    /// and the lines the decoder does not read, the CLI's status lines such
    /// as `command_lifecycle`, are read past, since they answer no message;
    /// any other output waiting, or an end, means the process went on
    /// without a request, and the run starts a new one from the transcript
    /// instead.
    pub(crate) async fn quiet(&mut self) -> bool {
        loop {
            let Some(line) = self.lines.next().now_or_never() else {
                return true;
            };
            match self.read(line) {
                Read::Blank | Read::Line(_, None) => {}
                Read::Line(_, Some(Line::ControlRequest(line))) => {
                    if self.control_request(line).await.is_err() {
                        return false;
                    }
                }
                Read::Line(text, _) => {
                    tracing::warn!(line = %text, "a kept Claude Code process printed output that answers no request");
                    return false;
                }
                Read::End | Read::Broken(_) => return false,
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
        self.shared
            .quota
            .observe(Observation::CliLine(&value), self.call_requested_at);
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
            input::line(&response)
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
                    (call.tool_use_id.clone(), mcp::tool_result(output, is_error))
                })
                .collect::<Vec<_>>()
        })
        .await?;
        if let Some(mcp) = &self.mcp {
            mcp.deliver(results);
        }
        Ok(())
    }

    /// Abandons the response being streamed, whose calls never run and
    /// whose entries belong to no block: what it showed, which the caller
    /// keeps out of the session where nothing repeats it.
    pub(crate) fn abandon_streaming(&mut self) -> Replaced {
        self.collecting.clear();
        self.writing.clear();
        let streaming = self.streaming.take().unwrap_or_default();
        if let Some(id) = streaming.id() {
            self.mirror.abandon(id.to_owned());
        }
        streaming.replaced()
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
        let given = input::user_message_items(&request.items)
            .skip(sent)
            .map(|(index, _)| index);
        self.mirror.gave(given);
        self.unreplayed += count - sent;
        self.sent.count = count;
        if self.sent.first.is_none() {
            self.sent.first = first_user_message(&request.items).cloned();
        }
        Ok(())
    }

    /// Hands `batch` on to the agent and keeps it for the next run, which
    /// delivers its results: the calls, the entries now placed, and the
    /// response with the usage of the call that wrote them (`compaction.md`
    /// § When compaction runs). The run ends with these events.
    pub(crate) fn hand_over(
        &mut self,
        batch: Vec<ToolCall>,
        request: &InferenceRequest,
    ) -> Vec<ProviderEvent> {
        self.collecting.clear();
        self.writing.clear();
        self.streaming = None;
        let mut events: Vec<ProviderEvent> =
            batch.iter().cloned().map(ProviderEvent::ToolCall).collect();
        self.held = batch;
        for call in &events {
            self.mirror.observe(call);
        }
        events.extend(self.mirror.place(request, false, None));
        events.push(ProviderEvent::response(
            self.last_call.take(),
            FAMILY,
            &request.model_id,
        ));
        events
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
        self.handle
            .control()
            .write_stdin(Bytes::from(bytes))
            .await
            .map_err(|error| failure(format!("Claude Code's input could not be written: {error}")))
    }

    /// The failure of a process whose output ended: results it was given and
    /// never asked for, or an exit other than success, told by the tail of
    /// its standard error when it wrote one.
    pub(crate) async fn end(mut self) -> Option<ProviderFailure> {
        let unasked = self.mcp.as_ref().map(Mcp::unasked).unwrap_or_default();
        let stderr = self.stderr.borrow().text();
        let exit = self.exit.clone().await;
        tracing::trace!(target: WIRE, direction = "exit", end = ?exit);
        self.handle.ended().await;
        if matches!(exit, ProcessEnd::Lost(_)) {
            return Some(ended(&exit, exit_message(&exit)));
        }
        if self.cut.get() {
            return Some(failure(format!(
                "Claude Code's output ended in the middle of a line: {}",
                exit_message(&exit)
            )));
        }
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
    pub(crate) async fn close(mut self) {
        drop(self.mcp.take());
        self.handle.close().await;
    }
}

/// A process's control, its end and its own directory: closed, SIGTERM
/// first, however the process goes, its directory removed once it ended.
/// Every CLI process the provider holds is held this way, so none is
/// dropped raw, which would kill it outright.
pub(crate) struct Handle(Option<Parts>);

struct Parts {
    control: Box<dyn ProcessControl>,
    exit: SharedFuture<LocalBoxFuture<'static, ProcessEnd>>,
    config: Option<Box<dyn ConfigDir>>,
}

impl Handle {
    /// Holds the process `control` drives, whose end is `exit`, with its
    /// own directory `config`, when it has one.
    pub(crate) fn of(
        control: Box<dyn ProcessControl>,
        exit: SharedFuture<LocalBoxFuture<'static, ProcessEnd>>,
        config: Option<Box<dyn ConfigDir>>,
    ) -> Self {
        Self(Some(Parts {
            control,
            exit,
            config,
        }))
    }

    pub(crate) fn control(&self) -> &dyn ProcessControl {
        // The parts go only with the handle's end, after which nothing
        // writes to the process.
        &*self.0.as_ref().expect("the process is not closed").control
    }

    /// Closes the process and waits for its end.
    pub(crate) async fn close(&mut self) {
        if let Some(parts) = self.0.take() {
            close(parts).await;
        }
    }

    /// Waits for the process to end by itself, then removes its
    /// directory. Dropping the future while it waits leaves the process
    /// held, to be closed.
    pub(crate) async fn ended(&mut self) {
        let Some(parts) = &self.0 else {
            return;
        };
        parts.exit.clone().await;
        if let Some(Parts { config, .. }) = self.0.take()
            && let Some(config) = config
        {
            config.remove().await;
        }
    }
}

impl Drop for Handle {
    /// A process nobody closed, as when the turn drops its run's stream, is
    /// closed by a task of its own, SIGTERM first.
    fn drop(&mut self) {
        if let Some(parts) = self.0.take() {
            tokio::task::spawn_local(close(parts));
        }
    }
}

/// Closes a process: SIGTERM asks it to end, and SIGKILL follows after five
/// seconds; its end is awaited, then its directory removed.
async fn close(parts: Parts) {
    let Parts {
        control,
        exit,
        config,
    } = parts;
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
    if let Some(config) = config {
        config.remove().await;
    }
}

/// Closes the process `control` drives, whose end is `exit`, SIGTERM first
/// and SIGKILL five seconds later, and waits for its end: how anything that
/// holds a CLI process closes it, a sign-in or the backend's relay of a
/// process to another user's shard (`claude-code.md` § Process lifetime).
pub async fn close_process(
    control: Box<dyn ProcessControl>,
    exit: SharedFuture<LocalBoxFuture<'static, ProcessEnd>>,
) {
    close(Parts {
        control,
        exit,
        config: None,
    })
    .await;
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
) -> Option<(&'a [ResultPart], bool)> {
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
pub(crate) struct Tail(Vec<u8>);

impl Tail {
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
        let excess = self.0.len().saturating_sub(STDERR_TAIL_BYTES);
        self.0.drain(..excess);
    }

    /// The tail as text, trimmed; a character the cut split is left out.
    pub(crate) fn text(&self) -> String {
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

/// The failure of a process that ended as `end` says: one whose machine went
/// away, as when the Host's runner reconnected, had no answer and is
/// retried; any other end is the CLI's own failure.
fn ended(end: &ProcessEnd, message: String) -> ProviderFailure {
    match end {
        ProcessEnd::Lost(_) => ProviderFailure::no_answer(message),
        _ => failure(message),
    }
}

/// The failure of a line Demi cannot read, with the line as its record.
fn undecodable(error: &dyn std::fmt::Display, text: String) -> ProviderFailure {
    ProviderFailure::protocol(
        format!("Claude Code sent a line Demi cannot read: {error}"),
        text,
    )
}

pub(crate) fn exit_message(end: &ProcessEnd) -> String {
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
