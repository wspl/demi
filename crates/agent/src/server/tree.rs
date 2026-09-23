//! A conversation's live tree (`runtime.md` § Connections and the live
//! tree): its nodes, its attachment to at most one connection, the frames its
//! root's session events become, and its eviction once it has stayed
//! detached and quiescent.

use std::{
    rc::{Rc, Weak},
    time::Duration,
};

use demi_agent_protocol::{ModelSwitchApply, ServerFrame};
use demi_core::{ModelSelection, NodeId};
use demi_gates::ActivityGate;
use tokio::sync::watch;
use tokio_util::task::AbortOnDropHandle;

use super::{AgentServer, ResolveError, connection::Outbox};
use crate::{
    AgentHarness, Node,
    node::{self, AssembleError, NodeSpec},
    session::{AdmissionError, Continuation, ModelSwitch, SessionEvent, Settle, Subscription},
    store::{NodeRecord, StoreError},
};

/// A conversation's live tree.
pub struct Tree<H: AgentHarness> {
    root: Rc<Node<H>>,
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
}

impl<H: AgentHarness> Tree<H> {
    /// Opens the conversation's tree from its store, or creates it, with a
    /// runtime for `model`; a restored root aligns with `model` from its next
    /// turn on. The caller holds the root's opening order.
    pub(crate) async fn open(
        server: &Rc<AgentServer<H>>,
        root: &NodeId,
        cwd: &str,
        model: ModelSelection,
    ) -> Result<(Rc<Self>, Option<Continuation>), OpenError> {
        let deps = &server.deps;
        let runtime = deps.providers.runtime(root, &model).await?;
        let assembled = node::assemble(NodeSpec {
            record: NodeRecord::root(root.clone(), deps.clock.now()),
            root: root.clone(),
            cwd: cwd.to_owned(),
            model: model.clone(),
            runtime,
            harness: deps.harness.clone(),
            store: (deps.stores)(root),
            admission: ActivityGate::new(),
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
        let eviction = tokio::task::spawn_local(evict_when_idle(
            Rc::downgrade(server),
            root.clone(),
            sink.attachment.subscribe(),
            session.settle_watch(),
            deps.config.idle_tree,
        ));
        let tree = Rc::new(Self {
            root: Rc::new(assembled.node),
            sink,
            _frames: frames,
            _eviction: AbortOnDropHandle::new(eviction),
        });
        server.trees.borrow_mut().insert(root.clone(), tree.clone());
        Ok((tree, assembled.continuation))
    }

    /// The root node.
    pub fn root(&self) -> &Rc<Node<H>> {
        &self.root
    }

    /// Whether a connection is attached.
    pub fn is_attached(&self) -> bool {
        self.sink.attachment.borrow().is_some()
    }

    pub(crate) fn sink(&self) -> &FrameSink {
        &self.sink
    }

    /// Whether the tree does nothing by itself: no action runs or waits.
    pub(crate) fn is_quiescent(&self) -> bool {
        self.root.session().is_settled()
    }

    /// Attaches a connection and sends it the open handshake in one step, so
    /// nothing happens to the session between `opened` and `pending_steers`.
    pub(crate) fn attach(&self, connection: u64, outbox: Rc<Outbox>) {
        self.sink.attach(Attachment {
            connection,
            outbox: outbox.clone(),
        });
        let session = self.root.session();
        let snapshot = session.transcript();
        for frame in [
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
                pending_steers: Vec::new(),
            },
        ] {
            // A frame the fresh outbox refuses belongs to a socket that is
            // gone; the next emit detaches the connection.
            outbox.push(frame);
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

    /// What a restored root does next (`runtime.md` § Dispose and restore):
    /// an interrupted turn is recorded as such and left to the client, and
    /// the queued messages run again in order. The record and the queue are
    /// saved at once.
    pub(crate) async fn continue_restored(
        &self,
        continuation: Continuation,
    ) -> Result<(), StoreError> {
        let session = self.root.session();
        if continuation.interrupted {
            session.record_interruption();
        }
        for message in continuation.queued {
            // A restored session is live and has run nothing, so it admits
            // them.
            let _ = session.send(message.content, message.id);
        }
        session.flush().await
    }

    /// Disposes the tree: the root session saves its final checkpoint, and
    /// the attached connection receives what the disposal changed, then is
    /// detached.
    pub(crate) async fn dispose(&self) {
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
        SessionEvent::Error { report } => ServerFrame::Error {
            message: report.message.clone(),
            code: report.code.clone(),
            diagnostics: report.diagnostics.clone(),
        },
        SessionEvent::ActionFailed { .. } => return None,
    })
}

/// Waits until the tree has been detached and quiescent for `idle` without
/// a break, then hands the tree to the server for disposal.
async fn evict_when_idle<H: AgentHarness>(
    server: Weak<AgentServer<H>>,
    root: NodeId,
    mut attachment: watch::Receiver<Option<Attachment>>,
    mut settle: watch::Receiver<Settle>,
    idle: Duration,
) {
    loop {
        let idle_now = attachment.borrow_and_update().is_none()
            && *settle.borrow_and_update() == Settle::Settled;
        if idle_now {
            tokio::select! {
                () = tokio::time::sleep(idle) => break,
                changed = attachment.changed() => if changed.is_err() { return },
                changed = settle.changed() => if changed.is_err() { return },
            }
            continue;
        }
        tokio::select! {
            changed = attachment.changed() => if changed.is_err() { return },
            changed = settle.changed() => if changed.is_err() { return },
        }
    }
    if let Some(server) = server.upgrade() {
        server.evict(root);
    }
}
