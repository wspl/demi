//! A node of a conversation's tree and its assembly (`subagents.md`
//! § Runtime): every node, the root and each subagent, is one kind of node
//! built by one assembly, which is the one place that creates a node's
//! session. What differs between nodes is configuration: the instructions,
//! the model, the commands, the spawn restriction, and the role that sets
//! the lifecycle policy.

use std::{rc::Rc, sync::Arc};

use demi_agent_session::{
    AgentSession, Continuation, NewContext, RestoreError, SeenContext, SessionConfig, SessionDeps,
    SessionInit, SessionRuntime, ToolFailure, ToolInvocation, ToolOutcome,
};
use demi_agent_store::{AgentTreeStore, NodeRecord, StoreError};
use demi_agent_tools::{
    CallError, ContextSource, Environments, HostResolver, NodeContext, ShellAccess,
    ShellEnvironmentFactory, StoreNumbers, definitions, stored_running_commands, system_prompt,
};
use demi_agent_transcript::IdSource;
use demi_host_interface::{
    CommandSet, JobCaller, Numbers, PageFeed, PageState, PageView, ShellError, WholeOutput,
};
use demi_provider_common::{ProviderRuntime, ToolDefinition};
use demi_shared_gates::{ActivityGate, GateLease, Purpose, Reservation};
use demi_shared_types::{Clock, CommandId, ModelSelection, NodeId, QueuedMessage, TurnId};
use futures_util::future::LocalBoxFuture;

/// A node's place in its tree, which sets its lifecycle policy: a child
/// resumes a turn the process interrupted and closes once it is quiescent;
/// the root leaves both to its client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeRole {
    Root,
    Child,
}

/// One live node: its record, its role and its session.
pub struct Node<H: HostResolver> {
    record: NodeRecord,
    role: NodeRole,
    session: AgentSession,
    runtime: Rc<NodeRuntime<H>>,
}

impl<H: HostResolver> Node<H> {
    pub fn id(&self) -> &NodeId {
        &self.record.id
    }

    pub fn record(&self) -> &NodeRecord {
        &self.record
    }

    pub fn session(&self) -> &AgentSession {
        &self.session
    }

    /// The node's working directory.
    pub fn cwd(&self) -> &str {
        &self.runtime.cwd
    }

    /// The commands the node's shell offers: the product's, with the
    /// `demi agent` group grafted. The backend's command router dispatches
    /// the node's `rpc` calls through them.
    pub fn commands(&self) -> &Rc<CommandSet> {
        &self.runtime.commands
    }

    /// Whose command storage a job the node starts now reaches: this node,
    /// at its current generation.
    pub fn job_caller(&self) -> JobCaller {
        JobCaller {
            node: self.record.id.clone(),
            generation: self.session.generation_number(),
        }
    }

    /// Writes `stdin` to a running command through the node's environment
    /// for the conversation's current Host, as a page's `shell_write` asks.
    pub(crate) async fn shell_write(
        &self,
        command: &CommandId,
        stdin: String,
    ) -> Result<(), CallError> {
        self.runtime.shell_access().write(command, stdin).await?;
        Ok(())
    }

    /// Stops a running command through the node's environment for the
    /// conversation's current Host, as a page's `shell_abort` asks.
    pub(crate) async fn shell_abort(&self, command: &CommandId) -> Result<(), CallError> {
        self.runtime.shell_access().abort(command).await?;
        Ok(())
    }

    /// Whether one of the node's environments holds `command`.
    pub(crate) fn holds(&self, command: &CommandId) -> bool {
        self.runtime.environments.owning(command).is_some()
    }

    /// What the Host of `command`, which one of the node's environments
    /// runs, has kept of its output so far.
    pub(crate) async fn read_output(&self, command: &CommandId) -> Result<WholeOutput, ShellError> {
        let environment = self
            .runtime
            .environments
            .owning(command)
            .ok_or_else(|| ShellError::UnknownCommand(command.clone()))?;
        environment.read_output(command).await
    }

    /// The pages' view of each live command of the node (`runtime.md`
    /// § Live output): each one its shells run, and each one the transcript
    /// last saw running that they still hold, oldest first.
    pub(crate) fn live_views(&self) -> Vec<PageView> {
        let stored = stored_running_commands(&self.session.transcript().blocks);
        let mut views: Vec<PageView> = self
            .runtime
            .environments
            .page_views()
            .into_iter()
            .filter(|view| view.state == PageState::Running || stored.contains(&view.command_id))
            .collect();
        views.sort_by_key(|view| std::cmp::Reverse(view.running_ms));
        views
    }

    /// Ends the node's shells on every Host, their running commands with
    /// them; a later tool call makes fresh ones.
    pub(crate) async fn end_shells(&self) {
        self.runtime.environments.end_all().await;
    }

    /// The product's commands a child of this node inherits: this node's,
    /// before the `demi agent` graft.
    pub(crate) fn inherited_commands(&self) -> &Rc<CommandSet> {
        &self.runtime.inherited
    }

    /// The instructions of the node's system prompt, which a child without
    /// a profile's inherits.
    pub(crate) fn instructions(&self) -> &Rc<str> {
        &self.runtime.instructions
    }

    /// Held by each start and close of the node's children, and reserved by
    /// an edit of its transcript.
    pub(crate) fn lifecycle(&self) -> &ActivityGate {
        &self.runtime.lifecycle
    }

    /// Runs what a restored or new node has yet to run, under its role's
    /// policy (`subagents.md` § Persistence): a child resumes the turn the
    /// process interrupted, while the root records the interruption and
    /// leaves the turn to its client; the queued messages run again in
    /// order; waiting input and due wakeups wake the node only when its last
    /// turn was not interrupted. What this changed is saved at once.
    pub(crate) async fn continue_from(&self, continuation: Continuation) -> Result<(), StoreError> {
        let session = &self.session;
        if continuation.interrupted {
            match self.role {
                NodeRole::Root => session.record_interruption(),
                // The action reports its course as events, and a refusal
                // means the session is closing.
                NodeRole::Child => drop(session.resume()),
            }
        }
        for message in continuation.queued {
            // A restored session is live and has run nothing, so it admits
            // them; the actions report their course as events.
            drop(session.send(message.content, message.id));
        }
        if !continuation.interrupted {
            session.wake();
        }
        session.flush().await
    }
}

/// What the session calls in its node: the tree's admission, the system
/// prompt rendered at assembly, the preamble, the context sources, the
/// tools, and the hold on its children that an edit needs.
pub(crate) struct NodeRuntime<H: HostResolver> {
    node: NodeId,
    root: NodeId,
    cwd: String,
    hosts: Rc<H>,
    /// The instructions of its system prompt.
    instructions: Rc<str>,
    /// Its system prompt, rendered once.
    system_prompt: String,
    /// A child's identity, the text before each of its user turns.
    preamble: Option<String>,
    /// The product's context sources, in their order.
    context: Rc<[Rc<dyn ContextSource>]>,
    inherited: Rc<CommandSet>,
    commands: Rc<CommandSet>,
    admission: ActivityGate,
    lifecycle: ActivityGate,
    store: Rc<dyn AgentTreeStore>,
    shells: Rc<dyn ShellEnvironmentFactory<H::Host>>,
    /// The node's shell environment on each Host its tools used.
    environments: Environments,
    /// Where its environments tell the pages of its commands.
    feed: Rc<dyn PageFeed>,
    /// The conversation's command and shell numbers, from the tree store.
    numbers: Rc<dyn Numbers>,
    /// The node's agent number.
    agent: u64,
}

impl<H: HostResolver> NodeRuntime<H> {
    fn node_context(&self) -> NodeContext<'_> {
        NodeContext {
            node: &self.node,
            root: &self.root,
            cwd: &self.cwd,
        }
    }

    /// What the standard tools reach the node's shells through.
    fn shell_access(&self) -> ShellAccess<'_, H> {
        ShellAccess {
            hosts: &self.hosts,
            shells: self.shells.as_ref(),
            environments: &self.environments,
            context: self.node_context(),
            agent: self.agent,
            commands: &self.commands,
            feed: &self.feed,
            numbers: &self.numbers,
        }
    }
}

impl<H: HostResolver> SessionRuntime for NodeRuntime<H> {
    fn enter_action(&self) -> LocalBoxFuture<'_, GateLease> {
        Box::pin(self.admission.enter(Purpose::Demand))
    }

    fn reserve_edit(&self) -> LocalBoxFuture<'_, Result<Option<Reservation>, String>> {
        Box::pin(async move {
            let reservation = self.lifecycle.try_reserve().ok_or_else(|| {
                "Cannot edit while a child lifecycle operation is in progress".to_owned()
            })?;
            let children = self
                .store
                .children(&self.node)
                .await
                .map_err(|error| error.to_string())?;
            if children
                .iter()
                .any(|child| child.closed.is_none() || !child.delivered)
            {
                return Err(
                    "Cannot edit while children or completion notifications are pending".to_owned(),
                );
            }
            Ok(Some(reservation))
        })
    }

    fn system_prompt(&self) -> LocalBoxFuture<'_, String> {
        let text = self.system_prompt.clone();
        Box::pin(async move { text })
    }

    fn preamble(&self) -> LocalBoxFuture<'_, Option<String>> {
        let text = self.preamble.clone();
        Box::pin(async move { text })
    }

    /// Asks each source in turn with its own blocks. A source that fails
    /// adds nothing: it is asked again before the next request.
    fn context<'a>(
        &'a self,
        seen: &'a [SeenContext<'a>],
        turn: &'a TurnId,
    ) -> LocalBoxFuture<'a, Vec<NewContext>> {
        Box::pin(async move {
            let mut news = Vec::new();
            for source in self.context.iter() {
                let name = source.name();
                let own: Vec<&str> = seen
                    .iter()
                    .filter(|seen| seen.source == name)
                    .map(|seen| seen.text)
                    .collect();
                match source.context(self.node_context(), turn, &own).await {
                    Ok(Some(text)) => news.push(NewContext {
                        source: name.to_owned(),
                        text,
                    }),
                    Ok(None) => {}
                    Err(error) => tracing::warn!(
                        node = %self.node,
                        source = name,
                        %error,
                        "a context source failed; it is asked again before the next request"
                    ),
                }
            }
            news
        })
    }

    /// The standard tools, and only these.
    fn tools(&self) -> Arc<[ToolDefinition]> {
        definitions()
    }

    fn invoke_tool(
        &self,
        call: ToolInvocation,
    ) -> LocalBoxFuture<'_, Result<ToolOutcome, ToolFailure>> {
        Box::pin(async move { self.shell_access().invoke(call).await })
    }

    /// Ends the node's shells on every Host, their running commands with
    /// them.
    fn dispose(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(self.environments.dispose())
    }
}

/// What makes a node itself, and what it is built with.
pub(crate) struct NodeSpec<H: HostResolver> {
    /// The record a new node is created with; a stored node keeps its own.
    pub(crate) record: NodeRecord,
    pub(crate) role: NodeRole,
    pub(crate) root: NodeId,
    pub(crate) cwd: String,
    pub(crate) model: ModelSelection,
    pub(crate) runtime: Box<dyn ProviderRuntime>,
    pub(crate) hosts: Rc<H>,
    pub(crate) instructions: Rc<str>,
    /// A child's identity, the text before each of its user turns.
    pub(crate) preamble: Option<String>,
    pub(crate) context: Rc<[Rc<dyn ContextSource>]>,
    /// The product's commands, narrowed by a child's profile, before the
    /// `demi agent` graft: what the node's own children inherit.
    pub(crate) inherited: Rc<CommandSet>,
    /// `inherited` with the node's `demi agent` group grafted.
    pub(crate) commands: Rc<CommandSet>,
    /// A new node's first message, queued in its create commit, so that a
    /// node the process loses before its first save still has it.
    pub(crate) first_message: Option<QueuedMessage>,
    pub(crate) store: Rc<dyn AgentTreeStore>,
    pub(crate) shells: Rc<dyn ShellEnvironmentFactory<H::Host>>,
    /// Where the node's environments tell the pages of its commands: the
    /// node's feed of its tree.
    pub(crate) feed: Rc<dyn PageFeed>,
    pub(crate) admission: ActivityGate,
    pub(crate) ids: Rc<dyn IdSource>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) config: SessionConfig,
}

/// A node, and what it has yet to run: what its restore handed back, or a new
/// node's first message.
pub(crate) struct Assembled<H: HostResolver> {
    pub(crate) node: Node<H>,
    pub(crate) continuation: Option<Continuation>,
}

/// Why a node could not be assembled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum AssembleError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Restore(#[from] RestoreError),
}

/// The one way a node comes to exist: restored from the tree store when the
/// store holds it, otherwise created, its record and first checkpoint in one
/// commit. When assembly fails, the provider runtime it was given served no
/// run, and dropping it releases what it holds, the provider contract's
/// fallback for a runtime that is not closed.
pub(crate) async fn assemble<H: HostResolver>(
    spec: NodeSpec<H>,
) -> Result<Assembled<H>, AssembleError> {
    let NodeSpec {
        record,
        role,
        root,
        cwd,
        model,
        runtime,
        hosts,
        instructions,
        preamble,
        context,
        inherited,
        commands,
        first_message,
        store,
        shells,
        feed,
        admission,
        ids,
        clock,
        config,
    } = spec;
    let stored = store.node(&record.id).await?;
    let session_store = store.session_store(&record.id);
    let checkpoint = match &stored {
        Some(_) => {
            let checkpoint = session_store.load().await?;
            Some(checkpoint.ok_or_else(|| {
                StoreError::Corrupt(format!("node {} has no checkpoint", record.id))
            })?)
        }
        None => None,
    };
    let record = stored.unwrap_or(record);
    let cwd = checkpoint
        .as_ref()
        .map_or(cwd, |checkpoint| checkpoint.state.cwd.clone());
    let system_prompt = system_prompt(&instructions, &commands.render_help());
    let node_runtime = Rc::new(NodeRuntime {
        node: record.id.clone(),
        root,
        cwd: cwd.clone(),
        hosts,
        instructions,
        system_prompt,
        preamble,
        context,
        inherited,
        commands,
        admission,
        lifecycle: ActivityGate::new(),
        numbers: Rc::new(StoreNumbers(store.clone())),
        agent: record.number,
        store: store.clone(),
        shells,
        environments: Environments::default(),
        feed,
    });
    let deps = SessionDeps {
        runtime: node_runtime.clone(),
        store: session_store,
        ids,
        clock,
        config,
    };
    let (session, continuation) = match checkpoint {
        Some(checkpoint) => {
            let (session, continuation) =
                AgentSession::restore(checkpoint, record.id.clone(), runtime, deps)?;
            (session, Some(continuation))
        }
        None => {
            let init = SessionInit {
                id: record.id.clone(),
                cwd,
                model,
                runtime,
            };
            let session = AgentSession::create(init, deps);
            let mut initial = session.first_checkpoint();
            initial.state.queue.extend(first_message.clone());
            store.create_node(record.clone(), initial).await?;
            let continuation = first_message.map(|message| Continuation {
                interrupted: false,
                queued: vec![message],
            });
            (session, continuation)
        }
    };
    Ok(Assembled {
        node: Node {
            record,
            role,
            session,
            runtime: node_runtime,
        },
        continuation,
    })
}
