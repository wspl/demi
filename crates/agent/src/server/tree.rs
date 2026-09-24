//! A conversation's live tree (`runtime.md` § Connections and the live
//! tree, `subagents.md`): its root and every live child, its attachment to at
//! most one connection, the frames its sessions' events become, and its
//! eviction once it has stayed detached and quiescent. The supervisor
//! operations on its children are in [`supervisor`].

mod supervisor;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
    sync::Arc,
    time::Duration,
};

use demi_agent_protocol::{ModelSwitchApply, ServerFrame};
use demi_core::{Clock, ModelSelection, NodeId};
use demi_gates::{ActivityGate, KeyedSerialGate};
use demi_shell::RegisterError;
use futures_util::future::join_all;
use tokio::sync::watch;
use tokio_util::task::{AbortOnDropHandle, TaskTracker};

pub(crate) use self::supervisor::{AgentSnapshot, StartInput, TreeEntry};
use self::supervisor::{Child, INHERIT_PROFILE};
use super::{AgentServer, ResolveError, commands, connection::Outbox};
use crate::{
    AgentHarness, IdSource, Node, Profile,
    node::{self, AssembleError, NodeRole, NodeSpec, Prompt},
    session::{
        AdmissionError, Continuation, ModelSwitch, SessionEvent, Settle, Status, Subscription,
    },
    store::{AgentTreeStore, NodeRecord, StoreError},
};

/// A conversation's live tree.
pub struct Tree<H: AgentHarness> {
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
    /// Starts past their reservation, and closes: what dispose waits for.
    lifecycle: TaskTracker,
    /// Every action of the tree holds a lease on it, and so does a child's
    /// creation; a target switch or an archive reserves it.
    admission: ActivityGate,
    profiles: Rc<[Profile]>,
    store: Rc<dyn AgentTreeStore>,
    clock: Arc<dyn Clock>,
    ids: Rc<dyn IdSource>,
    disposing: Cell<bool>,
    sink: Rc<FrameSink>,
    /// The root session's events as frames, until the tree is dropped.
    _frames: Subscription,
    /// Disposes the tree once it has stayed detached and quiescent.
    _eviction: AbortOnDropHandle<()>,
}

/// Where a tree's frames go: the attached connection's outbox, or nowhere
/// while the tree is detached; its turns keep running either way.
pub(crate) struct FrameSink {
    attachment: watch::Sender<Option<Attachment>>,
}

#[derive(Clone)]
pub(crate) struct Attachment {
    pub(crate) connection: u64,
    pub(crate) outbox: Rc<Outbox>,
}

impl FrameSink {
    fn new() -> Self {
        Self {
            attachment: watch::Sender::new(None),
        }
    }

    /// Sends `frame` to the attached connection. A connection whose outbox is
    /// full, or whose socket is gone, is detached.
    pub(crate) fn emit(&self, frame: ServerFrame) {
        let delivered = match &*self.attachment.borrow() {
            Some(attachment) => attachment.outbox.push(frame),
            None => return,
        };
        if !delivered {
            self.attachment.send_replace(None);
        }
    }

    pub(crate) fn is_attached_to(&self, connection: u64) -> bool {
        matches!(&*self.attachment.borrow(), Some(attachment) if attachment.connection == connection)
    }

    /// Attaches a connection; one attached before it receives `closed` and is
    /// detached.
    fn attach(&self, attachment: Attachment) {
        if let Some(previous) = self.attachment.send_replace(Some(attachment)) {
            // The previous connection is detached either way; a `closed` it
            // cannot take goes nowhere.
            previous.outbox.push(ServerFrame::Closed);
        }
    }

    pub(crate) fn detach(&self, connection: u64) {
        self.attachment.send_if_modified(|attachment| {
            if !matches!(attachment, Some(current) if current.connection == connection) {
                return false;
            }
            *attachment = None;
            true
        });
    }
}

/// Why a conversation could not be opened.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum OpenError {
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    Assemble(#[from] AssembleError),
    #[error(transparent)]
    Admission(#[from] AdmissionError),
    #[error(
        "subagent profile name \"{INHERIT_PROFILE}\" is reserved: omitting --profile already inherits the parent"
    )]
    ReservedProfile,
    /// The harness's commands cannot take the `demi agent` group.
    #[error("the demi agent commands cannot be added: {0}")]
    Commands(#[from] RegisterError),
}

impl<H: AgentHarness> Tree<H> {
    /// Opens the conversation's tree from its store, or creates it, with a
    /// runtime for `model`; a restored root aligns with `model` from its next
    /// turn on. The caller holds the root's opening order and then hands the
    /// continuation to [`continue_restored`](Self::continue_restored).
    pub(crate) async fn open(
        server: &Rc<AgentServer<H>>,
        root: &NodeId,
        cwd: &str,
        model: ModelSelection,
    ) -> Result<(Rc<Self>, Option<Continuation>), OpenError> {
        let deps = &server.deps;
        let profiles: Rc<[Profile]> = deps.harness.profiles().into();
        if profiles
            .iter()
            .any(|profile| profile.name == INHERIT_PROFILE)
        {
            return Err(OpenError::ReservedProfile);
        }
        let inherited = deps.harness.commands();
        let commands = Rc::new(commands::with_agent_group(
            &inherited, server, true, &profiles,
        )?);
        let store = (deps.stores)(root);
        let admission = ActivityGate::new();
        let runtime = deps.providers.runtime(root, &model).await?;
        let assembled = node::assemble(NodeSpec {
            record: NodeRecord::root(root.clone(), deps.clock.now()),
            role: NodeRole::Root,
            root: root.clone(),
            cwd: cwd.to_owned(),
            model: model.clone(),
            runtime,
            harness: deps.harness.clone(),
            prompt: Prompt::Harness,
            preamble_suffix: None,
            inherited,
            commands,
            first_message: None,
            store: store.clone(),
            shells: deps.shells.clone(),
            admission: admission.clone(),
            ids: deps.ids.clone(),
            clock: deps.clock.clone(),
            config: deps.config.session,
        })
        .await?;
        let session = assembled.node.session().clone();
        if assembled.continuation.is_some() {
            // The runtime already serves `model`'s provider.
            session.update_model(ModelSwitch {
                model,
                runtime: None,
                apply: ModelSwitchApply::NextTurn,
            })?;
        }
        let sink = Rc::new(FrameSink::new());
        let frames = session.subscribe({
            let sink = sink.clone();
            move |event| {
                if let Some(frame) = frame_of(event) {
                    sink.emit(frame);
                }
            }
        });
        let changes = watch::Sender::new(0);
        let idle = deps.config.idle_tree;
        let tree = Rc::new_cyclic(|tree: &Weak<Self>| {
            let eviction = tokio::task::spawn_local(evict_when_idle(
                tree.clone(),
                idle,
                sink.attachment.subscribe(),
                session.status_watch(),
                changes.subscribe(),
            ));
            Self {
                server: Rc::downgrade(server),
                root: Rc::new(assembled.node),
                children: RefCell::new(HashMap::new()),
                changes,
                starts: KeyedSerialGate::new(),
                lifecycle: TaskTracker::new(),
                admission,
                profiles,
                store,
                clock: deps.clock.clone(),
                ids: deps.ids.clone(),
                disposing: Cell::new(false),
                sink,
                _frames: frames,
                _eviction: AbortOnDropHandle::new(eviction),
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

    /// The tree's admission: every action of its nodes holds a lease, and a
    /// target switch or an archive reserves the idle tree through it
    /// (`runtime.md` § Actions).
    pub fn admission(&self) -> &ActivityGate {
        &self.admission
    }

    /// Whether a connection is attached.
    pub fn is_attached(&self) -> bool {
        self.sink.attachment.borrow().is_some()
    }

    pub(crate) fn sink(&self) -> &FrameSink {
        &self.sink
    }

    /// Whether the tree does nothing by itself: no child is live, and the
    /// root runs and waits for nothing and has no wakeup scheduled.
    pub fn is_quiescent(&self) -> bool {
        self.children.borrow().is_empty() && quiescent(&self.root.session().status())
    }

    /// Attaches a connection and sends it the open handshake in one step, so
    /// nothing happens to the tree between `opened` and the last child's
    /// transcript: the root's snapshot frames, then each live child's
    /// `started` and transcript, depth first in spawn order.
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
        for frame in root.into_iter().chain(self.replay()) {
            // A frame the fresh outbox refuses belongs to a socket that is
            // gone; the next emit detaches the connection.
            outbox.push(frame);
        }
    }

    /// Sends the attached connection `connection` fresh transcripts of the
    /// root and of every live child, after a gap in the patch revisions.
    pub(crate) fn sync(&self, connection: u64) {
        if !self.sink.is_attached_to(connection) {
            return;
        }
        let snapshot = self.root.session().transcript();
        self.sink.emit(ServerFrame::TranscriptReset {
            blocks: snapshot.blocks,
            version: snapshot.version,
            failures: None,
        });
        for frame in self.replay() {
            self.sink.emit(frame);
        }
    }

    /// Aligns the tree's model with `model`: from the next action, or also
    /// inside a running turn when `apply` says so. A model of another
    /// provider gets its runtime first, so a failure changes nothing.
    pub(crate) async fn align_model(
        &self,
        server: &AgentServer<H>,
        model: ModelSelection,
        apply: ModelSwitchApply,
    ) -> Result<(), OpenError> {
        let session = self.root.session();
        let runtime = if session.needs_runtime_for(&model) {
            Some(
                server
                    .deps
                    .providers
                    .runtime(self.root.id(), &model)
                    .await?,
            )
        } else {
            None
        };
        session.update_model(ModelSwitch {
            model,
            runtime,
            apply,
        })?;
        Ok(())
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
    /// store for the next open. The attached connection receives what the
    /// disposal changed, then is detached.
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
        self.sink.attachment.send_replace(None);
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

/// Whether a root in this status does nothing by itself.
fn quiescent(status: &Status) -> bool {
    status.settle == Settle::Settled && !status.wakeups
}

/// Waits until the tree has been detached and quiescent for `idle` without
/// a break, then hands the tree to the server for disposal.
async fn evict_when_idle<H: AgentHarness>(
    tree: Weak<Tree<H>>,
    idle: Duration,
    mut attachment: watch::Receiver<Option<Attachment>>,
    mut status: watch::Receiver<Status>,
    mut changes: watch::Receiver<u64>,
) {
    loop {
        attachment.mark_unchanged();
        status.mark_unchanged();
        changes.mark_unchanged();
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
            changed = attachment.changed() => if changed.is_err() { return },
            changed = status.changed() => if changed.is_err() { return },
            changed = changes.changed() => if changed.is_err() { return },
        }
    }
    let Some(live) = tree.upgrade() else {
        return;
    };
    if let Some(server) = live.server.upgrade() {
        server.evict(live.root.id().clone());
    }
}
