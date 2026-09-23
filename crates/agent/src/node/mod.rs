//! A node of a conversation's tree and its assembly (`subagents.md`
//! § Runtime): every node, the root and each subagent, is one kind of node
//! built by one assembly, which is the one place that creates a node's
//! session.

use std::{rc::Rc, sync::Arc};

use demi_core::{Clock, ModelSelection, NodeId};
use demi_gates::{ActivityGate, GateLease, Purpose};
use demi_provider::{ProviderRuntime, ToolDefinition};
use futures_util::future::LocalBoxFuture;

use crate::{
    AgentHarness, IdSource, PromptContext,
    session::{
        AgentSession, Continuation, RestoreError, SessionConfig, SessionDeps, SessionInit,
        SessionRuntime, ToolFailure, ToolInvocation, ToolOutcome,
    },
    store::{AgentTreeStore, NodeRecord, StoreError},
};

/// One live node: its record and its session.
pub struct Node<H: AgentHarness> {
    record: NodeRecord,
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
}

/// What the session calls in its node: the tree's admission, the harness
/// with the node's context and rendered command help, and the tools.
pub(crate) struct NodeRuntime<H> {
    node: NodeId,
    root: NodeId,
    cwd: String,
    harness: Rc<H>,
    /// The node's command help, rendered once.
    commands: String,
    admission: ActivityGate,
}

impl<H: AgentHarness> NodeRuntime<H> {
    fn prompt_context(&self) -> PromptContext<'_> {
        PromptContext {
            node: &self.node,
            root: &self.root,
            cwd: &self.cwd,
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

    fn system_prompt(&self) -> LocalBoxFuture<'_, String> {
        Box::pin(
            self.harness
                .system_prompt(self.prompt_context(), &self.commands),
        )
    }

    fn preamble(&self) -> LocalBoxFuture<'_, Option<String>> {
        Box::pin(self.harness.preamble(self.prompt_context()))
    }

    fn context(&self) -> LocalBoxFuture<'_, Option<String>> {
        Box::pin(self.harness.context(self.prompt_context()))
    }

    /// The standard tools arrive with the shell environments they run in;
    /// until then the model has none, and a call it makes anyway completes as
    /// `Tool not found`.
    fn tools(&self) -> Arc<[ToolDefinition]> {
        Arc::from([])
    }

    fn invoke_tool(
        &self,
        call: ToolInvocation,
    ) -> LocalBoxFuture<'_, Result<ToolOutcome, ToolFailure>> {
        let outcome = ToolOutcome::error(format!("Tool not found: {}", call.tool_name));
        Box::pin(async move { Ok(outcome) })
    }
}

/// What makes a node itself, and what it is built with.
pub(crate) struct NodeSpec<H> {
    /// The record a new node is created with; a stored node keeps its own.
    pub(crate) record: NodeRecord,
    pub(crate) root: NodeId,
    pub(crate) cwd: String,
    pub(crate) model: ModelSelection,
    pub(crate) runtime: Box<dyn ProviderRuntime>,
    pub(crate) harness: Rc<H>,
    pub(crate) store: Rc<dyn AgentTreeStore>,
    pub(crate) admission: ActivityGate,
    pub(crate) ids: Rc<dyn IdSource>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) config: SessionConfig,
}

/// A node, and what its restore handed back when it came from the store.
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
        root,
        cwd,
        model,
        runtime,
        harness,
        store,
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
    let node_runtime = Rc::new(NodeRuntime {
        node: record.id.clone(),
        root,
        cwd: cwd.clone(),
        commands: harness.commands().render_help(),
        harness: harness.clone(),
        admission,
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
            store
                .create_node(record.clone(), session.first_checkpoint())
                .await?;
            (session, None)
        }
    };
    Ok(Assembled {
        node: Node {
            record,
            session,
            runtime: node_runtime,
        },
        continuation,
    })
}
