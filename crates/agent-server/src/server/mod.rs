//! The agent server of one user shard (`runtime.md` § Frame protocol,
//! § Connections and the live tree): it holds each open conversation's live
//! tree and hands out the connections that attach conversation sockets to
//! them. The backend owns the sockets, decodes each client frame, and passes
//! it to its connection; the connection's outbox carries every server frame
//! back.

mod commands;
mod connection;
mod content;
mod shell_output;
mod tree;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use demi_agent_session::{
    AgentMessageError, Continuation, ForkError, ModelSwitch, SessionConfig, fork_seed,
};
use demi_agent_store::{AgentTreeStore, Checkpoint, CheckpointUpdate, NodeRecord, StoreError};
use demi_agent_tools::{
    ContextSource, HostResolver, ProfileModel, ShellEnvironmentFactory, SubagentSource, ToolsetSource,
    Unavailable,
};
use demi_agent_transcript::IdSource;

use demi_provider_common::ProviderRuntime;
use demi_shared_gates::KeyedSerialGate;
use demi_shared_types::{AgentMessage, BlockId, Clock, ModelSelection, NodeId, SessionPhase};
use futures_util::future::{LocalBoxFuture, join_all};
use tokio_util::task::TaskTracker;

pub use connection::{Connection, FrameRx, Outgoing};
pub use content::{ContentError, ContentResolver, FileReference, ResolvedFiles};
pub use tree::Tree;

use tree::OpenError;

use crate::Node;

/// Where the agent gets a conversation's model selection and the provider
/// runtimes its sessions infer with. The backend holds each conversation's
/// selection in its record, resolves the provider entry a selection names
/// and builds the runtime on the shard that will own it; the agent never sees
/// the provider itself.
pub trait ProviderResolver {
    /// The model selection the conversation `root`'s tree opens with: the
    /// one its record holds (`runtime.md` § Connections and the live tree).
    fn selection<'a>(
        &'a self,
        root: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<ModelSelection, ResolveError>>;

    /// A runtime for a session of the conversation `root` that infers with
    /// `model`.
    fn runtime<'a>(
        &'a self,
        root: &'a NodeId,
        model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<Box<dyn ProviderRuntime>, ResolveError>>;

    /// The window the token thresholds of a session that infers with
    /// `model` use: the model's context window, or the limit the user set on
    /// it (`models.md` § Context limit).
    fn context_window<'a>(&'a self, model: &'a ModelSelection) -> LocalBoxFuture<'a, u32>;

    /// The provider family of the entry that serves `model`, such as
    /// `anthropic`, which the system prompt's model identity names
    /// (`system-prompt.md` § Model identity).
    fn family<'a>(
        &'a self,
        model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<String, ResolveError>>;

    /// The model selection a child of a profile with the model settings
    /// `model` infers with, built from the entry's current catalog as a
    /// conversation's is on a model switch (`subagents.md` § An unavailable
    /// profile); what is missing when the entry, the model, its effort or
    /// its tier is gone. Nothing falls back.
    fn profile_selection<'a>(
        &'a self,
        model: &'a ProfileModel,
    ) -> LocalBoxFuture<'a, Result<ModelSelection, Unavailable>>;
}

/// Why no runtime could be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// No provider entry has this id.
    #[error("Provider \"{0}\" is not available")]
    Unknown(String),
    /// The entry exists but could not build a runtime.
    #[error("{0}")]
    Failed(String),
}

/// Why a conversation's tree could not be restored without a connection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RestoreError {
    /// The tree did not open, with why.
    #[error("the tree did not open: {0}")]
    Open(String),
    #[error("the restored tree did not continue: {0}")]
    Continue(#[from] StoreError),
}

fn fork_store(error: StoreError) -> ForkError {
    ForkError::Store(error.to_string())
}

/// The tree store of each conversation, by its root.
pub type TreeStores = Rc<dyn Fn(&NodeId) -> Rc<dyn AgentTreeStore>>;

/// How the server's trees and connections behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerConfig {
    pub session: SessionConfig,
    /// The most frames a connection's outbox holds; a connection whose
    /// client lags this far is closed.
    pub outbox_frames: usize,
    /// How long a tree stays live once it is detached and quiescent.
    pub idle_tree: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            session: SessionConfig::default(),
            outbox_frames: 4_096,
            idle_tree: Duration::from_secs(600),
        }
    }
}

/// What a product gives the agent server (`runtime.md` § Sessions and
/// turns): what every node is assembled from, and the answers it asks for
/// while a node runs.
pub struct ServerDeps<H: HostResolver> {
    /// What a tree opens with: the commands every node starts from, on
    /// which the server grafts its own groups per node.
    pub toolsets: Rc<dyn ToolsetSource>,
    /// The user's Subagent switch and profiles, read at each spawn and each
    /// `demi agent profiles`.
    pub subagents: Rc<dyn SubagentSource>,
    /// The instructions of every node's system prompt, its identity, which a
    /// profile's replace.
    pub instructions: Rc<str>,
    /// The harness guide every node's system prompt carries after its
    /// identity, which no profile replaces (`system-prompt.md` § Harness
    /// guide).
    pub guide: Rc<str>,
    /// Where a node's shell tools run.
    pub hosts: Rc<H>,
    /// What the model must learn before each request, in this order.
    pub context: Rc<[Rc<dyn ContextSource>]>,
    pub providers: Rc<dyn ProviderResolver>,
    /// Makes each node's shell environment on each Host it uses.
    pub shells: Rc<dyn ShellEnvironmentFactory<H::Host>>,
    pub stores: TreeStores,
    pub clock: Arc<dyn Clock>,
    pub ids: Rc<dyn IdSource>,
    pub config: ServerConfig,
    /// Told the root of a live tree that started or stopped working, whose
    /// root's phase changed, or that was disposed: a product that shows each
    /// conversation's status reads it again (`runtime.md` § Connections and
    /// the live tree).
    pub status_changed: Rc<dyn Fn(&NodeId)>,
}

/// The conversation's tree works, which a reload does not interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the conversation's agents are working")]
pub struct Working;

/// The agent server of one user shard.
pub struct AgentServer<H: HostResolver> {
    deps: ServerDeps<H>,
    /// Each open conversation's live tree, by its root.
    trees: RefCell<HashMap<NodeId, Rc<Tree<H>>>>,
    /// Serializes opening, closing and evicting a conversation's tree, so two
    /// opens of one conversation build one tree.
    opening: KeyedSerialGate<NodeId>,
    /// The disposals of trees that stayed detached and quiescent.
    evictions: TaskTracker,
    next_connection: Cell<u64>,
}

impl<H: HostResolver> AgentServer<H> {
    pub fn new(deps: ServerDeps<H>) -> Rc<Self> {
        Rc::new(Self {
            deps,
            trees: RefCell::new(HashMap::new()),
            opening: KeyedSerialGate::new(),
            evictions: TaskTracker::new(),
            next_connection: Cell::new(0),
        })
    }

    /// A connection for a socket of the conversation `root`, whose tree works
    /// in `cwd` when it is created, whose frames' files `resolver` resolves,
    /// and the receiving end of its outbox.
    pub fn connect(
        self: &Rc<Self>,
        root: NodeId,
        cwd: String,
        resolver: Rc<dyn ContentResolver>,
    ) -> (Connection<H>, FrameRx) {
        let id = self.next_connection.get();
        self.next_connection.set(id + 1);
        Connection::new(self.clone(), id, root, cwd, resolver)
    }

    /// The conversation's live tree, if it has one.
    pub fn tree(&self, root: &NodeId) -> Option<Rc<Tree<H>>> {
        self.trees.borrow().get(root).cloned()
    }

    /// Restores the conversation `root`'s tree, working in `cwd`, with no
    /// connection attached (`runtime.md` § Yield wakeups): it continues as a
    /// restored tree does, so its saved wakeups are armed again, and once it
    /// is quiescent the idle rule evicts it. A tree that is live already is
    /// left as it is.
    pub async fn restore(self: &Rc<Self>, root: &NodeId, cwd: &str) -> Result<(), RestoreError> {
        let _turn = self.opening.acquire(root.clone()).await;
        let (tree, continuation) = self
            .live_or_open(root, cwd)
            .await
            .map_err(|error| RestoreError::Open(error.to_string()))?;
        if let Some(continuation) = continuation {
            tree.continue_restored(continuation).await?;
        }
        Ok(())
    }

    /// The conversation `root`'s live tree, or its tree opened from the store
    /// in `cwd` with what the restored tree does next, which the caller hands
    /// to [`Tree::continue_restored`] once it attached what it attaches. The
    /// caller holds the root's opening order.
    async fn live_or_open(
        self: &Rc<Self>,
        root: &NodeId,
        cwd: &str,
    ) -> Result<(Rc<Tree<H>>, Option<Continuation>), OpenError> {
        match self.tree(root) {
            Some(tree) => Ok((tree, None)),
            None => Tree::open(self, root, cwd).await,
        }
    }

    /// Prepares a switch of the conversation `root`'s live tree to `model`
    /// (`runtime.md` § Model switch): a model of another provider than the
    /// root's latest selection gets its runtime now, so one that cannot run
    /// fails before anything changes. None when no tree is live, since a tree
    /// opens with the selection the conversation's record holds.
    pub async fn prepare_switch(
        &self,
        root: &NodeId,
        model: ModelSelection,
    ) -> Result<Option<ModelSwitch>, ResolveError> {
        let Some(tree) = self.tree(root) else {
            return Ok(None);
        };
        let runtime = if tree.root().session().needs_runtime_for(&model) {
            Some(self.deps.providers.runtime(root, &model).await?)
        } else {
            None
        };
        Ok(Some(ModelSwitch {
            model: Box::new(model),
            runtime,
        }))
    }

    /// Switches the conversation `root`'s live tree to a prepared switch; it
    /// lands at the root's next provider request. With no tree live, or one
    /// that is closing, the switch's runtime is closed: the conversation's
    /// next open takes the selection from its record.
    pub async fn switch_model(&self, root: &NodeId, switch: ModelSwitch) {
        let refused = match self.tree(root) {
            Some(tree) => tree.root().session().update_model(switch).err(),
            None => Some(switch),
        };
        if let Some(switch) = refused {
            switch.discard().await;
        }
    }

    /// Admits a message from the user into the agent `asking` of the
    /// conversation `root`'s open tree, or, when that agent is closed or
    /// closing, into its nearest live ancestor, the root at last
    /// (`permissions.md` § The decision's message). `message` builds the
    /// message for the recipient it is given. Answers the recipient once its
    /// session accepted the message durably; an error when no tree is open or
    /// the recipient refused it.
    pub async fn admit_from_user(
        &self,
        root: &NodeId,
        asking: &NodeId,
        message: impl Fn(&NodeId) -> AgentMessage,
    ) -> Result<NodeId, String> {
        let tree = self.tree(root).ok_or("the conversation's tree is not open")?;
        let mut current = asking.clone();
        loop {
            if let Some(node) = tree.live_agent(&current) {
                match node.session().accept_agent_message(message(&current)).await {
                    Ok(()) => return Ok(current),
                    // It closed meanwhile: its ancestor owns its work.
                    Err(AgentMessageError::Closed) => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            if &current == root {
                return Err("the conversation's root is closing".into());
            }
            let record = tree
                .store()
                .node(&current)
                .await
                .map_err(|error| error.to_string())?;
            current = match record.and_then(|record| record.parent) {
                Some(parent) => parent,
                // A node the tree no longer knows: the root owns its work.
                None => root.clone(),
            };
        }
    }

    /// The live node `node` of the conversation `root`: where the backend's
    /// command router dispatches a job's `rpc` calls (`Node::commands`). A
    /// job of a node that is not live has none.
    pub fn node(&self, root: &NodeId, node: &NodeId) -> Option<Rc<Node<H>>> {
        self.tree(root)?.node(node)
    }

    /// A Fork's seed from the root `source` through its completed text
    /// `target` (`conversation-fork.md` § The fork seed): captured from the
    /// live session in one step when the conversation is open, else read
    /// from its committed checkpoint. The source keeps running either way.
    pub async fn prepare_fork(
        &self,
        source: &NodeId,
        target: &BlockId,
    ) -> Result<Checkpoint, ForkError> {
        if let Some(tree) = self.tree(source) {
            return tree.root().session().prepare_fork(target);
        }
        let store = (self.deps.stores)(source);
        let stored = store.node(source).await.map_err(fork_store)?;
        if !stored.is_some_and(|record| record.parent.is_none()) {
            return Err(ForkError::NotRoot);
        }
        let checkpoint = store
            .session_store(source)
            .load()
            .await
            .map_err(fork_store)?
            .ok_or(ForkError::NoCheckpoint)?;
        fork_seed(&checkpoint.transcript, checkpoint.state, target)
    }

    /// Stores a Fork's seed as the first checkpoint of the new root
    /// `destination`, in one create commit; opening the conversation
    /// assembles its runtime. A seed that is not idle, or holds waiting work
    /// or edit receipts, is refused.
    pub async fn initialize_fork(
        &self,
        destination: &NodeId,
        seed: Checkpoint,
    ) -> Result<(), ForkError> {
        let state = &seed.state;
        let fresh = state.phase == SessionPhase::Idle
            && state.queue.is_empty()
            && state.agent_inputs.is_empty()
            && state.wakeups.is_empty()
            && state.edits.is_empty();
        if !fresh {
            return Err(ForkError::InvalidSeed);
        }
        let block_count = seed.transcript.len();
        let initial = CheckpointUpdate {
            state: seed.state,
            changed_blocks: seed.transcript.into_iter().enumerate().collect(),
            block_count,
        };
        let record = NodeRecord::root(destination.clone(), self.deps.clock.now());
        (self.deps.stores)(destination)
            .create_node(record, initial)
            .await
            .map_err(fork_store)
    }

    /// The roots of the live trees.
    pub fn live_roots(&self) -> Vec<NodeId> {
        self.trees.borrow().keys().cloned().collect()
    }

    /// Closes the conversation `root`'s live tree, so that it opens again
    /// with the product's current toolset: its attached connections receive
    /// `closed` and connect again. A tree that works is not closed; with no
    /// live tree there is nothing to do.
    pub async fn reload(&self, root: &NodeId) -> Result<(), Working> {
        let _turn = self.opening.acquire(root.clone()).await;
        let Some(tree) = self.tree(root) else {
            return Ok(());
        };
        if !tree.is_quiescent() {
            return Err(Working);
        }
        self.dispose_tree(root, &tree).await;
        Ok(())
    }

    /// Disposes every live tree, then waits for evictions under way.
    pub async fn shutdown(&self) {
        let roots: Vec<NodeId> = self.trees.borrow().keys().cloned().collect();
        join_all(roots.iter().map(|root| self.close_tree(root))).await;
        self.evictions.close();
        self.evictions.wait().await;
    }

    /// Disposes the conversation's live tree, under its opening order,
    /// whatever it does: its attached connections receive `closed`. A
    /// conversation that is deleted closes its tree once its work is
    /// stopped (`Tree::interrupt`); with no live tree there is nothing to do.
    pub async fn close_tree(&self, root: &NodeId) {
        let _turn = self.opening.acquire(root.clone()).await;
        let Some(tree) = self.tree(root) else {
            return;
        };
        self.dispose_tree(root, &tree).await;
    }

    /// Disposes `tree` and forgets it. The caller holds the root's opening
    /// order.
    async fn dispose_tree(&self, root: &NodeId, tree: &Rc<Tree<H>>) {
        tree.dispose().await;
        let removed = {
            let mut trees = self.trees.borrow_mut();
            let live = trees.get(root).is_some_and(|live| Rc::ptr_eq(live, tree));
            if live {
                trees.remove(root);
            }
            live
        };
        if removed {
            (self.deps.status_changed)(root);
        }
    }

    /// Disposes a tree whose idle time ran out, unless it was attached again
    /// or became busy meanwhile.
    fn evict(self: &Rc<Self>, root: NodeId) {
        let server = self.clone();
        self.evictions.spawn_local(async move {
            let _turn = server.opening.acquire(root.clone()).await;
            let Some(tree) = server.tree(&root) else {
                return;
            };
            if tree.is_attached() || !tree.is_quiescent() {
                return;
            }
            server.dispose_tree(&root, &tree).await;
        });
    }
}
