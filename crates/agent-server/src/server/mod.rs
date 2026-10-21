//! The agent server of one user shard.
//!
//! It holds each open conversation's live tree and hands out the connections
//! that attach conversation sockets to them. The backend owns the sockets,
//! decodes each client frame, and passes it to its connection; the connection's
//! outbox carries every server frame back.

mod commands;
mod connection;
mod content;
mod profile_commands;
mod shell_output;
mod tree;
mod workflow_commands;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use demi_agent_session::{
    AgentMessageError, Continuation, ForkError, ForkSeed, ModelSwitch, SessionConfig, fork_seed,
};
use demi_agent_store::{AgentTreeStore, NodeRecord, Placement, SideOf, StoreError};
use demi_conversation_socket_protocol::{AgentEntry, Version};
use demi_agent_tools::{
    CALL_WINDOW_MS, ContextSource, HostResolver, INTERVAL_FLOOR_MS, ProfileModel, ShellEnvironmentFactory,
    SubagentSource, Toolset, ToolsetSource, Unavailable,
};
use demi_agent_transcript::{ForkEnd, IdSource};

use demi_provider_common::{Ahead, ProviderRuntime};
use demi_host_interface::{CommandSet, RegisterError};
use demi_shared_gates::KeyedSerialGate;
use demi_shared_types::{AgentMessage, BlockId, Clock, ModelSelection, NodeId, SessionPhase};
use futures_util::future::{LocalBoxFuture, join_all};
use tokio_util::task::TaskTracker;

pub use connection::{Connection, FrameRx, Outgoing};
pub use content::{ContentError, ContentResolver, FileReference, ResolvedFiles};
pub use tree::{AskError, KeptAgents, QuiescenceWatch, SideCard, Tree, side_instruction};

use tree::OpenError;

use crate::Node;

/// Where the agent gets a conversation's model selection and the provider
/// runtimes its sessions infer with.
///
/// The backend holds each conversation's selection in its record, resolves the
/// provider entry a selection names and builds the runtime on the shard that
/// will own it. The agent never sees the provider itself.
pub trait ProviderResolver {
    /// The model selection the conversation `root`'s tree opens with: the
    /// one its record holds.
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

    /// The window the token thresholds and the usage of a session that infers
    /// with `model` use.
    ///
    /// It is the model's context window, or the limit the user set on it
    /// (`crates/shared-types/src/context_limit.rs`). For a subagent whose
    /// profile chose a Context `own`, that one replaces the user's
    /// (`crates/agent-server/src/server/tree/profiles.rs`). Either applies
    /// while the window offers it, and the full window otherwise.
    fn context_window<'a>(
        &'a self,
        model: &'a ModelSelection,
        own: Option<u32>,
    ) -> LocalBoxFuture<'a, u32>;

    /// The provider family of the entry that serves `model`, such as
    /// `anthropic`, which the system prompt's model identity names.
    fn family<'a>(
        &'a self,
        model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<String, ResolveError>>;

    /// The model selection a child of a profile with the model settings `model`
    /// infers with.
    ///
    /// It is built from the entry's current catalog as a conversation's is on a
    /// model switch (`crates/agent-server/src/server/tree/profiles.rs`). The
    /// error says what is missing when the entry, the model, its effort or its
    /// tier is gone. Nothing falls back.
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

/// A Fork's seed from the committed checkpoint of the stored node `source`,
/// through the completed text `end` names: a closed conversation's root, or
/// an archived child (`crates/agent-server/src/server/tree/supervisor.rs`).
pub(crate) async fn stored_fork_seed(
    store: &dyn AgentTreeStore,
    source: &NodeId,
    end: ForkEnd<'_>,
) -> Result<ForkSeed, ForkError> {
    let session = store.session_store(source);
    let checkpoint = session.load().await.map_err(fork_store)?.ok_or(ForkError::NoCheckpoint)?;
    // The load reads the window; the blocks before it stay rows.
    fork_seed(&*session, checkpoint.start, &checkpoint.transcript, checkpoint.state, end).await
}

/// The tree store of each conversation, by its root.
pub type TreeStores = Rc<dyn Fn(&NodeId) -> Rc<dyn AgentTreeStore>>;

/// How the server's trees and connections behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerConfig {
    pub session: SessionConfig,
    /// The most steps a connection's outbox holds, each the frames of one
    /// change; a connection whose client lags this far is closed.
    pub outbox_steps: usize,
    /// How long a tree stays live once it is detached and quiescent.
    pub idle_tree: Duration,
    /// The least interval a command reports at, in milliseconds
    /// (`crates/agent-tools/src/lib.rs`): [`INTERVAL_FLOOR_MS`] in the product,
    /// which a test shortens to see reports within its time.
    pub interval_floor_ms: u32,
    /// The longest a `shell` call of a command with an interval watches it,
    /// in milliseconds (`crates/agent-tools/src/lib.rs`): [`CALL_WINDOW_MS`] in the
    /// product, which a test shortens to see a call return within its time.
    pub call_window_ms: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            session: SessionConfig::default(),
            outbox_steps: 4_096,
            idle_tree: Duration::from_secs(600),
            interval_floor_ms: INTERVAL_FLOOR_MS,
            call_window_ms: CALL_WINDOW_MS,
        }
    }
}

/// What a product gives the agent server: what every node is assembled
/// from, and the answers it asks for while a node runs.
pub struct ServerDeps<H: HostResolver> {
    /// What a tree opens with: the commands every node starts from, on
    /// which the server grafts its own groups per node.
    pub toolsets: Rc<dyn ToolsetSource>,
    /// The user's Subagents switch, limits and profiles, read at each spawn
    /// and each `demi agent profiles`.
    pub subagents: Rc<dyn SubagentSource>,
    /// The instructions of every node's system prompt, its identity, which a
    /// profile's replace.
    pub instructions: Rc<str>,
    /// The harness guide every node's system prompt carries after its
    /// identity, which no profile replaces.
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
    /// conversation's status reads it again.
    pub status_changed: Rc<dyn Fn(&NodeId)>,
    /// What the product keeps of a conversation's agents that their entries
    /// show, read once as a tree opens; the product tells each change after
    /// that ([`Tree::cost_changed`], [`Tree::side_card_changed`]).
    pub agents: Rc<dyn AgentsSource>,
    /// Which paths a complete text's code spans name are files on the
    /// conversation's Host (`crates/agent-server/src/node/reply_files.rs`).
    pub reply_files: Rc<dyn crate::node::ReplyFiles>,
}

/// What the product keeps of a conversation's agents: what each has cost,
/// from its usage ledger, and each side question's card.
pub trait AgentsSource {
    /// What the product keeps of the agents of the conversation `root`; what
    /// it cannot read it leaves out, having logged why.
    fn kept<'a>(&'a self, root: &'a NodeId) -> LocalBoxFuture<'a, KeptAgents>;
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
    /// The command sets the nodes of every tree share.
    command_sets: commands::NodeCommandSets,
}

impl<H: HostResolver> AgentServer<H> {
    pub fn new(deps: ServerDeps<H>) -> Rc<Self> {
        Rc::new(Self {
            deps,
            trees: RefCell::new(HashMap::new()),
            opening: KeyedSerialGate::new(),
            evictions: TaskTracker::new(),
            next_connection: Cell::new(0),
            command_sets: commands::NodeCommandSets::default(),
        })
    }

    /// A connection for a socket of the conversation `root`, whose frames'
    /// files `resolver` resolves, and the receiving end of its outbox.
    pub fn connect(
        self: &Rc<Self>,
        root: NodeId,
        resolver: Rc<dyn ContentResolver>,
    ) -> (Connection<H>, FrameRx) {
        let id = self.next_connection.get();
        self.next_connection.set(id + 1);
        Connection::new(self.clone(), id, root, resolver)
    }

    /// The commands a root node that opens now starts from.
    ///
    /// They are the product's current ones with the server's `demi agent` and
    /// `demi shell` groups grafted, whose declarations, such as their
    /// permission categories, the product reads beside its own.
    pub async fn node_commands(self: &Rc<Self>) -> Result<Rc<CommandSet>, String> {
        let toolset = self.toolset().await?;
        self.runtime_commands(&toolset.commands, true)
            .map_err(|error| error.to_string())
    }

    /// Gives every live tree the product's toolset now, as the user's switch
    /// of a plugin asks (`crates/backend-plugins/src/lib.rs`): each node's
    /// commands change, and its system prompt keeps the commands it opened
    /// with.
    pub async fn adopt_toolset(self: &Rc<Self>) -> Result<(), String> {
        let toolset = self.toolset().await?;
        let trees: Vec<_> = self.trees.borrow().values().cloned().collect();
        for tree in trees {
            tree.adopt_commands(self, &toolset.commands)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// The product's toolset now: the one a node holds of the product's
    /// current revision, else the product's, composed once and kept while
    /// a node holds it. Trees that opened with another keep theirs.
    async fn toolset(&self) -> Result<Toolset, String> {
        let revision = self.deps.toolsets.revision().await?;
        if let Some(held) = self.command_sets.toolset(&revision) {
            return Ok(held);
        }
        let composed = self.deps.toolsets.current().await?;
        Ok(self.command_sets.hold_toolset(composed))
    }

    /// `inherited` with the server's `demi agent` and `demi shell` groups for
    /// a node with the spawning right `can_spawn`: the set another node of
    /// the same two holds, or a new one that later nodes share.
    fn runtime_commands(
        self: &Rc<Self>,
        inherited: &Rc<CommandSet>,
        can_spawn: bool,
    ) -> Result<Rc<CommandSet>, RegisterError> {
        if let Some(held) = self.command_sets.grafted(inherited, can_spawn) {
            return Ok(held);
        }
        let commands = Rc::new(commands::with_runtime_groups(inherited, self, can_spawn)?);
        self.command_sets.hold_grafted(inherited, can_spawn, &commands);
        Ok(commands)
    }

    /// The conversation's live tree, if it has one.
    pub fn tree(&self, root: &NodeId) -> Option<Rc<Tree<H>>> {
        self.trees.borrow().get(root).cloned()
    }

    /// The agents of the conversation `root` as its metadata lists them, and
    /// their version: the live tree's own, or, with no tree live, as its store
    /// keeps them while the conversation works at `placement`, at a version of
    /// their own that an open answers whole.
    pub async fn agents(
        &self,
        root: &NodeId,
        placement: Option<Placement>,
    ) -> Result<(Version, Vec<AgentEntry>), StoreError> {
        if let Some(tree) = self.tree(root) {
            return Ok(tree.agents());
        }
        let store = (self.deps.stores)(root);
        let subagents = store.subagents().await?;
        let sides = store.sides().await?;
        let selections: Vec<ModelSelection> = subagents.iter().map(|stored| stored.model.clone()).collect();
        let models = self.deps.subagents.runs_on(&selections).await;
        let kept = self.deps.agents.kept(root).await;
        // A tree opened meanwhile answers instead, so the answer never
        // disagrees with an open made before it.
        if let Some(tree) = self.tree(root) {
            return Ok(tree.agents());
        }
        Ok(tree::stored_agents(
            self.deps.ids.next_id(),
            root,
            subagents.into_iter().zip(models).collect(),
            sides,
            placement,
            kept,
        ))
    }

    /// Restores the conversation `root`'s tree with no connection attached, as
    /// an `open` does.
    ///
    /// It continues as a restored tree does, and once it is quiescent the idle
    /// rule evicts it. A tree that is live already is left as it is.
    pub async fn restore(self: &Rc<Self>, root: &NodeId) -> Result<(), RestoreError> {
        let _turn = self.opening.acquire(root.clone()).await;
        let (tree, continuation) = self
            .live_or_open(root)
            .await
            .map_err(|error| RestoreError::Open(error.to_string()))?;
        if let Some(continuation) = continuation {
            tree.continue_restored(continuation).await?;
        }
        Ok(())
    }

    /// Takes up the command of the conversation `root`'s live tree whose job is
    /// `job`, as the job's `rpc` call asks once the tree is restored
    /// (`crates/agent-session/src/reports.rs`).
    ///
    /// The error says why it could not be taken up, when it could not.
    pub async fn take_up_job(&self, root: &NodeId, job: &str) -> Result<(), String> {
        let unknown = || "rpc requires a live job dispatched to this device".to_owned();
        let tree = self.tree(root).ok_or_else(unknown)?;
        for node in tree.nodes() {
            if let Some(taken) = node.take_up_job(job).await {
                return taken;
            }
        }
        Err(unknown())
    }

    /// The conversation `root`'s live tree, or its tree opened from the store.
    ///
    /// An opened tree comes with what the restored tree does next, which the
    /// caller hands to [`Tree::continue_restored`] once it attached what it
    /// attaches. The caller holds the root's opening order.
    async fn live_or_open(
        self: &Rc<Self>,
        root: &NodeId,
    ) -> Result<(Rc<Tree<H>>, Option<Continuation>), OpenError> {
        match self.tree(root) {
            Some(tree) => Ok((tree, None)),
            None => Tree::open(self, root).await,
        }
    }

    /// Readies ahead the next request of every conversation a page has open
    /// whose model is of the provider entry `provider_id`, or of every one with
    /// none.
    ///
    /// With none is the case when the entry's active account changed
    /// (`crates/provider-claude-code/src/prepared.rs`).
    pub fn prepare_open(&self, provider_id: Option<&str>) {
        let trees: Vec<Rc<Tree<H>>> = self.trees.borrow().values().cloned().collect();
        for tree in trees {
            let session = tree.root().session();
            let of_entry = provider_id
                .is_none_or(|provider_id| session.latest_model().provider_id == provider_id);
            if tree.sink().attached() && of_entry {
                session.prepare_ahead(Ahead::Chosen);
            }
        }
    }

    /// Prepares a switch of the conversation `root`'s live tree to `model`.
    ///
    /// A model of another provider than the root's latest selection gets its
    /// runtime now, so one that cannot run fails before anything changes. None
    /// when no tree is live, since a tree opens with the selection the
    /// conversation's record holds.
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
    /// lands at the root's next provider request.
    ///
    /// With no tree live, or one that is closing, the switch's runtime is
    /// closed: the conversation's next open takes the selection from its
    /// record.
    pub async fn switch_model(&self, root: &NodeId, switch: ModelSwitch) {
        let refused = match self.tree(root) {
            Some(tree) => {
                let session = tree.root().session();
                let refused = session.update_model(switch).err();
                if refused.is_none() {
                    // The next request is known but for its input
                    // (`crates/provider-claude-code/src/prepared.rs`).
                    session.prepare_ahead(Ahead::Chosen);
                }
                refused
            }
            None => Some(switch),
        };
        if let Some(switch) = refused {
            switch.discard().await;
        }
    }

    /// Admits a message from the user into the agent `asking` of the
    /// conversation `root`'s open tree.
    ///
    /// When that agent is closed or closing, the message goes into its nearest
    /// live ancestor, the root at last. `message` builds the message for the
    /// recipient it is given. Answers the recipient once its session accepted
    /// the message durably; an error when no tree is open or the recipient
    /// refused it.
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
    /// `target`.
    ///
    /// It is captured from the live session in one step when the conversation
    /// is open, else read from its committed checkpoint. The source keeps
    /// running either way.
    pub async fn prepare_fork(
        &self,
        source: &NodeId,
        target: &BlockId,
    ) -> Result<ForkSeed, ForkError> {
        let end = ForkEnd::At(target);
        if let Some(tree) = self.tree(source) {
            return tree.root().session().prepare_fork(end).await;
        }
        let store = (self.deps.stores)(source);
        let stored = store.node(source).await.map_err(fork_store)?;
        if !stored.is_some_and(|record| record.parent.is_none()) {
            return Err(ForkError::NotRoot);
        }
        stored_fork_seed(store.as_ref(), source, end).await
    }

    /// Asks the side question `side` of the conversation `root`, whose tree is
    /// open, about `of`.
    ///
    /// Its history is the root's through `of`'s block, captured from the live
    /// root in one step. The root goes on with whatever it does, and never
    /// learns of it. The same question again under its id asks nothing; answers
    /// whether this call asked it.
    pub async fn ask(&self, root: &NodeId, side: NodeId, of: SideOf) -> Result<bool, AskError> {
        self.tree(root).ok_or(AskError::NotOpen)?.ask(side, of).await
    }

    /// Sends the follow-up `text` to the side question `side` of the
    /// conversation `root`, whose tree is open.
    pub async fn ask_more(&self, root: &NodeId, side: NodeId, text: String) -> Result<(), AskError> {
        self.tree(root).ok_or(AskError::NotOpen)?.ask_more(side, text).await
    }

    /// Runs the failed turn of the side question `side` of the conversation
    /// `root`, whose tree is open, again.
    pub async fn retry_side(&self, root: &NodeId, side: NodeId) -> Result<(), AskError> {
        self.tree(root).ok_or(AskError::NotOpen)?.retry_side(side).await
    }

    /// Stores a Fork's seed as the first checkpoint of the new root
    /// `destination`, in one create commit; opening the conversation assembles
    /// its runtime.
    ///
    /// A seed that is not idle, or holds waiting work or edit receipts, is
    /// refused.
    pub async fn initialize_fork(
        &self,
        destination: &NodeId,
        seed: ForkSeed,
    ) -> Result<(), ForkError> {
        let state = &seed.checkpoint.state;
        let fresh = state.phase == SessionPhase::Idle
            && state.queue.is_empty()
            && state.agent_inputs.is_empty()
            && state.reports.is_empty()
            && state.next_turn_reports.is_empty()
            && state.intervals.is_empty()
            && state.edits.is_empty();
        if !fresh {
            return Err(ForkError::InvalidSeed);
        }
        let record = NodeRecord::root(destination.clone(), self.deps.clock.now());
        // A conversation Fork's source is a root: the prefix's node names
        // the conversation whose tree holds its rows.
        let source = seed.prefix.as_ref().map(|prefix| prefix.node.clone());
        seed.create(&*(self.deps.stores)(destination), record, source.as_ref())
            .await
            .map_err(fork_store)
    }

    /// The conversation `root` works elsewhere now, after a move or a switch of
    /// its Host.
    ///
    /// Its live children's pages hear where each works, the conversation's new
    /// directory, which no write to the children records
    /// (`crates/agent-server/src/node/mod.rs`). Each child's next request
    /// learns it from its `working_directory` block.
    pub async fn placement_changed(&self, root: &NodeId) {
        if let Some(tree) = self.tree(root) {
            tree.placement_changed().await;
        }
    }

    pub fn live_roots(&self) -> Vec<NodeId> {
        self.trees.borrow().keys().cloned().collect()
    }

    /// Closes the conversation `root`'s live tree, so that it opens again with
    /// the product's current toolset.
    ///
    /// Its attached connections receive `closed` and connect again. A tree that
    /// works is not closed; with no live tree there is nothing to do.
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

    /// Stops every live tree at once, then disposes each and waits for the
    /// evictions under way: no agent runs on while a close waits for another
    /// (`docs/decisions/0024-a-close-stops-before-it-waits.md`).
    pub async fn shutdown(&self) {
        let trees: Vec<Rc<Tree<H>>> = self.trees.borrow().values().cloned().collect();
        for tree in &trees {
            tree.stop();
        }
        let roots: Vec<NodeId> = self.trees.borrow().keys().cloned().collect();
        join_all(roots.iter().map(|root| self.close_tree(root))).await;
        self.evictions.close();
        self.evictions.wait().await;
    }

    /// Disposes the conversation's live tree, under its opening order, whatever
    /// it does; its attached connections receive `closed`.
    ///
    /// A conversation that is deleted closes its tree once its work is stopped
    /// (`Tree::interrupt`); with no live tree there is nothing to do.
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
