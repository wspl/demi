//! A node of a conversation's tree and its assembly (`subagents.md`
//! § Runtime): every node, the root and each subagent, is one kind of node
//! built by one assembly, which is the one place that creates a node's
//! session. What differs between nodes is configuration: the instructions,
//! the model, the commands, the spawn restriction, and the role that sets
//! the lifecycle policy.

use std::{
    rc::{Rc, Weak},
    sync::Arc,
};

use bytes::Bytes;

use demi_agent_session::{
    AgentSession, Continuation, NewContext, RestoreError, SeenContext, SessionConfig, SessionDeps,
    SessionInit, SessionRuntime, ToolFailure, ToolInvocation, ToolOutcome,
};
use demi_agent_store::{AgentTreeStore, Checkpoint, NodeRecord, StoreError};
use demi_agent_tools::{
    CallError, ContextSource, ConversationCommands, Environments, HostResolver, ModelIdentity,
    NodeContext, ShellAccess, ShellEnvironmentFactory, StoreNumbers, definitions, runs_together,
    stored_running_commands, system_prompt,
};
use demi_agent_transcript::IdSource;
use demi_host_interface::{
    CommandSet, Ending, JobCaller, Numbers, PageFeed, PageState, PageView, ShellEnvironment,
    ShellError, WholeOutput,
};
use demi_provider_common::{ProviderRuntime, ToolDefinition};
use demi_shared_gates::{ActivityGate, GateLease, Purpose, Reservation};
use demi_shared_types::{
    Clock, CommandEnd, CommandId, ModelSelection, NodeId, QueuedMessage, TurnId, WakeupCommand,
};
use futures_util::future::{LocalBoxFuture, select_all};

use crate::{
    AgentServer,
    server::{CommandPlace, ProviderResolver, Tree},
};

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

    /// The caller a job the node starts records: this node.
    pub fn job_caller(&self) -> JobCaller {
        JobCaller {
            node: self.record.id.clone(),
        }
    }

    /// Writes `stdin` to a running command through the node's environment
    /// for the conversation's current Host, as a page's `shell_write` asks.
    pub(crate) async fn shell_write(
        &self,
        command: &CommandId,
        stdin: String,
    ) -> Result<(), CallError> {
        self.runtime.shell_access().write(command, stdin).await
    }

    /// Stops a running command through the node's environment for the
    /// conversation's current Host, as a page's `shell_abort` asks.
    pub(crate) async fn shell_abort(&self, command: &CommandId) -> Result<(), CallError> {
        self.runtime.shell_access().abort(command).await
    }

    /// Whether one of the node's environments holds `command`.
    pub(crate) fn holds(&self, command: &CommandId) -> bool {
        self.runtime.environments.owning(command).is_some()
    }

    /// The node's environment that holds `command`.
    pub(crate) fn environment_of(&self, command: &CommandId) -> Option<Rc<dyn ShellEnvironment>> {
        self.runtime.environments.owning(command)
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

    /// Medium `number` of `command`, which one of the node's environments
    /// runs, as its Host keeps it.
    pub(crate) async fn read_medium(
        &self,
        command: &CommandId,
        number: u32,
    ) -> Result<Bytes, ShellError> {
        let environment = self
            .runtime
            .environments
            .owning(command)
            .ok_or_else(|| ShellError::UnknownCommand(command.clone()))?;
        environment.read_medium(command, number).await
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
    /// before the `demi agent` graft, which every node of the tree shares.
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
/// prompt, the preamble, the context sources, the tools, the window its
/// model is used with, and the hold on its children that an edit needs.
pub(crate) struct NodeRuntime<H: HostResolver> {
    node: NodeId,
    root: NodeId,
    /// Where the window in use for a model comes from.
    providers: Rc<dyn ProviderResolver>,
    cwd: String,
    hosts: Rc<H>,
    /// The instructions of its system prompt, its identity.
    instructions: Rc<str>,
    /// The product's harness guide.
    guide: Rc<str>,
    /// The capability index of its commands, rendered once.
    index: String,
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
    /// The server, through which the node finds its tree's commands.
    server: Weak<AgentServer<H>>,
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
            conversation: self,
        }
    }

    /// The node's tree, while it is live.
    fn tree(&self) -> Option<Rc<Tree<H>>> {
        self.server.upgrade()?.tree(&self.root)
    }
}

impl<H: HostResolver> ConversationCommands for NodeRuntime<H> {
    fn knows<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<bool, String>> {
        Box::pin(async move {
            let tree = self
                .tree()
                .ok_or_else(|| format!("the conversation {} is not open", self.root))?;
            let place = tree.command_place(command).await?;
            Ok(!matches!(place, CommandPlace::Unknown))
        })
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

    /// The layers rendered at assembly, and the line naming `model` with
    /// the family of the entry that serves it.
    fn system_prompt<'a>(
        &'a self,
        model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let family = self
                .providers
                .family(model)
                .await
                .map_err(|error| error.to_string())?;
            let identity = ModelIdentity {
                name: &model.model.name,
                family: &family,
                id: &model.model.id,
            };
            Ok(system_prompt(
                &self.instructions,
                &self.guide,
                &self.index,
                identity,
            ))
        })
    }

    fn context_window<'a>(&'a self, model: &'a ModelSelection) -> LocalBoxFuture<'a, u32> {
        self.providers.context_window(model)
    }

    fn preamble(&self) -> LocalBoxFuture<'_, Option<String>> {
        let text = self.preamble.clone();
        Box::pin(async move { text })
    }

    /// Asks every source at once, each with its own blocks, and takes
    /// their answers in the sources' order: a source that reads a Host does
    /// not make the others wait. A source that fails adds nothing: it is
    /// asked again before the next request.
    fn context<'a>(
        &'a self,
        seen: &'a [SeenContext<'a>],
        turn: &'a TurnId,
    ) -> LocalBoxFuture<'a, Vec<NewContext>> {
        Box::pin(async move {
            let asked = self.context.iter().map(|source| async move {
                let name = source.name();
                let own: Vec<&str> = seen
                    .iter()
                    .filter(|seen| seen.source == name)
                    .map(|seen| seen.text)
                    .collect();
                (name, source.context(self.node_context(), turn, &own).await)
            });
            let mut news = Vec::new();
            for (name, answer) in futures_util::future::join_all(asked).await {
                match answer {
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

    fn runs_together(&self, tool: &str) -> bool {
        runs_together(tool)
    }

    fn invoke_step(
        &self,
        calls: Vec<ToolInvocation>,
    ) -> LocalBoxFuture<'_, Vec<Result<ToolOutcome, ToolFailure>>> {
        Box::pin(async move { self.shell_access().invoke_step(calls).await })
    }

    /// Finds each command in the tree, whichever node's shells hold it, and
    /// then holds only the environments it waits on, never the tree. A
    /// command whose result was given already, so that only the store
    /// knows it, has ended without a known end.
    fn command_end(&self, commands: Vec<CommandId>) -> LocalBoxFuture<'static, WakeupCommand> {
        let tree = self.tree();
        Box::pin(async move {
            let Some(tree) = tree else {
                // A tree that is not live runs no command; it is closing.
                return std::future::pending().await;
            };
            let mut waits = Vec::new();
            for command in commands {
                let environment = match tree.command_place(&command).await {
                    Ok(CommandPlace::Held(node)) => node.environment_of(&command),
                    Ok(CommandPlace::Stored | CommandPlace::Unknown) => None,
                    Err(error) => {
                        // The time still wakes the node.
                        tracing::warn!(%command, %error, "a command a yield waits for could not be found");
                        continue;
                    }
                };
                let Some(environment) = environment else {
                    return WakeupCommand {
                        command_id: command,
                        end: CommandEnd::Ended,
                    };
                };
                waits.push(Box::pin(async move {
                    let end = match environment.ended(&command).await {
                        Ok(Ending::Exited(exit_code)) => CommandEnd::Exited { exit_code },
                        Ok(Ending::Aborted) => CommandEnd::Stopped,
                        // Released after its end was given.
                        Err(_) => CommandEnd::Ended,
                    };
                    WakeupCommand {
                        command_id: command,
                        end,
                    }
                }));
            }
            drop(tree);
            if waits.is_empty() {
                return std::future::pending().await;
            }
            select_all(waits).await.0
        })
    }

    /// Ends the node's shells on every Host, their running commands with
    /// them.
    fn dispose(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(self.environments.dispose())
    }
}

/// Where a node comes from: created now, or restored from what the tree
/// store holds of it.
pub(crate) enum Origin {
    New {
        record: NodeRecord,
        cwd: String,
        model: ModelSelection,
        /// Its first message, queued in its create commit, so that a node
        /// the process loses before its first save still has it.
        first_message: Option<QueuedMessage>,
    },
    Stored {
        record: NodeRecord,
        checkpoint: Checkpoint,
    },
}

impl Origin {
    /// What the tree store holds of the node `id`; none when it holds no
    /// such node.
    pub(crate) async fn stored(
        store: &dyn AgentTreeStore,
        id: &NodeId,
    ) -> Result<Option<Self>, StoreError> {
        let Some(record) = store.node(id).await? else {
            return Ok(None);
        };
        let checkpoint = store
            .session_store(id)
            .load()
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("node {id} has no checkpoint")))?;
        Ok(Some(Self::Stored { record, checkpoint }))
    }

    pub(crate) fn record(&self) -> &NodeRecord {
        match self {
            Self::New { record, .. } | Self::Stored { record, .. } => record,
        }
    }

    /// The model selection the node infers with: a new node's, or the one
    /// its checkpoint saved.
    pub(crate) fn model(&self) -> &ModelSelection {
        match self {
            Self::New { model, .. } => model,
            Self::Stored { checkpoint, .. } => &checkpoint.state.model,
        }
    }
}

/// What makes a node itself, and what it is built with.
pub(crate) struct NodeSpec<H: HostResolver> {
    pub(crate) origin: Origin,
    pub(crate) role: NodeRole,
    pub(crate) root: NodeId,
    /// A runtime that serves the origin's model selection's provider entry.
    pub(crate) runtime: Box<dyn ProviderRuntime>,
    pub(crate) providers: Rc<dyn ProviderResolver>,
    pub(crate) hosts: Rc<H>,
    pub(crate) instructions: Rc<str>,
    /// The product's harness guide, which every node carries.
    pub(crate) guide: Rc<str>,
    /// A child's identity, the text before each of its user turns.
    pub(crate) preamble: Option<String>,
    pub(crate) context: Rc<[Rc<dyn ContextSource>]>,
    /// The product's commands before the `demi agent` graft: what the
    /// node's own children inherit.
    pub(crate) inherited: Rc<CommandSet>,
    /// `inherited` with the node's `demi agent` group grafted.
    pub(crate) commands: Rc<CommandSet>,
    pub(crate) store: Rc<dyn AgentTreeStore>,
    pub(crate) shells: Rc<dyn ShellEnvironmentFactory<H::Host>>,
    /// Where the node's environments tell the pages of its commands: the
    /// node's feed of its tree.
    pub(crate) feed: Rc<dyn PageFeed>,
    pub(crate) admission: ActivityGate,
    pub(crate) ids: Rc<dyn IdSource>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) config: SessionConfig,
    /// The server, through which the node finds its tree's commands.
    pub(crate) server: Weak<AgentServer<H>>,
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

/// The one way a node comes to exist: restored from what the tree store
/// holds of it, or created, its record and first checkpoint in one commit.
/// When assembly fails, the provider runtime it was given served no run,
/// and dropping it releases what it holds, the provider contract's fallback
/// for a runtime that is not closed.
pub(crate) async fn assemble<H: HostResolver>(
    spec: NodeSpec<H>,
) -> Result<Assembled<H>, AssembleError> {
    let NodeSpec {
        origin,
        role,
        root,
        runtime,
        providers,
        hosts,
        instructions,
        guide,
        preamble,
        context,
        inherited,
        commands,
        store,
        shells,
        feed,
        admission,
        ids,
        clock,
        config,
        server,
    } = spec;
    let record = origin.record().clone();
    let session_store = store.session_store(&record.id);
    let cwd = match &origin {
        Origin::New { cwd, .. } => cwd.clone(),
        Origin::Stored { checkpoint, .. } => checkpoint.state.cwd.clone(),
    };
    let index = commands.render_index();
    let node_runtime = Rc::new(NodeRuntime {
        node: record.id.clone(),
        root,
        providers,
        cwd: cwd.clone(),
        hosts,
        instructions,
        guide,
        index,
        preamble,
        context,
        inherited,
        commands,
        admission,
        lifecycle: ActivityGate::new(),
        numbers: Rc::new(StoreNumbers(store.clone())),
        agent: record.number,
        server,
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
    let (session, continuation) = match origin {
        Origin::Stored { checkpoint, .. } => {
            let (session, continuation) =
                AgentSession::restore(checkpoint, record.id.clone(), runtime, deps)?;
            (session, Some(continuation))
        }
        Origin::New {
            model,
            first_message,
            ..
        } => {
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
