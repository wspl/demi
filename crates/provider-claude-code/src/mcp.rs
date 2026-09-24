//! The SDK MCP channel (`claude-code.md` § The SDK MCP channel, § Tool-call
//! batches): Demi's MCP server for one CLI process, an rmcp server in local
//! mode on the user's shard, reached through an in-memory transport. The run
//! passes it each MCP message the CLI sends in a control request and writes
//! its replies back. A `tools/call` waits for the agent's result, which a
//! later run delivers; rmcp handles each request on its own task, so `ping`
//! and `tools/list` are answered meanwhile.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use demi_core::{ToolMediaSource, ToolResultContentBlock};
use demi_provider::{ToolCall, ToolDefinition, UnloadedMedia};
use futures_channel::mpsc;
use futures_util::StreamExt as _;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ClientJsonRpcMessage, ContentBlock,
    Implementation, JsonRpcMessage, ListToolsResult, PaginatedRequestParams, RequestId,
    ServerCapabilities, ServerConfig, ServerJsonRpcMessage, ServerResult, Tool,
};
use rmcp::service::{RequestContext, RoleServer, serve_server_with_ct};
use rmcp::{ErrorData as McpError, ServerHandler};
use tokio::sync::oneshot;
use tokio_util::sync::{CancellationToken, DropGuard};
use tokio_util::task::AbortOnDropHandle;

use crate::output::tool_name;

/// Where a `tools/call` names the model's own tool-use id.
const TOOL_USE_ID: &str = "claudecode/toolUseId";

/// A process's MCP server and the run's end of it.
pub(crate) struct Mcp {
    /// MCP messages from the CLI, into the server.
    to_server: mpsc::UnboundedSender<ClientJsonRpcMessage>,
    /// The server's replies, which the run writes back to the CLI.
    from_server: mpsc::UnboundedReceiver<ServerJsonRpcMessage>,
    /// Each call the server opens, as it opens.
    opened: mpsc::UnboundedReceiver<ToolCall>,
    board: Rc<RefCell<Board>>,
    /// The control request each MCP request came in, by the request's id,
    /// oldest first, so that a reply goes back in the right one.
    requests: HashMap<RequestId, VecDeque<String>>,
    _server: AbortOnDropHandle<()>,
    /// Stops the server, and with it every handler still waiting.
    _stop: DropGuard,
}

/// What the server shares with the run: the tools it lists, and the
/// meeting of each call with its result.
struct Board {
    /// The current request's tools, which `tools/list` answers.
    tools: Arc<[ToolDefinition]>,
    /// Calls the CLI made whose result the agent has not given yet, by
    /// tool-use id.
    open: HashMap<String, oneshot::Sender<CallToolResult>>,
    /// Results the agent gave before the CLI asked for them, by tool-use id.
    ready: HashMap<String, CallToolResult>,
    /// Why the server ended its handshake, when it failed.
    failure: Option<String>,
}

/// Something the server did that the run acts on.
pub(crate) enum McpEvent {
    /// A reply, and the control request it answers.
    Reply(String, ServerJsonRpcMessage),
    /// A call the server opened.
    Opened(ToolCall),
}

impl Mcp {
    /// Starts the server of one process, listing `tools`.
    pub(crate) fn start(tools: Arc<[ToolDefinition]>) -> Self {
        let (to_server, inbox) = mpsc::unbounded::<ClientJsonRpcMessage>();
        let (outbox, from_server) = mpsc::unbounded::<ServerJsonRpcMessage>();
        let (announce, opened) = mpsc::unbounded::<ToolCall>();
        let board = Rc::new(RefCell::new(Board {
            tools,
            open: HashMap::new(),
            ready: HashMap::new(),
            failure: None,
        }));
        let stop = CancellationToken::new();
        let server = Server {
            board: board.clone(),
            announce,
        };
        let task = {
            let board = board.clone();
            let stop = stop.clone();
            tokio::task::spawn_local(async move {
                match serve_server_with_ct(server, (outbox, inbox), stop).await {
                    // Serves until the process's end stops it.
                    Ok(running) => {
                        let _ = running.waiting().await;
                    }
                    Err(error) => board.borrow_mut().failure = Some(error.to_string()),
                }
            })
        };
        Self {
            to_server,
            from_server,
            opened,
            board,
            requests: HashMap::new(),
            _server: AbortOnDropHandle::new(task),
            _stop: stop.drop_guard(),
        }
    }

    /// Makes `tools` the ones `tools/list` answers.
    pub(crate) fn offer(&self, tools: Arc<[ToolDefinition]>) {
        self.board.borrow_mut().tools = tools;
    }

    /// Passes one MCP message the CLI sent in control request `request_id`
    /// to the server. A request is answered by the server's reply, later; any
    /// other message is acknowledged at once, and the acknowledgment is
    /// returned to be written.
    pub(crate) fn pass(
        &mut self,
        request_id: &str,
        message: ClientJsonRpcMessage,
    ) -> Result<Option<ServerJsonRpcMessage>, String> {
        let request = match &message {
            JsonRpcMessage::Request(request) => Some(request.id.clone()),
            _ => None,
        };
        if self.to_server.unbounded_send(message).is_err() {
            let reason = self.board.borrow().failure.clone();
            return Err(reason.unwrap_or_else(|| "the SDK MCP server ended".into()));
        }
        match request {
            Some(id) => {
                self.requests
                    .entry(id)
                    .or_default()
                    .push_back(request_id.to_owned());
                Ok(None)
            }
            None => Ok(Some(ServerJsonRpcMessage::response(
                ServerResult::empty(()),
                RequestId::Number(0),
            ))),
        }
    }

    /// The server's next reply or the next call it opened. A call that is no
    /// longer open, such as one answered from a stored result, is skipped. A
    /// server that ended has nothing more; the CLI learns that from the
    /// answers to what it sends ([`Mcp::pass`]).
    pub(crate) async fn next(&mut self) -> McpEvent {
        loop {
            tokio::select! {
                biased;
                Some(reply) = self.from_server.next() => {
                    if let Some(event) = self.request_of(reply) {
                        return event;
                    }
                }
                Some(call) = self.opened.next() => {
                    if self.board.borrow().open.contains_key(&call.tool_use_id) {
                        return McpEvent::Opened(call);
                    }
                }
                else => std::future::pending::<()>().await,
            }
        }
    }

    /// `reply` with the control request it answers; none for a message that
    /// answers no request of the CLI's, which the CLI would not expect.
    fn request_of(&mut self, reply: ServerJsonRpcMessage) -> Option<McpEvent> {
        let id = match &reply {
            JsonRpcMessage::Response(response) => Some(&response.id),
            JsonRpcMessage::Error(error) => error.id.as_ref(),
            JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) => None,
        }?;
        let waiting = self.requests.get_mut(id)?;
        let request_id = waiting.pop_front()?;
        if waiting.is_empty() {
            self.requests.remove(id);
        }
        Some(McpEvent::Reply(request_id, reply))
    }

    /// Hands the agent's results to their calls: a call the CLI made gets its
    /// result now, and one it has not made yet finds it when it does.
    pub(crate) fn deliver(&self, results: Vec<(String, CallToolResult)>) {
        let mut board = self.board.borrow_mut();
        for (tool_use_id, result) in results {
            match board.open.remove(&tool_use_id) {
                // A handler that gave up, as when the CLI cancelled the call,
                // takes nothing.
                Some(call) => {
                    let _ = call.send(result);
                }
                None => {
                    board.ready.insert(tool_use_id, result);
                }
            }
        }
    }

    /// The results the CLI never asked for, by tool-use id.
    pub(crate) fn unasked(&self) -> Vec<String> {
        let mut unasked: Vec<String> = self.board.borrow().ready.keys().cloned().collect();
        unasked.sort();
        unasked
    }
}

/// Demi's MCP server for one process.
struct Server {
    board: Rc<RefCell<Board>>,
    /// Tells the run of each call that opens.
    announce: mpsc::UnboundedSender<ToolCall>,
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("demi", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = self.board.borrow().tools.clone();
        Ok(ListToolsResult::with_all_items(
            tools
                .iter()
                .map(|tool| {
                    Tool::new(
                        tool.name.clone(),
                        tool.description.clone(),
                        Arc::new(tool.input_schema.clone()),
                    )
                })
                .collect(),
        ))
    }

    /// A call of the model's, by its original tool-use id when the CLI
    /// names it, else a new unique one. Its result comes from the agent: at
    /// once when it was given already, otherwise when a run delivers it.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let tool_use_id = context
            .meta
            .get(TOOL_USE_ID)
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
            .map_or_else(
                || format!("mcp-control-{}", uuid::Uuid::new_v4()),
                str::to_owned,
            );
        let call = ToolCall {
            tool_use_id: tool_use_id.clone(),
            tool_name: tool_name(&request.name).to_owned(),
            input: request
                .arguments
                .map_or_else(|| serde_json::json!({}), serde_json::Value::Object),
        };
        let receiver = {
            let mut board = self.board.borrow_mut();
            if let Some(result) = board.ready.remove(&tool_use_id) {
                return Ok(result.into());
            }
            let (sender, receiver) = oneshot::channel();
            board.open.insert(tool_use_id.clone(), sender);
            receiver
        };
        // The run is gone only when the process is closing, which stops
        // this handler too.
        let _ = self.announce.unbounded_send(call);
        tokio::select! {
            result = receiver => result
                .map(CallToolResponse::from)
                .map_err(|_| McpError::internal_error("The tool call ended with its process", None)),
            () = context.ct.cancelled() => {
                self.board.borrow_mut().open.remove(&tool_use_id);
                Err(McpError::internal_error("The tool call was cancelled", None))
            }
        }
    }
}

/// A tool's result as MCP content: text, an image as base64 with its media
/// type, and a video as text naming its media type, since MCP has no video;
/// a failed tool sets the error flag. A result naming media that was not
/// loaded cannot be sent.
pub(crate) fn tool_result(
    output: &[ToolResultContentBlock],
    is_error: bool,
) -> Result<CallToolResult, UnloadedMedia> {
    let content = output
        .iter()
        .map(|block| {
            Ok(match block {
                ToolResultContentBlock::Text { text } => ContentBlock::text(text.clone()),
                ToolResultContentBlock::Image { source } => match source {
                    ToolMediaSource::Binary { data, media_type } => {
                        ContentBlock::image(STANDARD.encode(data), media_type.clone())
                    }
                    ToolMediaSource::Ref { r#ref, .. } => {
                        return Err(UnloadedMedia(r#ref.to_string()));
                    }
                },
                ToolResultContentBlock::Video { source } => {
                    let (ToolMediaSource::Binary { media_type, .. }
                    | ToolMediaSource::Ref { media_type, .. }) = source;
                    ContentBlock::text(format!("[video:{media_type}]"))
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(if is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    })
}
