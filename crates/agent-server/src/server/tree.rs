//! A conversation's live tree (`runtime.md` § Connections and the live
//! tree, `subagents.md`): its root and every live child, the connections
//! attached to it, the frames its sessions' events become, and its eviction
//! once it has stayed detached and quiescent. The supervisor operations on
//! its children are in [`supervisor`], and its commands' live output in
//! [`live`].

mod live;
mod profiles;
mod supervisor;

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap},
    rc::{Rc, Weak},
    sync::Arc,
    time::Duration,
};

use demi_agent_session::{Continuation, ModelSwitch, SessionEvent, Settle, Status, Subscription};
use demi_agent_store::{AgentTreeStore, NodeRecord, StoreError};
use demi_agent_tools::{HostResolver, shell_output};
use demi_agent_transcript::IdSource;
use demi_conversation_socket_protocol::ServerFrame;
use demi_host_interface::RegisterError;
use demi_shared_gates::{ActivityGate, GateState, KeyedSerialGate, Reservation};
use demi_shared_types::{Clock, CommandEnd, CommandId, NodeId};
use futures_util::future::join_all;
use tokio::sync::watch;
use tokio_util::task::{AbortOnDropHandle, TaskTracker};

pub(crate) use self::profiles::ProfileListing;
pub(crate) use self::supervisor::{AgentSnapshot, StartInput, TreeEntry};
use self::{live::LiveOutput, supervisor::Child};
use super::{AgentServer, ResolveError, commands, connection::Outbox};
use crate::{
    Node,
    node::{self, AssembleError, NodeRole, NodeSpec, Origin},
};

/// A conversation's live tree.
pub struct Tree<H: HostResolver> {
    server: Weak<AgentServer<H>>,
    root: Rc<Node<H>>,
    /// Every live child at any depth, by id; with the root, the agent
    /// directory (`subagents.md` § Topology and the agent directory). A
    /// closing child stays until its close is complete.
    children: RefCell<HashMap<NodeId, Rc<Child<H>>>>,
    /// Bumped whenever a child starts or closes, which every supervision
    /// and the eviction watch.
    changes: watch::Sender<u64>,
    /// Serializes the starts of each node's children.
    starts: KeyedSerialGate<NodeId>,
    /// The starts under way past their owner's check, by owner: a node
    /// with one is not quiescent, as if the child were live already.
    starting: RefCell<HashMap<NodeId, usize>>,
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
    /// The root session's events as frames, until the tree is dropped.
    _frames: Subscription,
    /// Disposes the tree once it has stayed detached and quiescent.
    _eviction: AbortOnDropHandle<()>,
    /// Tells the product when the tree starts or stops working.
    _working: AbortOnDropHandle<()>,
    /// Sends the commands' new output to the attached connections.
    _live: AbortOnDropHandle<()>,
}

/// Where a tree's frames go: the outbox of every attached connection. The
/// turns of a tree with none keep running; their events go nowhere.
pub(crate) struct FrameSink {
    attachments: watch::Sender<Vec<Attachment>>,
    /// Whether any connection is attached, which the nodes' jobs follow
    /// (`runtime.md` § Live output).
    attached: watch::Sender<bool>,
}

#[derive(Clone)]
pub(crate) struct Attachment {
    pub(crate) connection: u64,
    pub(crate) outbox: Rc<Outbox>,
}

impl FrameSink {
    fn new() -> Self {
        Self {
            attachments: watch::Sender::new(Vec::new()),
            attached: watch::Sender::new(false),
        }
    }

    /// Sends the event `frame` to every attached connection. A connection
    /// whose outbox is full, or whose socket is gone, is detached; the
    /// others keep receiving.
    pub(crate) fn emit(&self, frame: ServerFrame) {
        let refused: Vec<u64> = self
            .attachments
            .borrow()
            .iter()
            .filter(|attachment| !attachment.outbox.push(frame.clone()))
            .map(|attachment| attachment.connection)
            .collect();
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
    /// It ended, its end was given, and the store keeps how it ended.
    Stored(CommandEnd),
    /// No command of the conversation has the number.
    Unknown,
}

impl<H: HostResolver> Tree<H> {
    /// The revision of the toolset the tree opened with, which the product
    /// compares with its current one.
    pub fn toolset(&self) -> &str {
        &self.toolset
    }

    /// Opens the conversation's tree from its store, or creates it, with the
    /// model selection its record holds and a runtime for it; a restored root
    /// switches to that selection at its next provider request. The caller
    /// holds the root's opening order and then hands the continuation to
    /// [`continue_restored`](Self::continue_restored).
    pub(crate) async fn open(
        server: &Rc<AgentServer<H>>,
        root: &NodeId,
        cwd: &str,
    ) -> Result<(Rc<Self>, Option<Continuation>), OpenError> {
        let deps = &server.deps;
        let toolset = deps.toolsets.current().await.map_err(OpenError::Toolset)?;
        let inherited = toolset.commands.clone();
        let commands = Rc::new(commands::with_runtime_groups(&inherited, server, true)?);
        let store = (deps.stores)(root);
        let admission = ActivityGate::new();
        let model = deps.providers.selection(root).await?;
        let stored = Origin::stored(store.as_ref(), root)
            .await
            .map_err(AssembleError::from)?;
        let origin = stored.unwrap_or_else(|| Origin::New {
            record: NodeRecord::root(root.clone(), deps.clock.now()),
            cwd: cwd.to_owned(),
            model: model.clone(),
            first_message: None,
        });
        let runtime = deps.providers.runtime(root, &model).await?;
        let sink = Rc::new(FrameSink::new());
        let live = LiveOutput::new(sink.clone());
        let assembled = node::assemble(NodeSpec {
            origin,
            role: NodeRole::Root,
            root: root.clone(),
            runtime,
            providers: deps.providers.clone(),
            hosts: deps.hosts.clone(),
            instructions: deps.instructions.clone(),
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
            server: Rc::downgrade(server),
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
        let frames = session.subscribe({
            let sink = sink.clone();
            let status_changed = deps.status_changed.clone();
            let root = root.clone();
            move |event| {
                if let Some(frame) = frame_of(event) {
                    sink.emit(frame);
                }
                if let SessionEvent::PhaseChanged { .. } = event {
                    status_changed(&root);
                }
            }
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
            Self {
                server: Rc::downgrade(server),
                root: Rc::new(assembled.node),
                children: RefCell::new(HashMap::new()),
                changes,
                starts: KeyedSerialGate::new(),
                starting: RefCell::new(HashMap::new()),
                lifecycle: TaskTracker::new(),
                admission,
                toolset: toolset.revision,
                store,
                clock: deps.clock.clone(),
                ids: deps.ids.clone(),
                disposing: Cell::new(false),
                sink,
                live,
                _frames: frames,
                _eviction: AbortOnDropHandle::new(eviction),
                _working: AbortOnDropHandle::new(working),
                _live: AbortOnDropHandle::new(sending),
            }
        });
        server.trees.borrow_mut().insert(root.clone(), tree.clone());
        Ok((tree, assembled.continuation))
    }

    /// The root node.
    pub fn root(&self) -> &Rc<Node<H>> {
        &self.root
    }

    /// The live node `id`: the root, or a child at any depth.
    pub fn node(&self, id: &NodeId) -> Option<Rc<Node<H>>> {
        if id == self.root.id() {
            return Some(self.root.clone());
        }
        self.children
            .borrow()
            .get(id)
            .map(|child| child.node().clone())
    }

    /// The provider entries the tree's live nodes infer with, the root's
    /// first: a child's profile may name another entry than its parent's
    /// (`subagents.md` § Runtime).
    pub fn providers(&self) -> Vec<String> {
        let children = self.children.borrow();
        let nodes = std::iter::once(&self.root).chain(children.values().map(|child| child.node()));
        let mut providers: Vec<String> = Vec::new();
        for node in nodes {
            let provider = node.session().model().provider_id;
            if !providers.contains(&provider) {
                providers.push(provider);
            }
        }
        providers
    }

    /// The tree's admission: every action of its nodes holds a lease, and a
    /// target switch or an archive reserves the idle tree through it
    /// (`runtime.md` § Actions).
    pub fn admission(&self) -> &ActivityGate {
        &self.admission
    }

    /// Interrupts the tree for a transition of a Host it cannot work
    /// without, such as a Cloud reset (`sessions-and-targets.md` § How a
    /// conversation uses a device): its admission is reserved, so no action
    /// starts; the root's running action is stopped as the user's Stop
    /// would stop it, every live child is aborted, and the root's shells
    /// end. The reservation is answered once every action has let go of
    /// the admission; what waits to run stays queued and runs once it is
    /// dropped.
    pub async fn interrupt(self: &Rc<Self>) -> Reservation {
        let stop = async {
            self.root.session().stop_running().await;
            self.abort_children_of(self.root.id()).await;
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

    /// The node whose shells hold `command`: the root or a live child; the
    /// root when none does, whose shells then refuse the handle.
    pub(crate) fn shells_of(&self, command: &CommandId) -> Rc<Node<H>> {
        self.holder(command).unwrap_or_else(|| self.root.clone())
    }

    /// The node whose shells hold `command`, the root or a live child.
    pub(crate) fn holder(&self, command: &CommandId) -> Option<Rc<Node<H>>> {
        if self.root.holds(command) {
            return Some(self.root.clone());
        }
        self.children
            .borrow()
            .values()
            .find(|child| child.node().holds(command))
            .map(|child| child.node().clone())
    }

    /// Where the conversation's command `command` is, whichever agent ran
    /// it: held by a node's shells while it runs and until its end is
    /// given, then known to the store, which keeps its output.
    pub(crate) async fn command_place(&self, command: &CommandId) -> Result<CommandPlace<H>, String> {
        if let Some(node) = self.holder(command) {
            return Ok(CommandPlace::Held(node));
        }
        match self.store.command_end(command).await {
            Ok(Some(end)) => Ok(CommandPlace::Stored(end)),
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
    /// conversation running (`web-api.md` § Sidebar mutations and read
    /// state): a child is live, or the root runs, waits or has a wakeup
    /// scheduled. A command that outlives its turn does not count: its exit
    /// wakes no one.
    pub fn works(&self) -> bool {
        !self.children.borrow().is_empty()
            || !self.starting.borrow().is_empty()
            || !quiescent(&self.root.session().status())
    }

    /// Whether the tree does nothing by itself: it does not work, and no
    /// command of it runs, which is the conversation's work as well
    /// (`resource-lifecycle.md` § Runtime).
    pub fn is_quiescent(&self) -> bool {
        !self.works() && !self.live.runs_commands()
    }

    /// A watch that wakes at each change that may make the tree quiescent or
    /// not, or its admission free to reserve: its root's status, a child's
    /// start or close, a command's start or end, a lease or a reservation of
    /// its admission. It does not keep the tree alive, and it wakes once the
    /// tree is gone.
    pub fn watch_quiescence(&self) -> QuiescenceWatch {
        QuiescenceWatch {
            status: self.root.session().status_watch(),
            changes: self.changes.subscribe(),
            running: self.live.watch_running(),
            admission: self.admission.subscribe(),
        }
    }

    /// Attaches a connection beside the others and sends it the open
    /// handshake in one step, so nothing happens to the tree between
    /// `opened` and the last live command: the root's snapshot frames, then
    /// each live child's `started` and transcript, depth first in spawn
    /// order, then the view of each live command of the tree.
    pub(crate) fn attach(&self, connection: u64, outbox: Rc<Outbox>) {
        self.sink.attach(Attachment {
            connection,
            outbox: outbox.clone(),
        });
        let session = self.root.session();
        let snapshot = session.transcript();
        let root = [
            ServerFrame::Opened,
            ServerFrame::TranscriptReset {
                blocks: snapshot.blocks,
                version: snapshot.version,
                failures: None,
            },
            ServerFrame::Phase {
                phase: session.phase(),
            },
            ServerFrame::Queue {
                queue: session.queued_messages(),
            },
            ServerFrame::PendingSteers {
                pending_steers: session.pending_steers(),
            },
        ];
        for frame in root
            .into_iter()
            .chain(self.replay())
            .chain(self.live_commands())
        {
            // A frame the fresh outbox refuses belongs to a socket that is
            // gone; the next emit detaches the connection.
            outbox.push(frame);
        }
    }

    /// Fresh transcripts of the root and of every live child, with the view
    /// of each live command of the tree: the answer to a connection that saw
    /// a gap in the patch revisions, which goes to that connection alone.
    pub(crate) fn fresh_transcripts(&self) -> Vec<ServerFrame> {
        let snapshot = self.root.session().transcript();
        let reset = ServerFrame::TranscriptReset {
            blocks: snapshot.blocks,
            version: snapshot.version,
            failures: None,
        };
        std::iter::once(reset)
            .chain(self.replay())
            .chain(self.live_commands())
            .collect()
    }

    /// The `shell_output` of each live command (`runtime.md` § Live
    /// output): the root's, then each live child's, depth first in spawn
    /// order.
    fn live_commands(&self) -> Vec<ServerFrame> {
        let mut frames: Vec<ServerFrame> = self
            .root
            .live_views()
            .into_iter()
            .map(|view| shell_output(None, view))
            .collect();
        for child in self.descendants() {
            let id = child.node().id();
            frames.extend(
                child
                    .node()
                    .live_views()
                    .into_iter()
                    .map(|view| shell_output(Some(id.clone()), view)),
            );
        }
        frames
    }

    /// What a restored tree does next (`runtime.md` § Dispose and restore,
    /// `subagents.md` § Persistence): the root continues under its policy,
    /// then its children come back from the store, each restoring its own.
    pub(crate) async fn continue_restored(
        self: &Rc<Self>,
        continuation: Continuation,
    ) -> Result<(), StoreError> {
        let continued = self.root.continue_from(continuation).await;
        self.restore_children(&self.root).await;
        continued
    }

    /// Disposes the tree: the starts and closes under way finish, then every
    /// session saves its final checkpoint and the subtree stays live in the
    /// store for the next open. Every attached connection receives what the
    /// disposal changed, then `closed`, and is detached.
    pub(crate) async fn dispose(&self) {
        self.disposing.set(true);
        for child in self.children.borrow().values() {
            child.stop_supervision();
        }
        self.lifecycle.close();
        self.lifecycle.wait().await;
        let children: Vec<Rc<Child<H>>> = self
            .children
            .borrow_mut()
            .drain()
            .map(|(_, child)| child)
            .collect();
        let disposals = children.iter().map(|child| async move {
            if let Err(error) = child.node().session().dispose().await {
                tracing::error!(node = %child.node().id(), %error, "the final checkpoint was not saved");
            }
        });
        join_all(disposals).await;
        if let Err(error) = self.root.session().dispose().await {
            tracing::error!(root = %self.root.id(), %error, "the final checkpoint was not saved");
            self.sink.emit(ServerFrame::Error {
                message: error.to_string(),
                code: None,
                diagnostics: None,
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
        tracing::error!(root = %self.root.id(), %message, "subagent lifecycle failure");
        self.sink.emit(ServerFrame::Error {
            message,
            code: None,
            diagnostics: None,
        });
    }
}

/// The frame a root session event becomes; an action's failure was already
/// reported as its `error`.
fn frame_of(event: &SessionEvent) -> Option<ServerFrame> {
    Some(match event {
        SessionEvent::TranscriptChanged { patches, revision } => ServerFrame::TranscriptPatch {
            patches: patches.clone(),
            revision: *revision,
            failures: None,
        },
        SessionEvent::PhaseChanged { phase } => ServerFrame::Phase { phase: *phase },
        SessionEvent::ContextUsageChanged { usage } => ServerFrame::ContextUsage { usage: *usage },
        SessionEvent::QueueChanged { queue } => ServerFrame::Queue {
            queue: queue.clone(),
        },
        SessionEvent::PendingSteersChanged { pending_steers } => ServerFrame::PendingSteers {
            pending_steers: pending_steers.clone(),
        },
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
        },
        SessionEvent::ActionFailed { .. } => return None,
    })
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
    status.settle == Settle::Settled && !status.wakeups
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
        let working = live.works();
        if reported != Some(working) {
            reported = Some(working);
            status_changed(live.root.id());
        }
        drop(live);
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
