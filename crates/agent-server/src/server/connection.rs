//! One conversation socket's frame handling: the backend decodes each client
//! frame, resolves its content, and hands it to [`Connection::handle`], one
//! frame at a time in arrival order. Every frame for the connection, a reply
//! or an event, goes through its one bounded outbox in causal order;
//! [`FrameRx`] is the outbox's receiving end.
//!
//! An event of the tree goes to every attached connection; a reply goes only
//! to the connection whose frame it answers. Connections that act at once need
//! no rule of their own: each hands the session its frames one at a time, and
//! the session takes the frames of all of them in the order they reach it, so
//! the rule that decides a lone client's frame decides each. Two sends run in
//! the order they arrive, an edit is refused while another connection's send
//! runs or waits, and each `abort` stops one thing.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
    time::Instant,
};

use demi_agent_session::{
    AgentSession, EditCheck, EditSubmission, accepted, edit_digest, regenerate_digest,
};
use demi_agent_tools::HostResolver;
use demi_conversation_socket_protocol::{
    AgentCode, ClientFrame, ClientFrameKind, EditOutcome, EditRequest, RegenerateRequest, Refusal, RemoveOutcome,
    ServerFrame, SteerOutcome,
};
use demi_provider_common::Ahead;
use demi_shared_types::{BlockId, NodeId, QueuedMessage, SessionPhase, TurnId};
use tokio::sync::mpsc::{self, error::TrySendError};

use super::{
    AgentServer,
    content::{ContentResolver, resolve_edit, resolve_message},
    tree::{Shown, Tree},
};

/// The refusal of a frame that needs a session when the connection has
/// none.
fn no_session() -> Refusal {
    Refusal::new(AgentCode::NoSession, "No session is open on this connection")
}

/// The refusal of a `compact` that names a node the tree no longer runs.
fn agent_ended() -> Refusal {
    Refusal::new(AgentCode::AgentEnded, "The agent has ended")
}

/// A connection's bounded outbox. When it is full, the client lags: the
/// outbox closes, and the backend closes the socket with a code that says so;
/// the client reconnects and adopts the running tree.
pub(crate) struct Outbox {
    sender: RefCell<Option<mpsc::Sender<ServerFrame>>>,
    /// Shared with the receiving end, which holds no sender: the channel
    /// closes once the connection and its tree let go of the outbox.
    lagged: Rc<Cell<bool>>,
}

impl Outbox {
    /// An outbox of at most `frames` frames, and its receiving end.
    pub(crate) fn new(frames: usize) -> (Rc<Self>, FrameRx) {
        let (sender, receiver) = mpsc::channel(frames);
        let lagged = Rc::new(Cell::new(false));
        let outbox = Rc::new(Self {
            sender: RefCell::new(Some(sender)),
            lagged: lagged.clone(),
        });
        (
            outbox,
            FrameRx {
                frames: receiver,
                lagged,
            },
        )
    }

    /// Queues `frame`; false once the outbox lagged or its receiver is gone,
    /// when the frame goes nowhere.
    pub(crate) fn push(&self, frame: ServerFrame) -> bool {
        let mut sender = self.sender.borrow_mut();
        let Some(queue) = sender.as_ref() else {
            return false;
        };
        match queue.try_send(frame) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.lagged.set(true);
                *sender = None;
                false
            }
            Err(TrySendError::Closed(_)) => {
                *sender = None;
                false
            }
        }
    }
}

/// What the outbox has next for the socket.
#[derive(Debug, Clone, PartialEq)]
pub enum Outgoing {
    Frame(ServerFrame),
    /// The client fell behind by a full outbox: close the socket as lagging.
    Lagged,
    /// The connection is gone.
    Closed,
}

/// The receiving end of a connection's outbox.
pub struct FrameRx {
    frames: mpsc::Receiver<ServerFrame>,
    lagged: Rc<Cell<bool>>,
}

impl FrameRx {
    pub async fn recv(&mut self) -> Outgoing {
        if self.lagged.get() {
            return Outgoing::Lagged;
        }
        match self.frames.recv().await {
            _ if self.lagged.get() => Outgoing::Lagged,
            Some(frame) => Outgoing::Frame(frame),
            None => Outgoing::Closed,
        }
    }

    /// The next frame if one is waiting.
    pub fn try_recv(&mut self) -> Option<Outgoing> {
        if self.lagged.get() {
            return Some(Outgoing::Lagged);
        }
        match self.frames.try_recv() {
            Ok(frame) => Some(Outgoing::Frame(frame)),
            Err(mpsc::error::TryRecvError::Empty) => None,
            Err(mpsc::error::TryRecvError::Disconnected) => Some(Outgoing::Closed),
        }
    }
}

/// One conversation socket's handling. It belongs to one conversation: the
/// backend names the conversation, and the client never sends it. Dropping
/// it detaches it; the tree's turns keep running.
pub struct Connection<H: HostResolver> {
    id: u64,
    root: NodeId,
    server: Rc<AgentServer<H>>,
    resolver: Rc<dyn ContentResolver>,
    outbox: Rc<Outbox>,
    /// The children whose transcripts the page shows, as its last
    /// `show_subagents` named them, attached or not.
    shown: Shown,
}

impl<H: HostResolver> Connection<H> {
    pub(super) fn new(
        server: Rc<AgentServer<H>>,
        id: u64,
        root: NodeId,
        resolver: Rc<dyn ContentResolver>,
    ) -> (Self, FrameRx) {
        let (outbox, frames) = Outbox::new(server.deps.config.outbox_frames);
        let connection = Self {
            id,
            root,
            server,
            resolver,
            outbox,
            shown: Shown::default(),
        };
        (connection, frames)
    }

    /// Handles one decoded frame to its end; a frame whose handling waits,
    /// such as a send whose uploads are being numbered and read or an
    /// `abort` waiting for the stop to be recorded, delays the frames behind
    /// it. A failure is answered as a frame, never returned. Answers whether
    /// the session took the message the frame carries: a `send` it admitted
    /// or a `steer` it accepted, which the backend needs to know of a
    /// message saved for later.
    pub async fn handle(&self, frame: ClientFrame) -> bool {
        let command = frame.kind();
        self.handle_as(frame, command).await
    }

    /// Handles `frame` as [`handle`](Self::handle) does, refusing it as the
    /// page's `command`: the backend hands the session a message saved for
    /// later as the `send` it becomes, and the page waits for the answer of
    /// the `send_saved` it sent.
    pub async fn handle_as(&self, frame: ClientFrame, command: ClientFrameKind) -> bool {
        match frame {
            ClientFrame::Open { from, edge } => {
                self.open(from, edge.as_ref()).await;
                false
            }
            ClientFrame::Close {} => {
                self.close().await;
                false
            }
            ClientFrame::ShowSubagents { ids } => {
                self.show(ids);
                false
            }
            frame => match self.attached() {
                Some(tree) => self.dispatch(&tree, frame, command).await,
                None => {
                    self.refuse(frame, command);
                    false
                }
            },
        }
    }

    /// Detaches the connection from its tree, when the socket is gone.
    pub fn detach(&self) {
        if let Some(tree) = self.server.tree(&self.root) {
            tree.sink().detach(self.id);
        }
    }

    /// The live tree this connection is attached to.
    fn attached(&self) -> Option<Rc<Tree<H>>> {
        self.server
            .tree(&self.root)
            .filter(|tree| tree.sink().is_attached_to(self.id))
    }

    fn send(&self, frame: ServerFrame) {
        // A frame for a connection that lagged or lost its socket goes
        // nowhere; the socket is closing.
        let _ = self.outbox.push(frame);
    }

    fn reject(&self, command: ClientFrameKind, refusal: Refusal) {
        self.send(ServerFrame::Rejected {
            command,
            refusal,
            request_id: None,
        });
    }

    /// An `error` frame of the agent's own failure `code`.
    fn error(&self, code: AgentCode, error: impl std::fmt::Display) {
        self.send(ServerFrame::Error {
            message: error.to_string(),
            code: Some(code.to_string()),
            diagnostics: None,
            request_id: None,
        });
    }

    fn steer_result(&self, steer_id: BlockId, outcome: Result<(), Refusal>) {
        let outcome = match outcome {
            Ok(()) => SteerOutcome::Accepted,
            Err(refusal) => SteerOutcome::Rejected { refusal },
        };
        self.send(ServerFrame::SteerResult { steer_id, outcome });
    }

    fn remove_result(&self, id: String, removed: bool) {
        let outcome = if removed {
            RemoveOutcome::Removed
        } else {
            RemoveOutcome::GoneOn
        };
        self.send(ServerFrame::RemoveResult { id, outcome });
    }

    /// Takes `ids` as the children the page shows: one it adds gets its
    /// transcript now when the connection is attached, and the open sends
    /// it otherwise; one it drops streams no more to it.
    fn show(&self, ids: Vec<NodeId>) {
        let ids: HashSet<NodeId> = ids.into_iter().collect();
        let added: Vec<NodeId> = ids.difference(&self.shown.borrow()).cloned().collect();
        self.shown.replace(ids);
        if let Some(tree) = self.attached() {
            for frame in tree.subagent_resets(&added) {
                self.send(frame);
            }
        }
    }

    /// Attaches the connection to the conversation's tree beside any other,
    /// restoring the tree when it is not live; a live tree already follows
    /// the conversation's model selection. A second `open` on one connection
    /// is refused.
    async fn open(&self, from: Option<u32>, edge: Option<&BlockId>) {
        if self.attached().is_some() {
            self.reject(
                ClientFrameKind::Open,
                Refusal::new(
                    AgentCode::SessionAlreadyOpen,
                    "A session is already open on this connection",
                ),
            );
            return;
        }
        let asked = Instant::now();
        let _turn = self.server.opening.acquire(self.root.clone()).await;
        let lock_wait_ms = asked.elapsed().as_millis();
        let opening = Instant::now();
        let was_live = self.server.tree(&self.root).is_some();
        let (tree, continuation) = match self.server.live_or_open(&self.root).await {
            Ok(opened) => opened,
            Err(error) => {
                self.error(AgentCode::OpenFailed, error);
                return;
            }
        };
        let open_ms = opening.elapsed().as_millis();
        // Read before the attach, which takes its frames in one step; the
        // root's transcript is read last, with nothing awaited after it.
        let placement = tree.placement().await;
        let building = Instant::now();
        let reset = match tree.root().session().reset(from, edge).await {
            Ok(reset) => reset,
            Err(error) => {
                self.error(AgentCode::OpenFailed, error);
                return;
            }
        };
        let frames = tree.attach(self.id, self.outbox.clone(), self.shown.clone(), reset, placement.as_ref());
        tracing::info!(
            root = %self.root,
            was_live,
            lock_wait_ms,
            open_ms,
            handshake_ms = building.elapsed().as_millis(),
            handshake_frames = frames,
            shown = self.shown.borrow().len(),
            "a page opened the conversation"
        );
        // Each node's usage follows the handshake, since the window in use
        // is asked for. It is the estimate at the moment it is sent, so a
        // later change still arrives after it.
        for frame in tree.held_context_usage().await {
            self.send(frame);
        }
        // The open is answered. The user will likely send next: its request
        // is known but for its input, and readying it may read the media
        // the open read none of (`crates/provider-claude-code/src/prepared.rs`).
        tree.root().session().prepare_ahead(Ahead::Opened);
        if let Some(continuation) = continuation
            && let Err(error) = tree.continue_restored(continuation).await
        {
            self.error(AgentCode::OpenFailed, error);
        }
    }

    /// Disposes the tree this connection is attached to, which sends every
    /// attached connection `closed`; a connection attached to none is
    /// answered `closed` alone.
    async fn close(&self) {
        if self.attached().is_some() {
            let _turn = self.server.opening.acquire(self.root.clone()).await;
            // Another connection's close may have disposed the tree meanwhile.
            if let Some(tree) = self.attached() {
                self.server.dispose_tree(&self.root, &tree).await;
                return;
            }
        }
        self.send(ServerFrame::Closed);
    }

    /// The answer to a frame that needs a session when none is open. A
    /// removal is answered `gone_on`, since nothing held what it names.
    fn refuse(&self, frame: ClientFrame, command: ClientFrameKind) {
        match frame {
            ClientFrame::Steer { steer_id, .. } => {
                self.send(ServerFrame::SteerResult {
                    steer_id,
                    outcome: SteerOutcome::Rejected {
                        refusal: no_session(),
                    },
                });
            }
            ClientFrame::CancelPendingSteer { steer_id } => {
                self.remove_result(steer_id.into_string(), false);
            }
            ClientFrame::DequeueMessage { message_id } => {
                self.remove_result(message_id.into_string(), false);
            }
            ClientFrame::SteerNow { .. }
            | ClientFrame::AbortSubagents {}
            | ClientFrame::AbortSubagent { .. } => {}
            _ => self.reject(command, no_session()),
        }
    }

    /// Handles a frame of an attached connection; answers whether the
    /// session took the message a `send` or `steer` carries.
    async fn dispatch(&self, tree: &Rc<Tree<H>>, frame: ClientFrame, kind: ClientFrameKind) -> bool {
        let session = tree.root().session();
        match frame {
            ClientFrame::Send {
                message_id,
                content,
                comments,
            } => {
                let (content, media) = match resolve_message(&*self.resolver, content).await {
                    Ok(resolved) => resolved,
                    Err(error) => {
                        self.reject(kind, error.refusal());
                        return false;
                    }
                };
                session.hold_media(media);
                // The session reports the action's course as events; the
                // handle is not needed.
                let message = QueuedMessage {
                    id: message_id,
                    content,
                    comments,
                    automation: None,
                };
                if let Err(error) = session.send_message(message) {
                    self.reject(kind, error.refusal());
                    return false;
                }
                return true;
            }
            ClientFrame::Steer { steer_id, content, comments } => {
                let outcome = match resolve_message(&*self.resolver, content).await {
                    Ok((content, media)) => {
                        session.hold_media(media);
                        session
                            .steer_with_comments(content, comments, steer_id.clone())
                            .map_err(|error| error.refusal())
                    }
                    Err(error) => Err(error.refusal()),
                };
                let accepted = outcome.is_ok();
                self.steer_result(steer_id, outcome);
                return accepted;
            }
            // The session tells the list's change as it happens, so the
            // answer follows the list's frame.
            ClientFrame::CancelPendingSteer { steer_id } => {
                let removed = session.cancel_pending_steer(&steer_id);
                self.remove_result(steer_id.into_string(), removed);
            }
            ClientFrame::SteerNow { steer_id } => session.steer_now(&steer_id),
            // The backend keeps the saved messages and hands the session
            // the `send` or `steer` a saved message becomes.
            ClientFrame::SendSaved { .. } => {
                self.reject(
                    kind,
                    Refusal::new(
                        AgentCode::SentByBackend,
                        "Only the backend sends a message saved for later",
                    ),
                );
            }
            ClientFrame::DequeueMessage { message_id } => {
                let removed = session.dequeue_message(&message_id);
                self.remove_result(message_id.into_string(), removed);
            }
            ClientFrame::SendQueuedMessage { message_id } => {
                session.send_queued_message(&message_id);
            }
            ClientFrame::ClearMessageQueue {} => {
                session.clear_message_queue();
            }
            ClientFrame::Abort {} => {
                let result = session.abort().await;
                // Every command of the conversation whose start waits for
                // its Host ends and never runs, whichever turn left it,
                // though started commands run on (`crates/backend-host-access/src/access.rs`).
                tree.stop_waiting().await;
                self.send(ServerFrame::AbortResult { result });
            }
            ClientFrame::SyncTranscript { from, edge } => {
                let placement = tree.placement().await;
                match tree.root().session().reset(from, edge.as_ref()).await {
                    Ok(reset) => {
                        let frames = tree.fresh_transcripts(reset, placement.as_ref(), &self.shown.borrow());
                        for frame in frames {
                            self.send(frame);
                        }
                    }
                    Err(error) => self.error(AgentCode::FrameFailed, error),
                }
            }
            // The `closed` frames report what these stopped.
            ClientFrame::AbortSubagents {} => tree.abort_children_of(tree.root().id()).await,
            ClientFrame::AbortSubagent { subagent_id } => {
                if tree.is_child_of(&subagent_id, tree.root().id()) {
                    tree.abort_child(&subagent_id).await;
                }
            }
            ClientFrame::Retry {} | ClientFrame::Resume {} => {
                self.recover(session, kind).await;
            }
            // A subagent's context card compacts that child, the root's the root.
            ClientFrame::Compact { node } => match node {
                None => self.recover(session, kind).await,
                Some(id) => match tree.node(&id) {
                    Some(node) => self.recover(node.session(), kind).await,
                    None => self.reject(kind, agent_ended()),
                },
            },
            ClientFrame::EditAndSend { request } => {
                let operation_id = request.operation_id.clone();
                let outcome = match self.edit(session, request).await {
                    Ok(turn_id) => EditOutcome::Accepted { turn_id },
                    Err(refusal) => EditOutcome::Rejected { refusal },
                };
                self.send(ServerFrame::EditResult {
                    operation_id,
                    outcome,
                });
            }
            ClientFrame::Regenerate { request } => {
                let operation_id = request.operation_id.clone();
                let outcome = match regenerate(session, request).await {
                    Ok(turn_id) => EditOutcome::Accepted { turn_id },
                    Err(refusal) => EditOutcome::Rejected { refusal },
                };
                self.send(ServerFrame::EditResult {
                    operation_id,
                    outcome,
                });
            }
            // The switch's patches reach every connection, and the tree
            // tells which children it moved; only a refusal is this one's.
            ClientFrame::SwitchVersion { block_id, to, version } => {
                if let Err(refusal) = session.switch_version(block_id, to, &version).await {
                    self.reject(kind, refusal);
                }
            }
            // What the command prints and its end reach every attached
            // connection as its `shell_output` (`crates/agent-server/src/server/tree/live.rs`);
            // only the write's acknowledgement is a reply.
            ClientFrame::ShellWrite { command_id, stdin } => {
                match tree.shell_write(&command_id, stdin).await {
                    Ok(()) => self.send(ServerFrame::ShellWriteResult { command_id }),
                    Err(error) => self.error(AgentCode::ShellInputFailed, error),
                }
            }
            ClientFrame::ShellAbort { command_id } => {
                if let Err(error) = tree.shell_abort(&command_id).await {
                    self.error(AgentCode::ShellStopFailed, error);
                }
            }
            ClientFrame::Open { .. } | ClientFrame::Close {} | ClientFrame::ShowSubagents { .. } => {
                unreachable!("open, close and show_subagents are handled before dispatch")
            }
        }
        false
    }
}

impl<H: HostResolver> Connection<H> {
    /// Retries or resumes the root's session, or compacts a node's,
    /// refusing while it is busy; a resume carries `reason` to the model.
    async fn recover(&self, session: &AgentSession, kind: ClientFrameKind) {
        if let Some(refusal) = busy(session) {
            self.reject(kind, refusal);
            return;
        }
        // The user compacts only a context that is full enough. The
        // estimate may read media, during which an action can start, so
        // the phase is checked again with nothing awaited before the
        // admission.
        if kind == ClientFrameKind::Compact {
            let refusal = session
                .compaction_refusal()
                .await
                .or_else(|| busy(session));
            if let Some(refusal) = refusal {
                self.reject(kind, refusal);
                return;
            }
        }
        let admitted = match kind {
            ClientFrameKind::Retry => session.retry(),
            ClientFrameKind::Resume => session.resume(None),
            _ => session.compact(),
        };
        if let Err(error) = admitted {
            self.reject(kind, error.refusal());
        }
    }

    /// An edit's acceptance: a repeated request is answered from its
    /// receipt, or shares the acceptance in flight, before any of its files
    /// is resolved; a new one resolves its content and returns once the
    /// replacement is durable.
    async fn edit(&self, session: &AgentSession, request: EditRequest) -> Result<TurnId, Refusal> {
        let digest = edit_digest(&request);
        let check = session
            .check_edit(&request.operation_id, &digest, &request.version)
            .map_err(|error| error.refusal())?;
        let receipt = match check {
            EditCheck::Accepted(receipt) => receipt,
            EditCheck::InFlight(acceptance) => accepted(acceptance)
                .await
                .map_err(|error| error.refusal())?,
            EditCheck::Proceed => {
                let (content, media) = resolve_edit(&*self.resolver, request.content)
                    .await
                    .map_err(|error| error.refusal())?;
                let submission = EditSubmission {
                    operation_id: request.operation_id,
                    target: request.target_block_id,
                    version: request.version,
                    content: Some(content),
                    digest,
                };
                session.hold_media(media);
                session
                    .edit_and_send(submission)
                    .await
                    .map_err(|error| error.refusal())?
            }
        };
        Ok(receipt.turn_id)
    }
}

impl<H: HostResolver> Drop for Connection<H> {
    fn drop(&mut self) {
        self.detach();
    }
}

/// A Regenerate's acceptance, as an edit's: a repeated request is answered
/// from its receipt, or shares the acceptance in flight; a new one returns
/// once the new answer's branch is durable.
async fn regenerate(session: &AgentSession, request: RegenerateRequest) -> Result<TurnId, Refusal> {
    let digest = regenerate_digest(&request);
    let submission = EditSubmission {
        operation_id: request.operation_id,
        target: request.target_block_id,
        version: request.version,
        content: None,
        digest,
    };
    let receipt = session
        .edit_and_send(submission)
        .await
        .map_err(|error| error.refusal())?;
    Ok(receipt.turn_id)
}

/// Why a session refuses an action that needs it idle, or none when it is
/// idle.
fn busy(session: &AgentSession) -> Option<Refusal> {
    let phase = session.phase();
    (phase != SessionPhase::Idle)
        .then(|| Refusal::new(AgentCode::SessionBusy, format!("Session is busy ({phase})")))
}
