//! A conversation's live tree: its root and every live child, the connections
//! attached to it, the frames its sessions' events become, and its eviction
//! once it has stayed detached and quiescent.
//!
//! The supervisor operations on its children are in [`supervisor`], and its
//! commands' live output in [`live`].
//!
//! Every node of the tree, the root, a child or a side question, is one kind of
//! agent session on the conversation's one socket:
//!
//! - A node's transcript goes only to a connection that follows it, at the
//!   version the page holds ([`following`], [`connection::Follows`]), the
//!   root's too, so opening a conversation whose five children each hold
//!   megabytes of output costs the latest page of the nodes it shows.
//! - What a page shows of every agent, its phase, wait, calls being written,
//!   context and cost, is its entry in the tree's agents ([`agents`]), which
//!   every connection that follows the agents follows by version.
//! - A child's blocks never reach another session's inference transcript: the
//!   parent never receiving a child's tool history is what subagents are for.
//!
//! A connection that closes only detaches. A tree detached and quiescent for
//! `ServerConfig::idle_tree` is disposed, saving its checkpoints, and the next
//! open restores it as after a restart. A live child or a running command is
//! not quiescent, so only an idle root is evicted.

mod agents;
mod following;
mod live;
mod profiles;
mod reading;
mod sides;
mod supervisor;
mod told;

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, HashMap},
    rc::{Rc, Weak},
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;
use demi_agent_session::{Continuation, LatestPage, ModelSwitch, SessionEvent, Settle, Status, Subscription};
use demi_agent_store::{AgentTreeStore, NodeRecord, StoreError};
use demi_agent_tools::{HostResolver, shell_status};
use demi_agent_transcript::IdSource;
use demi_conversation_socket_protocol::{
    AgentCost, AgentCode, AgentRole, CacheChart, RequestOutput, STORED_EPOCH, ServerFrame, ShellStatus, TranscriptPatch, Version,
};
use demi_host_interface::{CommandSet, PageState, PageView, RegisterError, ShellEnvironment};
use demi_shared_gates::{ActivityGate, GateState, KeyedSerialGate, Reservation};
use demi_shared_types::{Clock, CommandId, ModelSelection, NodeId, SessionPhase};
use futures_util::future::join_all;
use tokio::sync::watch;
use tokio_util::task::{AbortOnDropHandle, TaskTracker};

pub use self::agents::{KeptAgents, SideCard};
pub(crate) use self::following::{Following, compare_stored};
pub(crate) use self::profiles::{Deleted, ListedProfile, ProfileAnswer};
pub use self::sides::{AskError, side_instruction};
pub(crate) use self::supervisor::{ShownAgent, StartInput, TreeEntry, list_text};
use self::{agents::Agents, live::LiveOutput, reading::OutputReadings, sides::Side, supervisor::{Child, Summary}};
use super::{AgentServer, ResolveError, connection::{Follows, Outbox}};
use crate::{
    Node,
    node::{self, AssembleError, Instructions, NodeRole, NodeSpec, Origin},
};

/// A conversation's live tree.
pub struct Tree<H: HostResolver> {
    server: Weak<AgentServer<H>>,
    root: Rc<Node<H>>,
    /// Every live child at any depth, by id; with the root, the agent
    /// directory (`crates/agent-server/src/server/tree/supervisor.rs`). A
    /// closing child stays until its close is complete.
    children: RefCell<HashMap<NodeId, Rc<Child<H>>>>,
    /// Every live side question, by its node: beside the agent directory, never
    /// in it.
    sides: RefCell<HashMap<NodeId, Rc<Side<H>>>>,
    /// What each closed side question asked, which the conversation's agent
    /// learns of.
    closed_sides: RefCell<HashMap<NodeId, sides::ClosedSide>>,
    /// Every agent of the conversation as its metadata lists it.
    agents: Rc<Agents>,
    /// Every child of the tree, live and closed, by number: read from the
    /// store when the tree opens, and changed in the step that changes a
    /// child, so a page's read of the conversation lists them without an await.
    summaries: RefCell<BTreeMap<u64, Summary>>,
    /// Bumped whenever a child starts or closes, which every supervision
    /// and the eviction watch.
    changes: watch::Sender<u64>,
    /// Serializes the starts of each node's children.
    starts: KeyedSerialGate<NodeId>,
    /// The starts under way past their owner's check, by owner: a node
    /// with one is not quiescent, as if the child were live already.
    starting: RefCell<HashMap<NodeId, usize>>,
    /// The starts past the check of the live limit whose child is not yet
    /// in `children`, which that check, in this tree and in every other of
    /// the user's, counts as live.
    admitted: Cell<usize>,
    /// Starts past their reservation, and closes: what dispose waits for.
    lifecycle: TaskTracker,
    /// Every action of the tree holds a lease on it, and so does a child's
    /// creation; a target switch or an archive reserves it.
    admission: ActivityGate,
    /// The revision of the toolset the tree opened with.
    toolset: Rc<str>,
    store: Rc<dyn AgentTreeStore>,
    clock: Arc<dyn Clock>,
    ids: Rc<dyn IdSource>,
    disposing: Cell<bool>,
    sink: Rc<FrameSink>,
    /// The commands whose new output waits for the pages.
    live: Rc<LiveOutput>,
    /// The shells of closed children while a command of theirs still stops
    /// (`crates/agent-server/src/server/tree/supervisor.rs`), each under the
    /// key it was kept with.
    stopping: RefCell<HashMap<u64, StoppingShells>>,
    /// The key the next shells kept for a closed child go under.
    next_stopping: Cell<u64>,
    /// The root session's events as frames, until the tree is dropped.
    _frames: Subscription,
    /// Changes when a path of the tree is cut, by an edit, a Regenerate, a
    /// switch of versions, a retry or any other rewrite.
    pub(super) paths: watch::Sender<u64>,
    /// Tells the pages which children a cut path moved on or off the
    /// current path.
    _paths: AbortOnDropHandle<()>,
    /// Disposes the tree once it has stayed detached and quiescent.
    _eviction: AbortOnDropHandle<()>,
    /// Tells the product when the tree starts or stops working.
    _working: AbortOnDropHandle<()>,
    /// Sends the commands' new output to the attached connections.
    _live: AbortOnDropHandle<()>,
    /// Sends the root's request output to the attached connections.
    /// Each node's request output, which its pages are sent as it changes.
    readings: Rc<OutputReadings>,
    _reading: AbortOnDropHandle<()>,
}

/// The shells of a child that closed while its commands still ran
/// (`crates/agent-server/src/server/tree/supervisor.rs`).
///
/// The pages see each command stopping, any agent can look at it or stop it,
/// and its environment records its end when its runner reports it.
pub(crate) struct StoppingShells {
    /// The closed child whose commands they are.
    subagent: NodeId,
    environments: Vec<Rc<dyn ShellEnvironment>>,
    /// Forgets them once each of their commands ended.
    _ends: AbortOnDropHandle<()>,
}

/// The pages' view of each command `environment` holds that has not ended.
fn unended_views(environment: &dyn ShellEnvironment) -> impl Iterator<Item = PageView> + use<> {
    environment
        .page_views()
        .into_iter()
        .filter(|view| matches!(view.state, PageState::Running | PageState::Waiting { .. } | PageState::Stopping))
}

/// Where a tree's frames go: the outbox of every attached connection. The
/// turns of a tree with none keep running; their events go nowhere.
pub(crate) struct FrameSink {
    attachments: watch::Sender<Vec<Attachment>>,
    /// Whether any connection is attached, which the nodes' jobs follow
    /// (`crates/agent-server/src/server/tree/live.rs`).
    attached: watch::Sender<bool>,
    /// The frames of the step being delivered, by connection, which go into
    /// its outbox as one step when the step ends ([`FrameSink::step`]).
    step: RefCell<Vec<(u64, Vec<ServerFrame>)>>,
    /// How deep the steps being delivered nest: a listener of one session may
    /// change another, whose step joins the one under way.
    depth: Cell<usize>,
    /// The latest patches of each node that runs, by node, for a page that
    /// follows a transcript it holds at an older version
    /// ([`following::RecentPatches`]).
    recent: RefCell<HashMap<NodeId, following::RecentPatches>>,
}

#[derive(Clone)]
pub(crate) struct Attachment {
    pub(crate) connection: u64,
    pub(crate) outbox: Rc<Outbox>,
    /// What the connection follows.
    pub(crate) follows: Rc<Follows>,
}

impl FrameSink {
    fn new() -> Self {
        Self {
            attachments: watch::Sender::new(Vec::new()),
            attached: watch::Sender::new(false),
            step: RefCell::default(),
            depth: Cell::new(0),
            recent: RefCell::default(),
        }
    }

    /// Runs `deliver`, the delivery of one step of a session, and sends each
    /// connection the frames it emitted as one step, which the page applies
    /// as one change.
    pub(crate) fn step(&self, deliver: impl FnOnce()) {
        self.depth.set(self.depth.get() + 1);
        deliver();
        self.depth.set(self.depth.get() - 1);
        if self.depth.get() > 0 {
            return;
        }
        let steps = std::mem::take(&mut *self.step.borrow_mut());
        let refused: Vec<u64> = steps
            .into_iter()
            .filter_map(|(connection, frames)| {
                let outbox = self
                    .attachments
                    .borrow()
                    .iter()
                    .find(|attachment| attachment.connection == connection)
                    .map(|attachment| attachment.outbox.clone())?;
                (!outbox.push_step(frames)).then_some(connection)
            })
            .collect();
        self.detach_refused(&refused);
    }

    /// Sends the event `frame` to every attached connection. A connection
    /// whose outbox is full, or whose socket is gone, is detached; the
    /// others keep receiving.
    pub(crate) fn emit(&self, frame: ServerFrame) {
        self.emit_where(|_| true, frame);
    }

    /// Sends the node `node`'s frame `frame` to every attached connection
    /// that follows the node, and to no other.
    pub(crate) fn emit_to(&self, node: &NodeId, frame: ServerFrame) {
        self.emit_where(|attachment| attachment.follows.follows_node(node), frame);
    }

    /// Sends the agents' change `frame`, an agent's entry or the root's
    /// request output, to every attached connection that follows the agents,
    /// and to no other.
    pub(crate) fn emit_to_agents(&self, frame: ServerFrame) {
        self.emit_where(|attachment| attachment.follows.follows_agents(), frame);
    }

    /// Sends the patches of the node `node`'s transcript, at `version`, to
    /// the connections that follow it, and keeps them with the node's recent
    /// patches while it `running`; a node that does not run keeps none.
    pub(crate) fn patched(&self, node: &NodeId, patches: &[TranscriptPatch], version: &Version, running: bool) {
        if running {
            self.recent
                .borrow_mut()
                .entry(node.clone())
                .or_default()
                .keep(version.clone(), patches);
        } else {
            self.recent.borrow_mut().remove(node);
        }
        self.emit_to(
            node,
            ServerFrame::TranscriptPatch {
                node: node.clone(),
                patches: patches.to_vec(),
                version: version.clone(),
                failures: None,
                attachments: Vec::new(),
            },
        );
    }

    /// Frees the recent patches of the node `node`, which stopped running.
    pub(crate) fn stopped(&self, node: &NodeId) {
        self.recent.borrow_mut().remove(node);
    }

    /// Sends `frame` to the attached connections `to` picks, as a step of its
    /// own, or as part of the step being delivered.
    fn emit_where(&self, to: impl Fn(&Attachment) -> bool, frame: ServerFrame) {
        if self.depth.get() > 0 {
            let mut step = self.step.borrow_mut();
            for attachment in self.attachments.borrow().iter().filter(|attachment| to(attachment)) {
                match step.iter_mut().find(|(connection, _)| *connection == attachment.connection) {
                    Some((_, frames)) => frames.push(frame.clone()),
                    None => step.push((attachment.connection, vec![frame.clone()])),
                }
            }
            return;
        }
        let refused: Vec<u64> = self
            .attachments
            .borrow()
            .iter()
            .filter(|attachment| to(attachment))
            .filter(|attachment| !attachment.outbox.push(frame.clone()))
            .map(|attachment| attachment.connection)
            .collect();
        self.detach_refused(&refused);
    }

    /// Detaches the connections `refused`, whose outbox lagged or is gone.
    fn detach_refused(&self, refused: &[u64]) {
        if refused.is_empty() {
            return;
        }
        self.change(|attachments| {
            attachments.retain(|attachment| !refused.contains(&attachment.connection));
            true
        });
    }

    pub(crate) fn is_attached_to(&self, connection: u64) -> bool {
        self.attachments
            .borrow()
            .iter()
            .any(|attachment| attachment.connection == connection)
    }

    /// Whether any connection is attached.
    pub(crate) fn attached(&self) -> bool {
        *self.attached.borrow()
    }

    /// Whether any connection is attached, now and at each change.
    pub(crate) fn watch_attached(&self) -> watch::Receiver<bool> {
        self.attached.subscribe()
    }

    /// Attaches a connection beside the others.
    fn attach(&self, attachment: Attachment) {
        self.change(|attachments| {
            attachments.push(attachment);
            true
        });
    }

    pub(crate) fn detach(&self, connection: u64) {
        self.change(|attachments| {
            let before = attachments.len();
            attachments.retain(|attachment| attachment.connection != connection);
            attachments.len() != before
        });
    }

    /// Detaches every connection, each with `closed`.
    fn close_all(&self) {
        let mut detached = Vec::new();
        self.change(|attachments| {
            detached = std::mem::take(attachments);
            !detached.is_empty()
        });
        for attachment in detached {
            // A connection that lagged or lost its socket is detached
            // either way; its `closed` goes nowhere.
            attachment.outbox.push(ServerFrame::Closed);
        }
    }

    /// Changes the attachments with `modify`, which says whether it changed
    /// them, and whether any is attached with them.
    fn change(&self, modify: impl FnOnce(&mut Vec<Attachment>) -> bool) {
        if !self.attachments.send_if_modified(modify) {
            return;
        }
        let attached = !self.attachments.borrow().is_empty();
        self.attached
            .send_if_modified(|current| std::mem::replace(current, attached) != attached);
    }
}

/// Why a conversation could not be opened.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum OpenError {
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    Assemble(#[from] AssembleError),
    /// The product's commands cannot take the `demi agent` group.
    #[error("the demi agent commands cannot be added: {0}")]
    Commands(#[from] RegisterError),
    /// The product could not say what the tree opens with.
    #[error("the commands the conversation opens with cannot be read: {0}")]
    Toolset(String),
}

/// Where one of the conversation's commands is.
pub(crate) enum CommandPlace<H: HostResolver> {
    /// A node's shells hold it.
    Held(Rc<Node<H>>),
    /// The shells of a child that closed while it ran hold it until its
    /// runner reports its end (`crates/agent-server/src/server/tree/supervisor.rs`).
    Stopping(Rc<dyn ShellEnvironment>),
    /// It ended, its end was given, and the store keeps how it ended.
    Stored,
    /// No command of the conversation has the number.
    Unknown,
}

impl<H: HostResolver> Tree<H> {
    /// The revision of the toolset the tree opened with, which the product
    /// compares with its current one: its nodes' system prompts list those
    /// commands, whichever commands they have now.
    pub fn toolset(&self) -> &str {
        &self.toolset
    }

    /// Gives every live node of the tree, the root, each child and each side
    /// question, the product's commands `inherited` with its own runtime
    /// groups, as the user's switch of a plugin asks.
    pub(crate) fn adopt_commands(
        &self,
        server: &Rc<AgentServer<H>>,
        inherited: &Rc<CommandSet>,
    ) -> Result<(), RegisterError> {
        let children = self.children.borrow().values().map(|child| child.node().clone()).collect::<Vec<_>>();
        let sides = self.sides.borrow().values().map(|side| side.node().clone()).collect::<Vec<_>>();
        let nodes = std::iter::once((self.root.clone(), true)).chain(
            children
                .into_iter()
                .chain(sides)
                .map(|node| {
                    let can_spawn = node.record().can_spawn_subagents;
                    (node, can_spawn)
                }),
        );
        for (node, can_spawn) in nodes {
            let commands = server.runtime_commands(inherited, can_spawn)?;
            node.replace_commands(inherited.clone(), commands);
        }
        Ok(())
    }

    /// Opens the conversation's tree from its store, or creates it, with the
    /// model selection its record holds and a runtime for it.
    ///
    /// A restored root switches to that selection at its next provider request.
    /// The caller holds the root's opening order and then hands the
    /// continuation to [`continue_restored`](Self::continue_restored).
    pub(crate) async fn open(
        server: &Rc<AgentServer<H>>,
        root: &NodeId,
    ) -> Result<(Rc<Self>, Option<Continuation>), OpenError> {
        let deps = &server.deps;
        let toolset = server.toolset().await.map_err(OpenError::Toolset)?;
        let inherited = toolset.commands.clone();
        let commands = server.runtime_commands(&inherited, true)?;
        let store = (deps.stores)(root);
        let admission = ActivityGate::new();
        let model = deps.providers.selection(root).await?;
        let stored = Origin::stored(store.as_ref(), root)
            .await
            .map_err(AssembleError::from)?;
        let subagents = store.subagents().await.map_err(AssembleError::from)?;
        let selections: Vec<ModelSelection> = subagents.iter().map(|stored| stored.model().clone()).collect();
        let models = deps.subagents.runs_on(&selections).await;
        let mut summaries = BTreeMap::new();
        for (stored, runs_on) in subagents.into_iter().zip(models) {
            summaries.insert(
                stored.record.number,
                Summary {
                    record: stored.record,
                    runs_on,
                    started_on_path: stored.started_on_path,
                },
            );
        }
        let origin = stored.unwrap_or_else(|| Origin::New {
            record: NodeRecord::root(root.clone(), deps.clock.now()),
            model: model.clone(),
            first_message: None,
        });
        let runtime = deps.providers.runtime(root, &model).await?;
        let sink = Rc::new(FrameSink::new());
        let live = LiveOutput::new(sink.clone(), root.clone());
        let agents = Agents::new(deps.ids.next_id(), sink.clone());
        let assembled = node::assemble(NodeSpec {
            origin,
            role: NodeRole::Root,
            root: root.clone(),
            server: Rc::downgrade(server),
            runtime,
            providers: deps.providers.clone(),
            hosts: deps.hosts.clone(),
            instructions: Instructions {
                identity: deps.instructions.clone(),
                added: None,
            },
            guide: deps.guide.clone(),
            preamble: None,
            context: deps.context.clone(),
            inherited,
            commands,
            store: store.clone(),
            shells: deps.shells.clone(),
            feed: live.feed(None),
            admission: admission.clone(),
            ids: deps.ids.clone(),
            clock: deps.clock.clone(),
            config: deps.config.session,
            call_window_ms: deps.config.call_window_ms,
            reply_files: deps.reply_files.clone(),
        })
        .await?;
        let session = assembled.node.session().clone();
        if assembled.continuation.is_some() {
            // The runtime already serves `model`'s provider. A session just
            // restored is not closing, so it takes the switch, which holds no
            // runtime to close either way.
            let _ = session.update_model(ModelSwitch {
                model: Box::new(model),
                runtime: None,
            });
        }
        let paths = watch::Sender::new(0);
        agents.put(agents::resting(root.clone(), AgentRole::Root));
        agents.change(root, |entry| {
            entry.phase = session.phase();
            entry.waited_ms = session.waited_ms();
            entry.pending_calls = session.pending_calls();
            entry.queue = session.queued_messages();
            entry.pending_steers = session.pending_steers();
            entry.starting = session.starting();
        });
        let readings = OutputReadings::new(sink.clone());
        let frames = session.subscribe_steps({
            let sink = sink.clone();
            let agents = agents.clone();
            let readings = readings.clone();
            let status_changed = deps.status_changed.clone();
            let root = root.clone();
            let paths = paths.clone();
            // Whether the root runs, which its recent patches follow.
            let running = Cell::new(session.phase() != SessionPhase::Idle);
            // A step's frames go to each page as one change.
            move |step: &[SessionEvent]| agents.step(|| for event in step {
                cut_path(&paths, event);
                note_agent(&agents, &root, event);
                note_running(&sink, &root, &running, event);
                if let SessionEvent::OutputChanged { output } = event {
                    readings.changed(&root, output);
                }
                if let SessionEvent::TranscriptChanged { patches, version } = event {
                    sink.patched(&root, patches, version, running.get());
                } else if let Some(frame) = frame_of(event) {
                    sink.emit(frame);
                }
                if let SessionEvent::PhaseChanged { .. } = event {
                    status_changed(&root);
                }
            })
        });
        let changes = watch::Sender::new(0);
        let idle = deps.config.idle_tree;
        let tree = Rc::new_cyclic(|tree: &Weak<Self>| {
            let eviction = tokio::task::spawn_local(evict_when_idle(
                tree.clone(),
                idle,
                sink.attachments.subscribe(),
                session.status_watch(),
                changes.subscribe(),
                live.watch_running(),
            ));
            let working = tokio::task::spawn_local(report_working(
                tree.clone(),
                session.status_watch(),
                changes.subscribe(),
                deps.status_changed.clone(),
            ));
            let sending = tokio::task::spawn_local(live::send_changes(live.clone()));
            let reading = tokio::task::spawn_local(reading::send_changes(readings.clone()));
            let following = tokio::task::spawn_local(follow_paths(tree.clone(), paths.subscribe()));
            Self {
                server: Rc::downgrade(server),
                root: Rc::new(assembled.node),
                children: RefCell::new(HashMap::new()),
                sides: RefCell::new(HashMap::new()),
                closed_sides: RefCell::new(HashMap::new()),
                agents,
                summaries: RefCell::new(summaries),
                changes,
                starts: KeyedSerialGate::new(),
                starting: RefCell::new(HashMap::new()),
                admitted: Cell::new(0),
                lifecycle: TaskTracker::new(),
                admission,
                toolset: toolset.revision,
                store,
                clock: deps.clock.clone(),
                ids: deps.ids.clone(),
                disposing: Cell::new(false),
                sink,
                live,
                stopping: RefCell::new(HashMap::new()),
                next_stopping: Cell::new(0),
                _frames: frames,
                _eviction: AbortOnDropHandle::new(eviction),
                _working: AbortOnDropHandle::new(working),
                _live: AbortOnDropHandle::new(sending),
                readings,
                _reading: AbortOnDropHandle::new(reading),
                paths,
                _paths: AbortOnDropHandle::new(following),
            }
        });
        server.trees.borrow_mut().insert(root.clone(), tree.clone());
        tree.placement().await;
        tree.show_subagents();
        // Read once the tree is listed, so a ledger row announced meanwhile
        // reaches it, and what was kept before it stays behind it.
        let (kept, charts) = tokio::join!(deps.agents.kept(root), deps.agents.charts(root));
        tree.agents.adopt(kept);
        tree.agents.adopt_charts(charts);
        Ok((tree, assembled.continuation))
    }

    pub fn root(&self) -> &Rc<Node<H>> {
        &self.root
    }

    /// The live node `id`: the root, a child at any depth, or a side
    /// question.
    pub fn node(&self, id: &NodeId) -> Option<Rc<Node<H>>> {
        if id == self.root.id() {
            return Some(self.root.clone());
        }
        let child = self.children.borrow().get(id).map(|child| child.node().clone());
        child.or_else(|| self.side(id).map(|side| side.node().clone()))
    }

    /// Takes each live node's context usage into its entry, as its session
    /// estimates it now: an open asks for the window in use.
    ///
    /// None for a node whose session cannot tell it without reading a blob, so
    /// the page shows no figure from before.
    pub(crate) async fn refresh_context(&self) {
        for node in self.nodes() {
            let usage = node.session().held_context_usage().await;
            self.agents.change(node.id(), |entry| entry.context = usage);
        }
    }

    /// The conversation's agents as its metadata lists them, and the version
    /// they are at.
    /// Each live agent's wait is how long it has waited now, which the page
    /// counts from the answer.
    pub fn agents(&self) -> (Version, Vec<demi_conversation_socket_protocol::AgentEntry>) {
        let waits = |id: &NodeId| self.node(id).and_then(|node| node.session().waited_ms());
        (self.agents.version(), self.agents.all(waits))
    }

    /// The tree's live nodes, the root first, then its children and its
    /// side questions.
    pub fn nodes(&self) -> Vec<Rc<Node<H>>> {
        let children = self.children.borrow();
        let sides = self.sides.borrow();
        std::iter::once(self.root.clone())
            .chain(children.values().map(|child| child.node().clone()))
            .chain(sides.values().map(|side| side.node().clone()))
            .collect()
    }

    /// The provider entries the tree's live nodes infer with, the root's
    /// first: a child's profile may name another entry than its parent's
    /// (`crates/agent-server/src/node/mod.rs`).
    pub fn providers(&self) -> Vec<String> {
        let mut providers: Vec<String> = Vec::new();
        for node in self.nodes() {
            let provider = node.session().model().provider_id;
            if !providers.contains(&provider) {
                providers.push(provider);
            }
        }
        providers
    }

    /// The tree's admission: every action of its nodes holds a lease, and a
    /// target switch or an archive reserves the idle tree through it.
    pub fn admission(&self) -> &ActivityGate {
        &self.admission
    }

    /// Interrupts the tree for a transition of a Host it cannot work without,
    /// such as a Cloud reset (`crates/backend-cloud/src/uses.rs`).
    ///
    /// - Its admission is reserved, so no action starts.
    /// - The root's running action is stopped as the user's Stop would stop it.
    /// - Every live child is aborted.
    /// - The root's shells end.
    ///
    /// The reservation is answered once every action has let go of the
    /// admission; what waits to run stays queued and runs once it is dropped.
    /// `hold` names the transition, as the stopped calls' results say it
    /// (`crates/agent-session/src/core.rs`), such as "the Cloud is being
    /// reset".
    pub async fn interrupt(self: &Rc<Self>, hold: &str) -> Reservation {
        let stop = async {
            self.root.session().stop_running(hold).await;
            self.abort_children_of(self.root.id()).await;
            self.interrupt_sides(hold).await;
            self.root.end_shells().await;
        };
        // The reservation is polled first: it takes the free permits before
        // the stopped action gives its lease back, so no waiting action
        // slips in between.
        let (reservation, ()) = tokio::join!(biased; self.admission.reserve(), stop);
        reservation
    }

    /// Whether any connection is attached.
    pub fn is_attached(&self) -> bool {
        self.sink.attached()
    }

    /// The agent `number`, the root's 0, costs `cost` now, as the product's
    /// latest ledger row of it made it, and its cache chart is `chart`, for
    /// a row of a turn call.
    pub fn cost_changed(&self, number: u64, cost: AgentCost, chart: Option<CacheChart>) {
        self.agents.cost_changed(number, cost, chart);
    }

    /// The side question `id`'s card is `card` now, as the product keeps it.
    pub fn side_card_changed(&self, id: &NodeId, card: SideCard) {
        self.agents.card_changed(id, card);
    }

    /// Writes a page's input to the conversation's command `command`.
    ///
    /// The input goes through the shells of the live node that holds it, or of
    /// the closed child it stays with while it stops
    /// (`crates/agent-server/src/server/tree/supervisor.rs`). The root's shells
    /// refuse a command nothing holds.
    pub(crate) async fn shell_write(&self, command: &CommandId, stdin: String) -> Result<(), String> {
        match (self.holder(command), self.stopping_shell(command)) {
            (None, Some(environment)) => environment
                .write(command, Bytes::from(stdin))
                .await
                .map_err(|error| error.to_string()),
            (holder, _) => holder
                .unwrap_or_else(|| self.root.clone())
                .shell_write(command, stdin)
                .await
                .map_err(|error| error.to_string()),
        }
    }

    /// Stops the conversation's command `command` as a page's stop does,
    /// through the shells that hold it, as [`shell_write`](Self::shell_write)
    /// finds them.
    pub(crate) async fn shell_abort(&self, command: &CommandId) -> Result<(), String> {
        match (self.holder(command), self.stopping_shell(command)) {
            (None, Some(environment)) => environment
                .abort(command)
                .await
                .map_err(|error| error.to_string()),
            (holder, _) => holder
                .unwrap_or_else(|| self.root.clone())
                .shell_abort(command)
                .await
                .map_err(|error| error.to_string()),
        }
    }

    /// The node whose shells hold `command`, the root, a live child or a
    /// live side question.
    pub(crate) fn holder(&self, command: &CommandId) -> Option<Rc<Node<H>>> {
        self.nodes().into_iter().find(|node| node.holds(command))
    }

    /// The shells that hold the conversation's command `command`: a live
    /// node's, or a closed child's while the command stops.
    pub(crate) fn environment_of(&self, command: &CommandId) -> Option<Rc<dyn ShellEnvironment>> {
        self.holder(command)
            .and_then(|node| node.environment_of(command))
            .or_else(|| self.stopping_shell(command))
    }

    /// The shells of a closed child that hold `command` while it stops.
    fn stopping_shell(&self, command: &CommandId) -> Option<Rc<dyn ShellEnvironment>> {
        self.stopping
            .borrow()
            .values()
            .flat_map(|shells| &shells.environments)
            .find(|environment| environment.owns_command(command))
            .cloned()
    }

    /// Where the conversation's command `command` is, whichever agent ran it.
    ///
    /// It is held by a node's shells while it runs and until its end is given,
    /// then by a closed child's while it stops, then known to the store, which
    /// keeps its output.
    pub(crate) async fn command_place(&self, command: &CommandId) -> Result<CommandPlace<H>, String> {
        if let Some(node) = self.holder(command) {
            return Ok(CommandPlace::Held(node));
        }
        if let Some(environment) = self.stopping_shell(command) {
            return Ok(CommandPlace::Stopping(environment));
        }
        match self.store.command_end(command).await {
            Ok(Some(_)) => Ok(CommandPlace::Stored),
            Ok(None) => Ok(CommandPlace::Unknown),
            Err(error) => Err(format!("command {command} could not be found: {error}")),
        }
    }

    /// The conversation's store.
    pub(crate) fn store(&self) -> &Rc<dyn AgentTreeStore> {
        &self.store
    }

    pub(crate) fn sink(&self) -> &FrameSink {
        &self.sink
    }

    /// Whether the tree will go on working without the user, which makes its
    /// conversation running: a child is live, or the root runs or waits to run. A
    /// command that outlives its turn does not count, as that section says.
    pub fn works(&self) -> bool {
        !self.children.borrow().is_empty()
            || !self.starting.borrow().is_empty()
            || !quiescent(&self.root.session().status())
    }

    /// Ends every command of the tree's nodes whose start still waits for
    /// its Host, as the user's Stop does.
    pub async fn stop_waiting(&self) {
        let children: Vec<Rc<Node<H>>> = self
            .children
            .borrow()
            .values()
            .map(|child| child.node().clone())
            .collect();
        self.root.stop_waiting().await;
        for child in children {
            child.stop_waiting().await;
        }
    }

    pub fn runs_commands(&self) -> bool {
        self.live.runs_commands()
    }

    /// Whether the tree does nothing by itself: it does not work, and no
    /// command of it runs, which is the conversation's work as well
    /// (`crates/backend-user-shard/src/lifecycle/mod.rs`).
    pub fn is_quiescent(&self) -> bool {
        !self.works() && self.sides_quiescent() && !self.live.runs_commands()
    }

    /// A watch that wakes at each change that may make the tree quiescent or
    /// not, or its admission free to reserve.
    ///
    /// The changes are its root's status, a child's start or close, a command's
    /// start or end, and a lease or a reservation of its admission. The watch
    /// does not keep the tree alive, and it wakes once the tree is gone.
    pub fn watch_quiescence(&self) -> QuiescenceWatch {
        QuiescenceWatch {
            status: self.root.session().status_watch(),
            changes: self.changes.subscribe(),
            running: self.live.watch_running(),
            admission: self.admission.subscribe(),
        }
    }

    /// Attaches a connection beside the others and sends it `opened`, which
    /// carries no data: the page read what it shows by the conversation's
    /// routes, and follows it by the versions it holds
    /// ([`Connection::follow`]).
    pub(crate) fn attach(&self, connection: u64, outbox: Rc<Outbox>, follows: Rc<Follows>) {
        self.sink.attach(Attachment {
            connection,
            outbox: outbox.clone(),
            follows,
        });
        // A frame the fresh outbox refuses belongs to a socket that is gone;
        // the next emit detaches the connection.
        outbox.push(ServerFrame::Opened);
    }

    /// The agents' version now.
    pub fn agents_version(&self) -> Version {
        self.agents.version()
    }

    /// The latest page of the live node `id`'s transcript, read from its
    /// window now, with the version the window is at, the view of each of its
    /// live commands, with those a closed child's shells hold while they stop,
    /// and the output of the request its agent runs; none for a node that is
    /// not live.
    ///
    /// Read at once, nothing awaited, so the page's patches begin at the
    /// version it names.
    pub fn latest_page(&self, id: &NodeId) -> Option<(LatestPage, Vec<ShellStatus>, Option<RequestOutput>)> {
        let node = self.node(id)?;
        let page = node.session().latest_page();
        Some((page, self.live_commands_of(id), node.session().request_output()))
    }

    /// The node `id`'s cache chart, an empty chart for an agent that made no
    /// turn call; none for a side question or a node the collection does not
    /// hold.
    pub fn cache_chart(&self, id: &NodeId) -> Option<CacheChart> {
        self.agents.chart(id)
    }

    /// The view of each live command of the node `id`
    /// (`crates/agent-server/src/server/tree/live.rs`), with those a closed
    /// child's shells hold while they stop.
    fn live_commands_of(&self, id: &NodeId) -> Vec<ShellStatus> {
        let mut statuses: Vec<ShellStatus> = self
            .node(id)
            .map(|node| node.live_views())
            .unwrap_or_default()
            .into_iter()
            .map(shell_status)
            .collect();
        statuses.extend(self.stopping_commands(id));
        statuses
    }

    /// The view of each command that a closed child `id`'s shells hold while
    /// they stop, which its stored page cannot tell.
    pub fn stopping_commands(&self, id: &NodeId) -> Vec<ShellStatus> {
        self.stopping
            .borrow()
            .values()
            .filter(|shells| &shells.subagent == id)
            .flat_map(|shells| shells.environments.iter())
            .flat_map(|environment| unended_views(environment.as_ref()))
            .map(shell_status)
            .collect()
    }

    /// What a restored tree does next
    /// (`crates/agent-server/src/server/tree/supervisor.rs`).
    ///
    /// The root continues under its policy, then its children come back from
    /// the store, each restoring its own.
    pub(crate) async fn continue_restored(
        self: &Rc<Self>,
        continuation: Continuation,
    ) -> Result<(), StoreError> {
        let continued = self.root.continue_from(continuation).await;
        let restoring = std::time::Instant::now();
        tracing::info!(
            event = "tree.restore_started",
            conversation = %self.root.id(),
            "a restored tree's subagents are being restored"
        );
        self.restore_children(&self.root).await;
        tracing::info!(
            event = "tree.restored",
            conversation = %self.root.id(),
            children = self.children.borrow().len(),
            restore_ms = crate::millis(restoring.elapsed()),
            "a restored tree's subagents were restored"
        );
        self.restore_sides().await;
        continued
    }

    /// Stops every session of the tree at once, the root, each child and each
    /// side question, without waiting: what they run is cancelled as by a
    /// shutdown, and nothing new starts.
    pub(crate) fn stop(&self) {
        self.disposing.set(true);
        self.root.session().stop();
        for child in self.children.borrow().values() {
            child.node().session().stop();
        }
        for side in self.sides.borrow().values() {
            side.node().session().stop();
        }
    }

    /// Disposes the tree: every session stops, the starts and closes under
    /// way finish, then every session saves its final checkpoint and the
    /// subtree stays live in the store for the next open.
    ///
    /// Every attached connection receives what the disposal changed, then
    /// `closed`, and is detached.
    pub(crate) async fn dispose(&self) {
        self.stop();
        for child in self.children.borrow().values() {
            child.stop_supervision();
        }
        for side in self.sides.borrow().values() {
            side.stop_supervision();
        }
        // Every session stopped above, so this waits on no work that only
        // their stop would end, such as a spawn that waits for the root's
        // runtime.
        self.lifecycle.close();
        self.lifecycle.wait().await;
        // The commands of closed children still stopping run on, as a
        // disposed node's do; their shells let them go.
        let stopping: Vec<StoppingShells> = self
            .stopping
            .borrow_mut()
            .drain()
            .map(|(_, shells)| shells)
            .collect();
        join_all(
            stopping
                .iter()
                .flat_map(|shells| &shells.environments)
                .map(|environment| environment.detach_all()),
        )
        .await;
        let children: Vec<Rc<Child<H>>> = self
            .children
            .borrow_mut()
            .drain()
            .map(|(_, child)| child)
            .collect();
        let disposals = children.iter().map(|child| async move {
            if let Err(error) = child.node().session().dispose().await {
                tracing::error!(
                    event = "checkpoint.save_failed",
                    conversation = %self.root.id(),
                    node = %child.node().id(),
                    stage = "final",
                    %error,
                    "the final checkpoint was not saved"
                );
            }
        });
        join_all(disposals).await;
        self.dispose_sides().await;
        if let Err(error) = self.root.session().dispose().await {
            tracing::error!(
                event = "checkpoint.save_failed",
                conversation = %self.root.id(),
                node = %self.root.id(),
                stage = "root",
                %error,
                "the final checkpoint was not saved"
            );
            self.sink.emit(ServerFrame::Error {
                message: error.to_string(),
                code: Some(AgentCode::SaveFailed.to_string()),
                diagnostics: None,
                request_id: None,
            });
        }
        self.sink.close_all();
    }

    fn bump(&self) {
        self.changes
            .send_modify(|changes| *changes = changes.wrapping_add(1));
    }

    /// Reports a failure of the tree's own work, which no action answers.
    fn report(&self, message: String) {
        tracing::error!(
            event = "subagent.lifecycle_failed",
            conversation = %self.root.id(),
            reason = %message,
            "a subagent's lifecycle failed"
        );
        self.sink.emit(ServerFrame::Error {
            message,
            code: Some(AgentCode::SubagentFailed.to_string()),
            diagnostics: None,
            request_id: None,
        });
    }
}

/// The frame of the conversation's own that an event of the root's session
/// becomes: a retry and an error; its transcript goes to its subscribers and
/// the rest is its agent entry's. An action's failure was already reported as
/// its `error`.
fn frame_of(event: &SessionEvent) -> Option<ServerFrame> {
    Some(match event {
        SessionEvent::RetryScheduled {
            attempt,
            delay_ms,
            code,
            diagnostics,
        } => ServerFrame::RetryScheduled {
            attempt: *attempt,
            delay_ms: *delay_ms,
            code: code.clone(),
            diagnostics: diagnostics.clone(),
        },
        SessionEvent::Error { report } => ServerFrame::Error {
            message: report.message.clone(),
            code: report.code.clone(),
            diagnostics: report.diagnostics.clone(),
            request_id: None,
        },
        // The rest change the agent's entry (`note_agent`) or its transcript,
        // or are a request's output, which the tree's reading sends (`reading`).
        _ => return None,
    })
}

/// Takes what `event` of the node `id`'s session changes of its agent entry
/// into it: its phase, its wait, the calls being written, its context and its
/// waiting inputs.
pub(super) fn note_agent(agents: &Agents, id: &NodeId, event: &SessionEvent) {
    match event {
        SessionEvent::QueueChanged { queue } => agents.change(id, |entry| entry.queue = queue.clone()),
        SessionEvent::PendingSteersChanged { pending_steers } => {
            agents.change(id, |entry| entry.pending_steers = pending_steers.clone());
        }
        SessionEvent::StartingChanged { starting } => agents.change(id, |entry| entry.starting = starting.clone()),
        SessionEvent::PhaseChanged { phase } => agents.change(id, |entry| entry.phase = *phase),
        SessionEvent::WaitingChanged { waited_ms } => agents.change(id, |entry| entry.waited_ms = *waited_ms),
        SessionEvent::PendingCallsChanged { pending_calls } => {
            agents.change(id, |entry| entry.pending_calls = pending_calls.clone());
        }
        SessionEvent::ContextUsageChanged { usage } => agents.change(id, |entry| entry.context = Some(usage.clone())),
        _ => {}
    }
}

/// Follows whether the node `node` runs by its phase events: a node that
/// stops running frees its recent patches
/// (`following::RecentPatches`).
pub(super) fn note_running(sink: &FrameSink, node: &NodeId, running: &Cell<bool>, event: &SessionEvent) {
    if let SessionEvent::PhaseChanged { phase } = event {
        running.set(*phase != SessionPhase::Idle);
        if *phase == SessionPhase::Idle {
            sink.stopped(node);
        }
    }
}

/// What [`Tree::watch_quiescence`] answers.
pub struct QuiescenceWatch {
    status: watch::Receiver<Status>,
    changes: watch::Receiver<u64>,
    running: watch::Receiver<BTreeSet<CommandId>>,
    admission: watch::Receiver<GateState>,
}

impl QuiescenceWatch {
    /// Resolves at the next change after the last one this watch saw, or
    /// once the tree is gone, after which it resolves at once.
    pub async fn changed(&mut self) {
        // An error is the tree gone, which the caller finds when it looks.
        tokio::select! {
            _ = self.status.changed() => {}
            _ = self.changes.changed() => {}
            _ = self.running.changed() => {}
            _ = self.admission.changed() => {}
        }
    }
}

/// Whether a root in this status does nothing by itself.
fn quiescent(status: &Status) -> bool {
    status.settle == Settle::Settled
}

/// Marks a cut of a node's path in `paths` when `event` tells one: a
/// rewrite starts with a truncate.
pub(super) fn cut_path(paths: &watch::Sender<u64>, event: &SessionEvent) {
    if let SessionEvent::TranscriptChanged { patches, .. } = event
        && patches.iter().any(|patch| matches!(patch, TranscriptPatch::Truncate { .. }))
    {
        paths.send_modify(|cuts| *cuts = cuts.wrapping_add(1));
    }
}

/// Tells the pages, after each cut of a path, which children it moved on
/// or off the current path.
async fn follow_paths<H: HostResolver>(tree: Weak<Tree<H>>, mut paths: watch::Receiver<u64>) {
    while paths.changed().await.is_ok() {
        let Some(live) = tree.upgrade() else {
            return;
        };
        live.paths_changed().await;
    }
}

/// Tells the product, through `status_changed`, that the tree opened, and
/// each time it starts or stops working by itself.
async fn report_working<H: HostResolver>(
    tree: Weak<Tree<H>>,
    mut status: watch::Receiver<Status>,
    mut changes: watch::Receiver<u64>,
    status_changed: Rc<dyn Fn(&NodeId)>,
) {
    let mut reported = None;
    loop {
        status.mark_unchanged();
        changes.mark_unchanged();
        let Some(live) = tree.upgrade() else {
            return;
        };
        let state = live.works();
        if reported != Some(state) {
            reported = Some(state);
            status_changed(live.root.id());
        }
        drop(live);
        // The senders live as long as the tree, whose drop aborts this task.
        tokio::select! {
            changed = status.changed() => if changed.is_err() { return },
            changed = changes.changed() => if changed.is_err() { return },
        }
    }
}

/// Waits until the tree has been detached and quiescent for `idle` without
/// a break, then hands the tree to the server for disposal.
async fn evict_when_idle<H: HostResolver>(
    tree: Weak<Tree<H>>,
    idle: Duration,
    mut attachments: watch::Receiver<Vec<Attachment>>,
    mut status: watch::Receiver<Status>,
    mut changes: watch::Receiver<u64>,
    mut running: watch::Receiver<BTreeSet<CommandId>>,
) {
    loop {
        attachments.mark_unchanged();
        status.mark_unchanged();
        changes.mark_unchanged();
        running.mark_unchanged();
        let Some(live) = tree.upgrade() else {
            return;
        };
        let idle_now = !live.is_attached() && live.is_quiescent();
        drop(live);
        let timeout = async {
            if idle_now {
                tokio::time::sleep(idle).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            () = timeout => break,
            changed = attachments.changed() => if changed.is_err() { return },
            changed = status.changed() => if changed.is_err() { return },
            changed = changes.changed() => if changed.is_err() { return },
            changed = running.changed() => if changed.is_err() { return },
        }
    }
    let Some(live) = tree.upgrade() else {
        return;
    };
    if let Some(server) = live.server.upgrade() {
        server.evict(live.root.id().clone());
    }
}

/// The agents of a conversation whose tree is not live, as its store keeps
/// them: its `subagents` with what each runs on, its `sides`, and what the
/// product `kept`, while the conversation works at `placement`, at the stored
/// version, which a collection no live session runs keeps.
pub(super) fn stored_agents(
    root: &NodeId,
    subagents: Vec<(demi_agent_store::StoredSubagent, demi_conversation_socket_protocol::JobModel)>,
    sides: Vec<NodeRecord>,
    placement: Option<demi_agent_store::Placement>,
    kept: KeptAgents,
    saved: u64,
) -> (Version, Vec<demi_conversation_socket_protocol::AgentEntry>) {
    // No connection is attached to the sink, so the puts announce nothing.
    let agents = Agents::new(STORED_EPOCH.to_owned(), Rc::new(FrameSink::new()));
    agents.set_placement(placement.clone());
    agents.put(agents::resting(root.clone(), AgentRole::Root));
    for (stored, runs_on) in subagents {
        let role = agents::subagent_role(&stored.record, runs_on, placement.as_ref(), stored.started_on_path);
        agents.put(agents::resting(stored.record.id.clone(), role));
    }
    for record in sides {
        if let Some(of) = record.side {
            agents.put(agents::resting(record.id, agents::side_role(of)));
        }
    }
    agents.adopt(kept);
    let stored = Version {
        epoch: STORED_EPOCH.to_owned(),
        revision: saved,
    };
    (stored, agents.all(|_| None))
}
