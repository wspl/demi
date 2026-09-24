//! A node of a conversation's tree and its assembly (`subagents.md`
//! § Runtime): every node, the root and each subagent, is one kind of node
//! built by one assembly, which is the one place that creates a node's
//! session. What differs between nodes is configuration: the prompt, the
//! model, the commands, the spawn restriction, and the role that sets the
//! lifecycle policy.

use std::{rc::Rc, sync::Arc};

use demi_core::{Clock, ModelSelection, NodeId, QueuedMessage};
use demi_gates::{ActivityGate, GateLease, Purpose, Reservation};
use demi_provider::{ProviderRuntime, ToolDefinition};
use demi_shell::{CommandSet, JobCaller, PortError, StorageOp, StorageReply};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::{
    AgentHarness, IdSource, PromptContext, ShellEnvironmentFactory,
    session::{
        AgentSession, Continuation, RestoreError, SessionConfig, SessionDeps, SessionInit,
        SessionRuntime, ToolFailure, ToolInvocation, ToolOutcome,
    },
    store::{AgentTreeStore, NodeRecord, StoreError},
    tools::{self, Environments, ShellAccess},
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
pub struct Node<H: AgentHarness> {
    record: NodeRecord,
    role: NodeRole,
    session: AgentSession,
    runtime: Rc<NodeRuntime<H>>,
}

impl<H: AgentHarness> Node<H> {
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

    /// The commands the node's shell offers: its harness's, with the
    /// `demi agent` group grafted. The backend's command router dispatches
    /// the node's `rpc` calls through them.
    pub fn commands(&self) -> &Rc<CommandSet> {
        &self.runtime.commands
    }

    /// The command-storage generation current now, which a job the node
    /// starts now is bound to (`command-state-history.md` § Mutation API and
    /// concurrency).
    pub fn command_generation(&self) -> CancellationToken {
        self.session.command_generation()
    }

    /// Whose command storage a job the node starts now reaches: this node,
    /// at its current generation.
    pub fn job_caller(&self) -> JobCaller {
        JobCaller {
            node: self.record.id.clone(),
            generation: self.session.generation_number(),
        }
    }

    /// Serves one command-storage message of a job of this node, bound to
    /// `lifetimes`: the job's generation and its call's cancellation. A write
    /// returns once its version is committed.
    pub async fn storage(
        &self,
        op: StorageOp,
        lifetimes: Vec<CancellationToken>,
    ) -> Result<StorageReply, PortError> {
        self.session.storage(op, lifetimes).await
    }

    /// The harness commands a child of this node inherits: this node's,
    /// before the `demi agent` graft.
    pub(crate) fn inherited_commands(&self) -> &Rc<CommandSet> {
        &self.runtime.inherited
    }

    /// What the node speaks with, which a child without a profile prompt
    /// inherits.
    pub(crate) fn prompt(&self) -> &Prompt {
        &self.runtime.prompt
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

/// What a node speaks with.
#[derive(Clone)]
pub(crate) enum Prompt {
    /// The harness's system prompt and preamble.
    Harness,
    /// A profile's system prompt, which drops the harness's preamble.
    Profile(Rc<dyn Fn(PromptContext<'_>, &str) -> String>),
}

/// What the session calls in its node: the tree's admission, the prompts
/// with the node's context and rendered command help, the tools, and the
/// hold on its children that an edit needs.
pub(crate) struct NodeRuntime<H: AgentHarness> {
    node: NodeId,
    root: NodeId,
    cwd: String,
    harness: Rc<H>,
    prompt: Prompt,
    /// A child's identity, after the preamble it inherits.
    preamble_suffix: Option<String>,
    inherited: Rc<CommandSet>,
    commands: Rc<CommandSet>,
    /// The commands' help, rendered once.
    help: String,
    admission: ActivityGate,
    lifecycle: ActivityGate,
    store: Rc<dyn AgentTreeStore>,
    shells: Rc<dyn ShellEnvironmentFactory<H::Host>>,
    /// The node's shell environment on each Host its tools used.
    environments: Environments,
}

impl<H: AgentHarness> NodeRuntime<H> {
    fn prompt_context(&self) -> PromptContext<'_> {
        PromptContext {
            node: &self.node,
            root: &self.root,
            cwd: &self.cwd,
        }
    }

    /// What the standard tools reach the node's shells through.
    fn shell_access(&self) -> ShellAccess<'_, H> {
        ShellAccess {
            harness: &self.harness,
            shells: self.shells.as_ref(),
            environments: &self.environments,
            context: self.prompt_context(),
            commands: &self.commands,
        }
    }
}

impl<H: AgentHarness> SessionRuntime for NodeRuntime<H> {
    fn harness_name(&self) -> &str {
        self.harness.name()
    }

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
        match &self.prompt {
            Prompt::Harness => Box::pin(
                self.harness
                    .system_prompt(self.prompt_context(), &self.help),
            ),
            Prompt::Profile(prompt) => {
                let text = prompt(self.prompt_context(), &self.help);
                Box::pin(async move { text })
            }
        }
    }

    fn preamble(&self) -> LocalBoxFuture<'_, Option<String>> {
        Box::pin(async move {
            let harness = match &self.prompt {
                Prompt::Harness => self.harness.preamble(self.prompt_context()).await,
                Prompt::Profile(_) => None,
            };
            match (harness, &self.preamble_suffix) {
                (Some(harness), Some(suffix)) => Some(format!("{harness}\n\n{suffix}")),
                (harness, None) => harness,
                (None, Some(suffix)) => Some(suffix.clone()),
            }
        })
    }

    fn context(&self) -> LocalBoxFuture<'_, Option<String>> {
        Box::pin(self.harness.context(self.prompt_context()))
    }

    /// The standard tools, and only these.
    fn tools(&self) -> Arc<[ToolDefinition]> {
        tools::definitions()
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
pub(crate) struct NodeSpec<H: AgentHarness> {
    /// The record a new node is created with; a stored node keeps its own.
    pub(crate) record: NodeRecord,
    pub(crate) role: NodeRole,
    pub(crate) root: NodeId,
    pub(crate) cwd: String,
    pub(crate) model: ModelSelection,
    pub(crate) runtime: Box<dyn ProviderRuntime>,
    pub(crate) harness: Rc<H>,
    pub(crate) prompt: Prompt,
    pub(crate) preamble_suffix: Option<String>,
    /// The harness commands, narrowed by a child's profile, before the
    /// `demi agent` graft: what the node's own children inherit.
    pub(crate) inherited: Rc<CommandSet>,
    /// `inherited` with the node's `demi agent` group grafted.
    pub(crate) commands: Rc<CommandSet>,
    /// A new node's first message, queued in its create commit, so that a
    /// node the process loses before its first save still has it.
    pub(crate) first_message: Option<QueuedMessage>,
    pub(crate) store: Rc<dyn AgentTreeStore>,
    pub(crate) shells: Rc<dyn ShellEnvironmentFactory<H::Host>>,
    pub(crate) admission: ActivityGate,
    pub(crate) ids: Rc<dyn IdSource>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) config: SessionConfig,
}

/// A node, and what it has yet to run: what its restore handed back, or a new
/// node's first message.
pub(crate) struct Assembled<H: AgentHarness> {
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
pub(crate) async fn assemble<H: AgentHarness>(
    spec: NodeSpec<H>,
) -> Result<Assembled<H>, AssembleError> {
    let NodeSpec {
        record,
        role,
        root,
        cwd,
        model,
        runtime,
        harness,
        prompt,
        preamble_suffix,
        inherited,
        commands,
        first_message,
        store,
        shells,
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
    let help = commands.render_help();
    let node_runtime = Rc::new(NodeRuntime {
        node: record.id.clone(),
        root,
        cwd: cwd.clone(),
        harness,
        prompt,
        preamble_suffix,
        inherited,
        commands,
        help,
        admission,
        lifecycle: ActivityGate::new(),
        store: store.clone(),
        shells,
        environments: Environments::default(),
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
