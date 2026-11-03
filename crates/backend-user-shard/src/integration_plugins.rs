//! An integration's plugin: the commands and the context its program serves
//! over its socket (`crates/integration-api-protocol/src/lib.rs`).
//!
//! A socket that sends `declare` serves the integration's plugin from then on;
//! the declaration is kept, so the commands are its conversations' while the
//! program is away. Each command tree is a root of its own whose leaves are
//! `rpc` leaves, served by [`IntegrationCommand`], which sends the call to the
//! socket that declared last and relays the call's port messages both ways.
//!
//! ```text
//! agent: chatbot post ...  ->  IntegrationCommand  ->  request {id}  -> program
//!                               port.forward       <-  port {id, n}  <-
//!                               answer {id, n}     ->
//!                               exit code          <-  reply {id}    <-
//! ```
//!
//! A call while no socket serves the plugin waits for one as long as the
//! backend waits for a runner that dropped, then fails, telling the model the
//! integration is unreachable. A context request of an optional context does
//! not wait for a socket, and waits for its answer only so long: an agent then
//! asks its model without the block. One of a required context waits for a
//! socket as a call does and for its answer as long, and then fails the
//! agent's request.
//!
//! Each request names the turn its work is in and the program's message that
//! work descends from (`demi_backend_database::tree::turn_message`), which the
//! program finds its own data by: Demi keeps none of it.

use std::rc::{Rc, Weak};
use std::time::Duration;

use demi_agent_tools::{ContextAnswer, ContextFailure, ContextSource, NodeContext};
use demi_backend_database::integrations::IntegrationRecord;
use demi_backend_host_access::conversation_of;
use demi_command_declarations::{LeafKind, NativeOperation, Node};
use demi_host_interface::{Declared, RpcError, RpcHandler, RpcInvocation, RpcPort};
use demi_integration_api_protocol as api;
use demi_backend_database::tree;
use demi_backend_host_access::root_of;
use demi_shared_types::{NodeId, TurnId};
use demi_web_api_protocol::ids::{ConversationId, IntegrationId};
use futures_util::future::LocalBoxFuture;
use tokio::sync::oneshot;

use crate::shard::Shard;

/// The name of the integration's context source, whose blocks its program
/// writes.
pub(crate) const INTEGRATION_CONTEXT: &str = "integration";

/// How long a context request waits for its reply before the agent asks its
/// model without the block, or fails its request when it cannot go without,
/// unless the declaration names its own wait: a program that never answers
/// would otherwise hold the agent back for good.
const CONTEXT_WAIT: Duration = Duration::from_secs(10);

/// A plugin request that waits for its reply.
pub(crate) struct Call {
    pub(crate) integration: IntegrationId,
    /// The socket it went to.
    pub(crate) socket: u64,
    pub(crate) reply: oneshot::Sender<api::PluginReply>,
    /// A command's port, which the program's `port` messages reach.
    port: Option<RpcPort>,
}

impl Shard {
    /// What the integration `integration` declared last; none before its
    /// first declaration, or when it cannot be read.
    pub(crate) async fn declaration(&self, integration: &IntegrationId) -> Option<api::Declaration> {
        let text = self
            .services()
            .control
            .integration_declaration(integration.clone())
            .await
            .ok()??;
        serde_json::from_str(&text).ok()
    }

    /// The trees of `declaration` as a toolset holds them, each served by
    /// the integration.
    pub(crate) fn declared_trees(self: &Rc<Self>, integration: &IntegrationId, declaration: &api::Declaration) -> Vec<Declared> {
        let handler: Rc<dyn RpcHandler> = Rc::new(IntegrationCommand {
            shard: Rc::downgrade(self),
            integration: integration.clone(),
        });
        declaration
            .commands
            .iter()
            .map(|tree| Declared::served(tree.clone(), handler.clone()))
            .collect()
    }

    /// The id of the program's message the work of `node`'s turn `turn` in
    /// the conversation `conversation` descends from; none when it descends
    /// from none, or the conversation cannot be read.
    ///
    /// The node's rows are saved first: a turn's first block is saved a
    /// moment after it is written, and the turn's first request asks before
    /// then. The rows of the turns it descends from were saved before their
    /// calls ran.
    pub(crate) async fn turn_message(&self, conversation: &ConversationId, node: &NodeId, turn: &TurnId) -> Option<String> {
        if let Some(live) = self.agent().node(&root_of(conversation), node) {
            // A save that fails leaves the rows to the next: the turn's
            // first block may then be missing, and its message with it.
            let _ = live.session().flush().await;
        }
        let (node, turn) = (node.clone(), turn.clone());
        match self
            .services()
            .conversations
            .read(conversation, move |connection| tree::turn_message(connection, &node, &turn))
            .await
        {
            Ok(message) => message.flatten(),
            Err(error) => {
                tracing::warn!(
                    event = "turn_message.read_failed",
                    conversation = %conversation,
                    error = &error as &dyn std::error::Error,
                    "the message a turn descends from could not be read"
                );
                None
            }
        }
    }

    /// Takes a message of the integration's socket `socket`.
    pub async fn integration_message(self: &Rc<Self>, integration: &IntegrationRecord, socket: u64, message: api::ClientMessage) {
        let events = &self.integration_events;
        match message {
            api::ClientMessage::Declare { commands, context } => {
                let refused = self.declare(integration, commands, context).await.err();
                if refused.is_none() {
                    events.serve(&integration.id, socket);
                }
                events.send_to(&integration.id, socket, api::Event::Declared { refused });
            }
            api::ClientMessage::Reply { id, reply } => {
                let mut calls = events.calls.borrow_mut();
                // Only a socket of the integration whose call it is answers it.
                if calls.get(&id).is_some_and(|call| call.integration == integration.id)
                    && let Some(call) = calls.remove(&id)
                {
                    let _ = call.reply.send(reply);
                }
            }
            api::ClientMessage::Port { id, port, request } => {
                let held = events
                    .calls
                    .borrow()
                    .get(&id)
                    .filter(|call| call.integration == integration.id)
                    .and_then(|call| call.port.clone());
                let Some(rpc) = held else {
                    return;
                };
                let shard = self.clone();
                let integration = integration.id.clone();
                self.tasks().spawn_local(async move {
                    let response = match rpc.forward(request).await {
                        Ok(response) => response,
                        // The call ended meanwhile: its cancel tells the
                        // program.
                        Err(_) => return,
                    };
                    shard
                        .integration_events
                        .send_to(&integration, socket, api::Event::Answer { id, port, response });
                });
            }
        }
    }

    /// Keeps `commands` and `context` as the integration's plugin, once
    /// checked: each tree a root of its own that no command of Demi's takes,
    /// whose leaves are `rpc` leaves. Open trees take its commands at once.
    async fn declare(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        declaration: api::Declaration,
    ) -> Result<(), String> {
        let taken: Vec<String> = self
            .agent()
            .node_commands()
            .await?
            .declarations()
            .map(|node| node_name(node).to_owned())
            .collect();
        let mut names = Vec::new();
        for tree in &declaration.commands {
            let name = node_name(tree);
            if taken.iter().any(|taken| taken == name) || names.contains(&name) {
                return Err(format!("the command {name} is taken"));
            }
            names.push(name);
            if !only_rpc(tree) {
                return Err(format!("the command {name} has a leaf that is no rpc leaf"));
            }
        }
        // Every tree must be one a command set takes.
        let mut check = demi_host_interface::CommandSet::new();
        for tree in self.declared_trees(&integration.id, &declaration) {
            check.register(tree).map_err(|error| error.to_string())?;
        }
        let text = serde_json::to_string(&declaration).map_err(|error| error.to_string())?;
        self.services()
            .control
            .declare_integration(integration.id.clone(), text)
            .await
            .map_err(|error| error.to_string())?;
        self.agent().adopt_toolset().await?;
        Ok(())
    }

    /// Sends `request` to the socket that serves the integration's plugin and
    /// answers its reply; it waits `wait` for such a socket, and `answer` for
    /// the reply, without end for none. `port` is a command's, whose stop
    /// cancels the request.
    async fn request_plugin(
        self: &Rc<Self>,
        integration: &IntegrationId,
        request: api::PluginRequest,
        port: Option<RpcPort>,
        wait: Duration,
        answer: Option<Duration>,
    ) -> Result<api::PluginReply, String> {
        let events = &self.integration_events;
        let deadline = tokio::time::Instant::now() + wait;
        let socket = loop {
            let declared = events.declared.notified();
            if let Some(socket) = events.server(integration) {
                break socket;
            }
            if tokio::time::timeout_at(deadline, declared).await.is_err() {
                return Err("the integration is unreachable: no program of it is connected".to_owned());
            }
        };
        let id = events.next_call.get() + 1;
        events.next_call.set(id);
        let (reply, answered) = oneshot::channel();
        let cancel = port.as_ref().map(RpcPort::cancellation);
        events.calls.borrow_mut().insert(
            id,
            Call {
                integration: integration.clone(),
                socket,
                reply,
                port,
            },
        );
        // A request that ends unanswered, stopped, timed out or dropped,
        // tells the program, which ends its work on it.
        let _pending = Pending {
            shard: self,
            integration,
            socket,
            id,
        };
        if !events.send_to(integration, socket, api::Event::Request { id, request }) {
            return Err("the integration's program went away".to_owned());
        }
        let stopped = async {
            match &cancel {
                Some(cancel) => cancel.cancelled().await,
                None => std::future::pending().await,
            }
        };
        let late = async {
            match answer {
                Some(answer) => tokio::time::sleep(answer).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            answered = answered => answered.map_err(|_| "the integration's program went away".to_owned()),
            () = stopped => Err("the call was stopped".to_owned()),
            () = late => Err("the integration did not answer in time".to_owned()),
        }
    }
}

/// A plugin request sent: unless its reply took it, its end removes it and
/// sends its socket `cancel`.
struct Pending<'a> {
    shard: &'a Shard,
    integration: &'a IntegrationId,
    socket: u64,
    id: u64,
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        let events = &self.shard.integration_events;
        if events.calls.borrow_mut().remove(&self.id).is_some() {
            events.send_to(self.integration, self.socket, api::Event::Cancel { id: self.id });
        }
    }
}

/// A tree's own name.
fn node_name(node: &Node<NativeOperation>) -> &str {
    match node {
        Node::Group(group) => &group.name,
        Node::Leaf(leaf) => &leaf.name,
    }
}

/// Whether every leaf of `node` is an `rpc` leaf.
fn only_rpc(node: &Node<NativeOperation>) -> bool {
    match node {
        Node::Group(group) => group.subcommands.iter().all(only_rpc),
        Node::Leaf(leaf) => matches!(leaf.kind, LeafKind::Rpc),
    }
}

/// The handler of every leaf of an integration's commands.
struct IntegrationCommand {
    shard: Weak<Shard>,
    integration: IntegrationId,
}

impl RpcHandler for IntegrationCommand {
    fn call(&self, invocation: RpcInvocation, port: RpcPort) -> LocalBoxFuture<'_, Result<u8, RpcError>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| RpcError::Failed("the backend is shutting down".into()))?;
            let conversation = conversation_of_invocation(&invocation)?;
            let key = shard
                .services()
                .control
                .integration_conversation(conversation.clone())
                .await
                .map_err(|error| RpcError::Failed(error.to_string()))?
                .and_then(|kept| kept.key);
            let caller = invocation.caller.as_ref();
            let turn = caller.and_then(|caller| caller.turn.clone());
            let message_id = match (caller, &turn) {
                (Some(caller), Some(turn)) => shard.turn_message(&conversation, &caller.node, turn).await,
                _ => None,
            };
            let request = api::PluginRequest::Command {
                conversation,
                key,
                agent: caller.map(|caller| caller.node.clone()),
                agent_number: invocation.context.caller.agent_number(),
                turn: turn.map(|turn| turn.as_str().to_owned()),
                message_id,
                cloud: invocation.host,
                path: invocation.path,
                argv: invocation.argv,
                args: invocation.args,
                json: invocation.json,
                cwd: invocation.cwd,
                stdin: invocation.stdin,
            };
            // As long as a runner that dropped is awaited.
            let wait = shard.services().runners.returning;
            match shard.request_plugin(&self.integration, request, Some(port), wait, None).await {
                Ok(api::PluginReply::Exit { code }) => Ok(code),
                Ok(api::PluginReply::Failed { message }) => Err(RpcError::Failed(message)),
                Ok(api::PluginReply::Context { .. }) => {
                    Err(RpcError::Failed("the integration answered a command with a context block".into()))
                }
                Err(message) => Err(RpcError::Failed(message)),
            }
        })
    }
}

fn conversation_of_invocation(
    invocation: &RpcInvocation,
) -> Result<demi_web_api_protocol::ids::ConversationId, RpcError> {
    demi_web_api_protocol::ids::ConversationId::try_from(invocation.context.conversation.as_str())
        .map_err(|_| RpcError::Failed("this session has no conversation".into()))
}

/// The context source of an integration's plugin that declared itself one.
pub(crate) struct IntegrationContext {
    pub(crate) shard: Weak<Shard>,
}

impl ContextSource for IntegrationContext {
    fn name(&self) -> &str {
        INTEGRATION_CONTEXT
    }

    fn context<'a>(
        &'a self,
        node: NodeContext<'a>,
        turn: &'a TurnId,
        seen: &'a [&'a str],
    ) -> LocalBoxFuture<'a, Result<Option<ContextAnswer>, ContextFailure>> {
        Box::pin(async move {
            let Some(shard) = self.shard.upgrade() else {
                return Ok(None);
            };
            let conversation = conversation_of(node.root);
            let Some(kept) = shard
                .services()
                .control
                .integration_conversation(conversation.clone())
                .await
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            let Some(declaration) = shard.declaration(&kept.integration).await else {
                return Ok(None);
            };
            let mode = declaration.context;
            if mode == api::ContextMode::None {
                return Ok(None);
            }
            let answer_wait = declaration
                .context_wait_seconds
                .map_or(CONTEXT_WAIT, |seconds| Duration::from_secs(seconds.get().into()));
            let request = api::PluginRequest::Context {
                message_id: shard.turn_message(&conversation, node.node, turn).await,
                conversation,
                key: kept.key,
                agent: node.node.clone(),
                turn: turn.as_str().to_owned(),
                seen: seen.iter().map(|text| (*text).to_owned()).collect(),
            };
            if mode == api::ContextMode::Required {
                // A socket as long as a runner that dropped is awaited, as a
                // call waits for one.
                let wait = shard.services().runners.returning;
                return match shard.request_plugin(&kept.integration, request, None, wait, Some(answer_wait)).await {
                    Ok(api::PluginReply::Context { text }) => Ok(text.map(ContextAnswer::from)),
                    Ok(api::PluginReply::Failed { message }) => Err(ContextFailure::Required(message)),
                    Ok(api::PluginReply::Exit { .. }) => Err(ContextFailure::Required(
                        "the integration answered a context request with an exit".to_owned(),
                    )),
                    Err(message) => Err(ContextFailure::Required(message)),
                };
            }
            match shard.request_plugin(&kept.integration, request, None, Duration::ZERO, Some(answer_wait)).await {
                Ok(api::PluginReply::Context { text }) => Ok(text.map(ContextAnswer::from)),
                Ok(api::PluginReply::Failed { message }) => Err(ContextFailure::Skipped(message)),
                Ok(api::PluginReply::Exit { .. }) => Err(ContextFailure::Skipped(
                    "the integration answered a context request with an exit".to_owned(),
                )),
                // No program connected, or none answered in time: no block.
                Err(_) => Ok(None),
            }
        })
    }
}
