//! The agent server of one user shard (`runtime.md` § Frame protocol,
//! § Connections and the live tree): it holds each open conversation's live
//! tree and hands out the connections that attach conversation sockets to
//! them. The backend owns the sockets, decodes each client frame, and passes
//! it to its connection; the connection's outbox carries every server frame
//! back.

mod commands;
mod connection;
mod content;
mod tree;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use demi_core::{BlockId, Clock, ModelSelection, NodeId, SessionPhase};
use demi_gates::KeyedSerialGate;
use demi_provider::ProviderRuntime;
use demi_shell::{JobCaller, PortError, StorageOp, StorageReply};
use futures_util::future::{LocalBoxFuture, join_all};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub use connection::{Connection, FrameRx, Outgoing};
pub use content::{ContentError, ContentResolver, FileReference};
pub use tree::Tree;

use crate::{
    AgentHarness, IdSource, Node, SessionConfig, ShellEnvironmentFactory,
    session::{ForkError, fork_seed},
    store::{
        AgentTreeStore, Checkpoint, CheckpointUpdate, CommandStateHistory, NodeRecord, StoreError,
    },
};

/// Where the agent gets a provider runtime for a session. The backend
/// resolves the provider entry the selection names and builds the runtime on
/// the shard that will own it; the agent never sees the provider itself.
pub trait ProviderResolver {
    /// A runtime for a session of the conversation `root` that infers with
    /// `model`.
    fn runtime<'a>(
        &'a self,
        root: &'a NodeId,
        model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<Box<dyn ProviderRuntime>, ResolveError>>;
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

fn fork_store(error: crate::store::StoreError) -> ForkError {
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

/// What a product gives the agent server.
pub struct ServerDeps<H: AgentHarness> {
    pub harness: Rc<H>,
    pub providers: Rc<dyn ProviderResolver>,
    /// Makes each node's shell environment on each Host it uses.
    pub shells: Rc<dyn ShellEnvironmentFactory<H::Host>>,
    pub stores: TreeStores,
    pub clock: Arc<dyn Clock>,
    pub ids: Rc<dyn IdSource>,
    pub config: ServerConfig,
}

/// The agent server of one user shard.
pub struct AgentServer<H: AgentHarness> {
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

impl<H: AgentHarness> AgentServer<H> {
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

    /// The live node `node` of the conversation `root`: where the backend's
    /// command router dispatches a job's `rpc` calls (`Node::commands`) and
    /// relays its command storage (`Node::storage`). A job of a node that is
    /// not live has none.
    pub fn node(&self, root: &NodeId, node: &NodeId) -> Option<Rc<Node<H>>> {
        self.tree(root)?.node(node)
    }

    /// Serves one command-storage message of a job `caller` started, while
    /// its call `call` lives: the job's node and the generation it recorded
    /// (`command-state-history.md` § Mutation API and concurrency). A job of
    /// a node that is not live, or of a generation a rewrite or dispose
    /// ended, cannot read or write.
    pub async fn command_storage(
        &self,
        root: &NodeId,
        caller: &JobCaller,
        op: StorageOp,
        call: CancellationToken,
    ) -> Result<StorageReply, PortError> {
        let node = self
            .node(root, &caller.node)
            .ok_or_else(|| PortError::Storage(StoreError::Invalidated.to_string()))?;
        node.session()
            .job_storage(caller.generation, op, call)
            .await
    }

    fn new_id(&self) -> String {
        self.deps.ids.next_id()
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
            .filter(|checkpoint| checkpoint.state.harness == self.deps.harness.name())
            .ok_or(ForkError::NoCheckpoint)?;
        let commands = CommandStateHistory::restore(checkpoint.command_state)
            .map_err(|error| ForkError::Store(error.to_string()))?;
        fork_seed(&checkpoint.transcript, &commands, checkpoint.state, target)
    }

    /// Stores a Fork's seed as the first checkpoint of the new root
    /// `destination`, in one create commit; opening the conversation
    /// assembles its runtime. A seed that is not idle, holds waiting work or
    /// edit receipts, or belongs to another harness is refused.
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
            && state.edits.is_empty()
            && state.harness == self.deps.harness.name();
        if !fresh {
            return Err(ForkError::InvalidSeed);
        }
        let block_count = seed.transcript.len();
        let initial = CheckpointUpdate {
            state: seed.state,
            command_state: Some(seed.command_state),
            changed_blocks: seed.transcript.into_iter().enumerate().collect(),
            block_count,
        };
        let record = NodeRecord::root(destination.clone(), self.deps.clock.now());
        (self.deps.stores)(destination)
            .create_node(record, initial)
            .await
            .map_err(fork_store)
    }

    /// Disposes every live tree, then waits for evictions under way.
    pub async fn shutdown(&self) {
        let roots: Vec<NodeId> = self.trees.borrow().keys().cloned().collect();
        join_all(roots.iter().map(|root| self.close_tree(root))).await;
        self.evictions.close();
        self.evictions.wait().await;
    }

    /// Disposes the conversation's live tree, under its opening order.
    async fn close_tree(&self, root: &NodeId) {
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
        let mut trees = self.trees.borrow_mut();
        if trees.get(root).is_some_and(|live| Rc::ptr_eq(live, tree)) {
            trees.remove(root);
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
