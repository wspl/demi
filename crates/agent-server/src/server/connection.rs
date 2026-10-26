//! One conversation lane's frame handling.
//!
//! The backend decodes each client frame, resolves its content, and hands it to
//! [`Connection::handle`], one frame at a time in arrival order. Every frame
//! for the connection, a reply or an event, goes through its one bounded outbox
//! in causal order; [`FrameRx`] is the outbox's receiving end.
//!
//! An event of the tree goes to every attached connection; the agents' changes
//! only to those that follow the agents, a node's messages only to those that
//! follow the node, and a reply only to the connection whose frame it answers.
//! `open` sends no state, only `opened`: the page read the agents and each
//! node's latest page by routes before it opened the socket, and its `open` and
//! its `follow` frames name each value it holds with the version it holds. The
//! connection registers the follow ([`Follows`]) and compares the version with
//! the value's current one in one step of the tree
//! (`crates/agent-server/src/server/tree/following.rs`), and the frames that
//! follow carry the `Version` the page applies them against.
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
    collections::HashSet,
    rc::Rc,
    time::Instant,
};

use demi_agent_session::{
    AgentSession, EditCheck, EditSubmission, accepted, edit_digest, regenerate_digest,
};
use demi_agent_tools::HostResolver;
use demi_conversation_lane_protocol::{
    AgentCode, ClientFrame, ClientFrameKind, EditOutcome, EditRequest, Follow, Followed, Held, RegenerateRequest,
    Refusal, RemoveOutcome, STORED_EPOCH, SendMessage, ServerFrame, SteerOutcome, Version,
};
use demi_provider_common::Ahead;
use demi_shared_types::{BlockId, NodeId, QueuedMessage, SessionPhase, TurnId};
use tokio::sync::mpsc::{self, error::TrySendError};

use demi_agent_store::SavedVersions;
use super::{
    AgentServer,
    content::{ContentResolver, resolve_edit, resolve_message},
    tree::{Following, Stop, Tree, compare_stored},
};

/// What a connection follows of the tree: the agents, and the nodes whose
/// messages it shows. A follow is registered before its version is compared
/// ([`Connection::follow`]), so a change committed after the registration is
/// sent and none is lost between the two. It is shared with the connection's
/// attachment to the tree.
#[derive(Default)]
pub struct Follows {
    agents: Cell<bool>,
    nodes: RefCell<HashSet<NodeId>>,
}

impl Follows {
    /// Follows the agents' entries and the root's request output.
    pub fn follow_agents(&self) {
        self.agents.set(true);
    }

    pub fn unfollow_agents(&self) {
        self.agents.set(false);
    }

    /// Follows the node `node`'s patches until [`unfollow_node`](Self::unfollow_node).
    pub fn follow_node(&self, node: NodeId) {
        self.nodes.borrow_mut().insert(node);
    }

    pub fn unfollow_node(&self, node: &NodeId) {
        self.nodes.borrow_mut().remove(node);
    }

    pub fn follows_agents(&self) -> bool {
        self.agents.get()
    }

    pub fn follows_node(&self, node: &NodeId) -> bool {
        self.nodes.borrow().contains(node)
    }

    fn followed_nodes(&self) -> usize {
        self.nodes.borrow().len()
    }
}

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
/// frame.
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

/// One conversation lane's handling. It belongs to one conversation: the
/// backend names the conversation, and the client never sends it. Dropping
/// it detaches it; the tree's turns keep running.
pub struct Connection<H: HostResolver> {
    id: u64,
    root: NodeId,
    server: Rc<AgentServer<H>>,
    resolver: Rc<dyn ContentResolver>,
    outbox: Rc<Outbox>,
    /// What the page follows, as its `open` and its `follow` and `unfollow`
    /// frames registered it, attached or not.
    follows: Rc<Follows>,
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
            follows: Rc::default(),
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
    /// Answers whether the session took what the frame asks of it: a `send`
    /// it admitted, a `steer` it accepted, a stop it carried out, an edit or a
    /// retry it began. The backend needs to know it of a message saved for
    /// later and of an action that denies the requests of the agent's calls
    /// that wait for the user.
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
            ClientFrame::Open { follows, .. } => {
                self.open(follows).await;
                false
            }
            ClientFrame::Close {} => {
                self.close().await;
                false
            }
            ClientFrame::Follow(Follow { value, holds }) => {
                self.follow(value, &holds).await;
                false
            }
            ClientFrame::Unfollow { value } => {
                self.follows.unregister(&value);
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
    /// only once it holds the step.
    async fn shown_at(tree: &Tree<H>, session: &AgentSession) -> Option<Version> {
        session.delivered().await;
        Some(tree.agents_version())
    }

    /// What this connection follows, for a test that registers a follow
    /// without its frame.
    #[cfg(feature = "testing")]
    pub(crate) fn follows(&self) -> Rc<Follows> {
        self.follows.clone()
    }

    /// Follows `value` at the version `held`: registers the follow, then
    /// compares the version with the value's current one, in one step of the
    /// tree, and answers the frames that bring the page current, or `stale`,
    /// which ends the follow.
    ///
    /// A page that holds what it read while no session lived holds the
    /// stored version, which is compared with what the store holds, read
    /// before the step.
    async fn follow(&self, value: Followed, held: &Held) {
        self.follows.register(&value);
        let saved = self.saved_for(std::slice::from_ref(held)).await;
        self.compare(value, held, saved.as_ref());
    }

    /// What the store holds, when a follow holds a stored version.
    async fn saved_for(&self, held: &[Held]) -> Option<SavedVersions> {
        let stored = held
            .iter()
            .any(|held| matches!(held, Held::Version { version, .. } if version.epoch == STORED_EPOCH));
        if !stored {
            return None;
        }
        match (self.server.deps.stores)(&self.root).saved_versions().await {
            Ok(saved) => Some(saved),
            Err(error) => {
                tracing::warn!(
                    event = "stored_versions.read_failed",
                    conversation = %self.root,
                    %error,
                    "the stored versions could not be read"
                );
                None
            }
        }
    }

    /// Compares a follow registered already with the version it holds and
    /// sends what brings the page current.
    ///
    /// A connection with no tree attached, as of an archived conversation or
    /// one with no model yet, is compared with the store alone.
    fn compare(&self, value: Followed, held: &Held, saved: Option<&SavedVersions>) {
        let following = match self.attached() {
            Some(tree) => tree.compare(&value, held, saved),
            None => compare_stored(&value, held, saved),
        };
        let (value_name, node) = match &value {
            Followed::Agents => ("agents", None),
            Followed::Transcript { node } => ("transcript", Some(node.to_string())),
            _ => ("other", None),
        };
        let (held_revision, held_epoch) = match held {
            Held::Version { version, .. } => (Some(version.revision), Some(version.epoch.as_str())),
            Held::Tag { .. } => (None, None),
        };
        match following {
            Following::Current(frames) if !frames.is_empty() => {
                let rebased = frames.iter().any(|frame| matches!(frame, ServerFrame::Rebased { .. }));
                tracing::info!(
                    event = "follow.answered",
                    conversation = %self.root,
                    value = value_name,
                    node,
                    held_revision,
                    held_epoch,
                    sent = if rebased { "rebased" } else { "frames" },
                    frames = frames.len(),
                    "a follow was answered"
                );
                let _ = self.outbox.push_step(frames);
            }
            Following::Current(_) => {
                tracing::info!(
                    event = "follow.answered",
                    conversation = %self.root,
                    value = value_name,
                    node,
                    held_revision,
                    held_epoch,
                    sent = "nothing",
                    "a follow was answered"
                );
            }
            Following::Stale => {
                tracing::info!(
                    event = "follow.answered",
                    conversation = %self.root,
                    value = value_name,
                    node,
                    held_revision,
                    held_epoch,
                    sent = "stale",
                    "a follow was answered"
                );
                self.follows.unregister(&value);
                self.send(ServerFrame::Stale { value });
            }
        }
    }

    /// Attaches the connection to the conversation's tree beside any other,
    /// restoring the tree when it is not live, and follows what the page named
    /// in its `open`.
    ///
    /// A live tree already follows the conversation's model selection. A second
    /// `open` on one connection is refused.
    async fn open(&self, follows: Vec<Follow>) {
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
        let lock_wait_ms = crate::millis(asked.elapsed());
        let opening = Instant::now();
        let was_live = self.server.tree(&self.root).is_some();
        let (tree, continuation) = match self.server.live_or_open(&self.root).await {
            Ok(opened) => opened,
            Err(error) => {
                self.error(AgentCode::OpenFailed, error);
                return;
            }
        };
        let open_ms = crate::millis(opening.elapsed());
        // Each live node's context usage is read before the attach, which
        // takes its frames in one step with nothing awaited.
        tree.refresh_context().await;
        // What the store holds is read before the attach too, for the pages
        // that hold what they read while no session lived.
        let held: Vec<Held> = follows.iter().map(|follow| follow.holds.clone()).collect();
        let saved = self.saved_for(&held).await;
        let building = Instant::now();
        tree.attach(self.id, self.outbox.clone(), self.follows.clone());
        // In the step that attached it, with nothing awaited between: no
        // change falls between the attachment and the follows.
        let followed = follows.len();
        for Follow { value, holds } in follows {
            self.follows.register(&value);
            self.compare(value, &holds, saved.as_ref());
        }
        tracing::info!(
            event = "conversation.opened",
            conversation = %self.root,
            was_live,
            follows = followed,
            lock_wait_ms,
            open_ms,
            attach_ms = crate::millis(building.elapsed()),
            followed_nodes = self.follows.followed_nodes(),
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
    /// session took what it asks ([`handle`](Self::handle)).
    async fn dispatch(&self, tree: &Rc<Tree<H>>, frame: ClientFrame, kind: ClientFrameKind) -> bool {
        let session = tree.root().session();
        match frame {
            ClientFrame::Send(SendMessage {
                message_id,
                content,
                comments,
            }) => {
                // A message the session holds already is acknowledged before
                // its files are resolved, so a request sent again, after its
                // answer was lost, writes nothing a second time.
                if session.knows_turn(&message_id) {
                    let at = Self::shown_at(tree, &session).await;
                    self.send(ServerFrame::SendResult { message_id, at });
                    return true;
                }
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
                    schedule: None,
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
            ClientFrame::SteerNow { steer_id } => return session.steer_now(&steer_id),
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
                return session.send_queued_message(&message_id);
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
                return true;
            }
            // The agents' entries show what these stopped.
            ClientFrame::AbortSubagents {} => {
                tree.abort_children_of(tree.root().id(), Stop::User).await;
                return true;
            }
            ClientFrame::AbortSubagent { subagent_id } => {
                if tree.is_child_of(&subagent_id, tree.root().id()) {
                    tree.abort_child(&subagent_id, Stop::User).await;
                    return true;
                }
            }
            ClientFrame::Retry {} | ClientFrame::Resume {} => {
                return self.recover(session, kind).await;
            }
            // A subagent's context card compacts that child, the root's the root.
            ClientFrame::Compact { node } => match node {
                None => {
                    self.recover(session, kind).await;
                }
                Some(id) => match tree.node(&id) {
                    Some(node) => {
                        self.recover(node.session(), kind).await;
                    }
                    None => self.reject(kind, agent_ended()),
                },
            },
            ClientFrame::EditAndSend { request } => {
                let operation_id = request.operation_id.clone();
                let outcome = match self.edit(session, request).await {
                    Ok(turn_id) => EditOutcome::Accepted { turn_id },
                    Err(refusal) => EditOutcome::Rejected { refusal },
                };
                let accepted = matches!(outcome, EditOutcome::Accepted { .. });
                self.send(ServerFrame::EditResult {
                    operation_id,
                    outcome,
                });
                return accepted;
            }
            ClientFrame::Regenerate { request } => {
                let operation_id = request.operation_id.clone();
                let outcome = match regenerate(session, request).await {
                    Ok(turn_id) => EditOutcome::Accepted { turn_id },
                    Err(refusal) => EditOutcome::Rejected { refusal },
                };
                let accepted = matches!(outcome, EditOutcome::Accepted { .. });
                self.send(ServerFrame::EditResult {
                    operation_id,
                    outcome,
                });
                return accepted;
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
                match tree.shell_abort(&command_id).await {
                    Ok(()) => return true,
                    Err(error) => self.error(AgentCode::ShellStopFailed, error),
                }
            }
            ClientFrame::Open { .. }
            | ClientFrame::Close {}
            | ClientFrame::Follow(_)
            | ClientFrame::Unfollow { .. } => {
                unreachable!("open, close, follow and unfollow are handled before dispatch")
            }
        }
        false
    }
}

impl<H: HostResolver> Connection<H> {
    /// Retries or resumes the root's session, or compacts a node's,
    /// refusing while it is busy; a resume carries `reason` to the model.
    async fn recover(&self, session: &AgentSession, kind: ClientFrameKind) -> bool {
        if let Some(refusal) = busy(session) {
            self.reject(kind, refusal);
            return false;
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
                return false;
            }
        }
        let admitted = match kind {
            ClientFrameKind::Retry => session.retry(),
            ClientFrameKind::Resume => session.resume(None),
            _ => session.compact(),
        };
        match admitted {
            Ok(_) => true,
            Err(error) => {
                self.reject(kind, error.refusal());
                false
            }
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
                    comments: request.comments,
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
        comments: None,
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
