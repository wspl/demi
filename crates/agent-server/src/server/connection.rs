//! One conversation socket's frame handling.
//!
//! The backend decodes each client frame, resolves its content, and hands it to
//! [`Connection::handle`], one frame at a time in arrival order. Every frame
//! for the connection, a reply or an event, goes through its one bounded outbox
//! in causal order; [`FrameRx`] is the outbox's receiving end.
//!
//! An event of the tree goes to every attached connection, a node's messages
//! only to those that subscribe to the node, and a reply only to the
//! connection whose frame it answers.
//!
//! Connections that act at once need no rule of their own. Each hands the
//! session its frames one at a time, and the session takes the frames of all of
//! them in the order they reach it, so the rule that decides a lone client's
//! frame decides each:
//!
//! - Two sends run in the order they arrive.
//! - An edit is refused while another connection's send runs or waits.
//! - Each `abort` stops one thing.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Instant,
};

use demi_agent_session::{
    AgentSession, EditCheck, EditSubmission, accepted, edit_digest, regenerate_digest,
};
use demi_agent_tools::HostResolver;
use demi_conversation_socket_protocol::{
    AgentCode, ClientFrame, ClientFrameKind, EditOutcome, EditRequest, HeldEdge, RegenerateRequest, Refusal,
    RemoveOutcome, ServerFrame, SteerOutcome, Version,
};
use demi_provider_common::Ahead;
use demi_shared_types::{BlockId, NodeId, QueuedMessage, SessionPhase, TurnId};
use tokio::sync::mpsc::{self, error::TrySendError};

use super::{
    AgentServer,
    content::{ContentResolver, resolve_edit, resolve_message},
    tree::{Subscribed, Tree},
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
///
/// Each entry is the frames of one step, which go to the page in one
/// message and are applied together; a reply or a lone event is a step of one
/// frame (`docs/decisions/0023-a-command-settles-on-the-state-it-changed.md`).
pub(crate) struct Outbox {
    sender: RefCell<Option<mpsc::Sender<Vec<ServerFrame>>>>,
    /// Shared with the receiving end, which holds no sender: the channel
    /// closes once the connection and its tree let go of the outbox.
    lagged: Rc<Cell<bool>>,
}

impl Outbox {
    /// An outbox of at most `steps` steps, and its receiving end.
    pub(crate) fn new(steps: usize) -> (Rc<Self>, FrameRx) {
        let (sender, receiver) = mpsc::channel(steps);
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

    /// Queues `frame`, a step of its own; false once the outbox lagged or its
    /// receiver is gone, when the frame goes nowhere.
    pub(crate) fn push(&self, frame: ServerFrame) -> bool {
        self.push_step(vec![frame])
    }

    /// Queues the frames of one step, which go in one message.
    pub(crate) fn push_step(&self, frames: Vec<ServerFrame>) -> bool {
        let mut sender = self.sender.borrow_mut();
        let Some(queue) = sender.as_ref() else {
            return false;
        };
        match queue.try_send(frames) {
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
    /// The frames of one step, which go in one message.
    Frames(Vec<ServerFrame>),
    /// The client fell behind by a full outbox: close the socket as lagging.
    Lagged,
    /// The connection is gone.
    Closed,
}

/// The receiving end of a connection's outbox.
pub struct FrameRx {
    frames: mpsc::Receiver<Vec<ServerFrame>>,
    lagged: Rc<Cell<bool>>,
}

impl FrameRx {
    pub async fn recv(&mut self) -> Outgoing {
        if self.lagged.get() {
            return Outgoing::Lagged;
        }
        match self.frames.recv().await {
            _ if self.lagged.get() => Outgoing::Lagged,
            Some(frames) => Outgoing::Frames(frames),
            None => Outgoing::Closed,
        }
    }

    /// The next frame if one is waiting.
    pub fn try_recv(&mut self) -> Option<Outgoing> {
        if self.lagged.get() {
            return Some(Outgoing::Lagged);
        }
        match self.frames.try_recv() {
            Ok(frames) => Some(Outgoing::Frames(frames)),
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
    /// The nodes whose messages the page shows, as its `subscribe` and
    /// `unsubscribe` named them, attached or not.
    subscribed: Subscribed,
    /// What the page held of each node it subscribed to before the open,
    /// which the open's handshake answers.
    held: RefCell<HashMap<NodeId, Option<HeldEdge>>>,
}

impl<H: HostResolver> Connection<H> {
    pub(super) fn new(
        server: Rc<AgentServer<H>>,
        id: u64,
        root: NodeId,
        resolver: Rc<dyn ContentResolver>,
    ) -> (Self, FrameRx) {
        let (outbox, frames) = Outbox::new(server.deps.config.outbox_steps);
        let connection = Self {
            id,
            root,
            server,
            resolver,
            outbox,
            subscribed: Subscribed::default(),
            held: RefCell::default(),
        };
        (connection, frames)
    }

    /// Handles one decoded frame to its end.
    ///
    /// A frame whose handling waits, such as a send whose uploads are being
    /// numbered and read or an `abort` waiting for the stop to be recorded,
    /// delays the frames behind it. A failure is answered as a frame, never
    /// returned.
    ///
    /// Answers whether the session took the message the frame carries: a `send`
    /// it admitted or a `steer` it accepted, which the backend needs to know of
    /// a message saved for later.
    pub async fn handle(&self, frame: ClientFrame) -> bool {
        let command = frame.kind();
        self.handle_as(frame, command).await
    }

    /// Handles `frame` as [`handle`](Self::handle) does, refusing it as the
    /// page's `command`.
    ///
    /// The backend hands the session a message saved for later as the `send` it
    /// becomes, and the page waits for the answer of the `send_saved` it sent.
    pub async fn handle_as(&self, frame: ClientFrame, command: ClientFrameKind) -> bool {
        match frame {
            ClientFrame::Open { agents, .. } => {
                self.open(agents.as_ref()).await;
                false
            }
            ClientFrame::Close {} => {
                self.close().await;
                false
            }
            ClientFrame::Subscribe { node, held } => {
                self.subscribe(node, held).await;
                false
            }
            ClientFrame::Unsubscribe { node } => {
                self.subscribed.borrow_mut().remove(&node);
                self.held.borrow_mut().remove(&node);
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

    /// Answers a steer: accepted at `at`, the agents' version that shows it,
    /// or refused.
    fn steer_result(&self, steer_id: BlockId, outcome: Result<Option<Version>, Refusal>) {
        let outcome = match outcome {
            Ok(at) => SteerOutcome::Accepted { at },
            Err(refusal) => SteerOutcome::Rejected { refusal },
        };
        self.send(ServerFrame::SteerResult { steer_id, outcome });
    }

    /// Answers a removal, whose outcome the agents show at `at`.
    fn remove_result(&self, id: String, removed: bool, at: Option<Version>) {
        let outcome = if removed {
            RemoveOutcome::Removed
        } else {
            RemoveOutcome::GoneOn
        };
        self.send(ServerFrame::RemoveResult { id, outcome, at });
    }

    /// The agents' version once the step of the command just taken reached
    /// every page: an answer names it, so the page takes the command as done
    /// only once it holds the step
    /// (`docs/decisions/0023-a-command-settles-on-the-state-it-changed.md`).
    async fn shown_at(tree: &Tree<H>, session: &AgentSession) -> Option<Version> {
        session.delivered().await;
        Some(tree.agents_version())
    }

    /// Shows the node `node`'s messages on this connection from now on: an
    /// attached connection receives its transcript now, from the blocks the
    /// page `held`, a closed node's from its last checkpoint, and the open's
    /// handshake sends it otherwise.
    async fn subscribe(&self, node: NodeId, held: Option<HeldEdge>) {
        self.subscribed.borrow_mut().insert(node.clone());
        match self.attached() {
            Some(tree) => {
                for frame in tree.node_frames(&node, held.as_ref()) {
                    self.send(frame);
                }
                self.send_closed(&tree, &node, held.as_ref()).await;
            }
            None => {
                self.held.borrow_mut().insert(node, held);
            }
        }
    }

    /// Sends the closed node `node`'s transcript, when the page does not hold
    /// its closed version, while the connection still subscribes to it.
    async fn send_closed(&self, tree: &Rc<Tree<H>>, node: &NodeId, held: Option<&HeldEdge>) {
        match tree.closed_frames(node, held).await {
            Ok(Some(frame)) if self.subscribed.borrow().contains(node) => self.send(frame),
            Ok(_) => {}
            Err(error) => self.error(AgentCode::FrameFailed, error),
        }
    }

    /// Attaches the connection to the conversation's tree beside any other,
    /// restoring the tree when it is not live.
    ///
    /// A live tree already follows the conversation's model selection. A second
    /// `open` on one connection is refused.
    async fn open(&self, agents: Option<&Version>) {
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
        // Each live node's context usage is read before the attach, which
        // takes its frames in one step with nothing awaited.
        tree.refresh_context().await;
        let building = Instant::now();
        let held = std::mem::take(&mut *self.held.borrow_mut());
        let frames = tree.attach(self.id, self.outbox.clone(), self.subscribed.clone(), agents, &held);
        // The closed nodes it subscribes to follow the handshake: their
        // transcripts are read from the store, and nothing changes them.
        let nodes: Vec<NodeId> = self.subscribed.borrow().iter().cloned().collect();
        for node in nodes {
            self.send_closed(&tree, &node, held.get(&node).and_then(Option::as_ref)).await;
        }
        tracing::info!(
            root = %self.root,
            was_live,
            lock_wait_ms,
            open_ms,
            handshake_ms = building.elapsed().as_millis(),
            handshake_frames = frames,
            subscribed = self.subscribed.borrow().len(),
            "a page opened the conversation"
        );
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
                self.remove_result(steer_id.into_string(), false, None);
            }
            ClientFrame::DequeueMessage { message_id } => {
                self.remove_result(message_id.into_string(), false, None);
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
            ClientFrame::AgentsSince { held } => self.send(tree.agents_since(Some(&held))),
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
                    id: message_id.clone(),
                    content,
                    comments,
                    automation: None,
                };
                if let Err(error) = session.send_message(message) {
                    self.reject(kind, error.refusal());
                    return false;
                }
                let at = Self::shown_at(tree, &session).await;
                self.send(ServerFrame::SendResult { message_id, at });
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
                let outcome = match outcome {
                    Ok(()) => Ok(Self::shown_at(tree, &session).await),
                    Err(refusal) => Err(refusal),
                };
                self.steer_result(steer_id, outcome);
                return accepted;
            }
            ClientFrame::CancelPendingSteer { steer_id } => {
                let removed = session.cancel_pending_steer(&steer_id);
                let at = Self::shown_at(tree, &session).await;
                self.remove_result(steer_id.into_string(), removed, at);
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
                let at = Self::shown_at(tree, &session).await;
                self.remove_result(message_id.into_string(), removed, at);
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
            // The agents' entries show what these stopped.
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
            ClientFrame::Open { .. }
            | ClientFrame::Close {}
            | ClientFrame::Subscribe { .. }
            | ClientFrame::Unsubscribe { .. } => {
                unreachable!("open, close and subscriptions are handled before dispatch")
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
        // The user compacts only a context that is full enough. The estimate
        // may read media, during which an action can start, so the phase is
        // checked again with nothing awaited before the admission.
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

    /// An edit's acceptance.
    ///
    /// A repeated request is answered from its receipt, or shares the
    /// acceptance in flight, before any of its files is resolved. A new one
    /// resolves its content and returns once the replacement is durable.
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
