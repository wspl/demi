//! One conversation socket's frame handling (`runtime.md` § Frame protocol):
//! the backend decodes each client frame, resolves its content, and hands it
//! to [`Connection::handle`], one frame at a time in arrival order. Every
//! frame for the connection, a reply or an event, goes through its one
//! bounded outbox in causal order; [`FrameRx`] is the outbox's receiving end.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use demi_agent_protocol::{
    ClientContent, ClientFrame, ClientFrameKind, EditOutcome, EditRequest, ModelSwitchApply,
    ServerFrame, SteerOutcome,
};
use demi_core::{BlockId, ModelSelection, NodeId, SessionPhase, TurnId};
use tokio::sync::mpsc::{self, error::TrySendError};

use super::{
    AgentServer,
    content::{ContentError, ContentResolver, resolve_edit, resolve_message},
    tree::Tree,
};
use crate::{
    AgentHarness, AgentSession,
    session::{EditCheck, EditSubmission, accepted, edit_digest},
    tools,
};

/// What a refusal says when the connection has no session.
const NO_SESSION: &str = "No session is open";
/// What a steer's and a model change's refusal says then.
const NO_SESSION_HERE: &str = "No session is open on this connection";
/// What a queued message's steer says when the message is not queued.
const NOT_QUEUED: &str = "Queued message not found";

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
/// backend names the conversation and its working directory, and the client
/// never sends them. Dropping it detaches it; the tree's turns keep running.
pub struct Connection<H: AgentHarness> {
    id: u64,
    root: NodeId,
    cwd: String,
    server: Rc<AgentServer<H>>,
    resolver: Rc<dyn ContentResolver>,
    outbox: Rc<Outbox>,
}

impl<H: AgentHarness> Connection<H> {
    pub(super) fn new(
        server: Rc<AgentServer<H>>,
        id: u64,
        root: NodeId,
        cwd: String,
        resolver: Rc<dyn ContentResolver>,
    ) -> (Self, FrameRx) {
        let (sender, frames) = mpsc::channel(server.deps.config.outbox_frames);
        let lagged = Rc::new(Cell::new(false));
        let outbox = Rc::new(Outbox {
            sender: RefCell::new(Some(sender)),
            lagged: lagged.clone(),
        });
        let connection = Self {
            id,
            root,
            cwd,
            server,
            resolver,
            outbox,
        };
        (connection, FrameRx { frames, lagged })
    }

    /// Handles one decoded frame to its end; a frame whose handling waits,
    /// such as a send whose uploads are being written to the Host or an
    /// `abort` waiting for the stop to be recorded, delays the frames behind
    /// it. A failure is answered as a frame, never returned.
    pub async fn handle(&self, frame: ClientFrame) {
        match frame {
            ClientFrame::Open { model } => self.open(model).await,
            ClientFrame::Close {} => self.close().await,
            frame => match self.attached() {
                Some(tree) => self.dispatch(&tree, frame).await,
                None => self.refuse(frame),
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

    fn reject(&self, command: ClientFrameKind, reason: impl Into<String>) {
        self.send(ServerFrame::Rejected {
            command,
            reason: reason.into(),
        });
    }

    fn error(&self, error: impl std::fmt::Display) {
        self.send(ServerFrame::Error {
            message: error.to_string(),
            code: None,
            diagnostics: None,
        });
    }

    fn content_error(&self, error: ContentError) {
        self.send(ServerFrame::Error {
            message: error.message,
            code: error.code,
            diagnostics: None,
        });
    }

    fn steer_result(&self, steer_id: BlockId, outcome: Result<(), String>) {
        let outcome = match outcome {
            Ok(()) => SteerOutcome::Accepted,
            Err(reason) => SteerOutcome::Rejected { reason },
        };
        self.send(ServerFrame::SteerResult { steer_id, outcome });
    }

    /// Attaches the connection to the conversation's tree, restoring the
    /// tree when it is not live; a second `open` on one connection is
    /// refused. Opening a tree another connection is attached to takes it
    /// over.
    async fn open(&self, model: ModelSelection) {
        if self.attached().is_some() {
            self.reject(
                ClientFrameKind::Open,
                "A session is already open on this connection",
            );
            return;
        }
        let _turn = self.server.opening.acquire(self.root.clone()).await;
        let (tree, continuation) = match self.server.tree(&self.root) {
            Some(tree) => {
                if let Err(error) = tree
                    .align_model(&self.server, model, ModelSwitchApply::NextTurn)
                    .await
                {
                    self.error(error);
                    return;
                }
                (tree, None)
            }
            None => match Tree::open(&self.server, &self.root, &self.cwd, model).await {
                Ok(opened) => opened,
                Err(error) => {
                    self.error(error);
                    return;
                }
            },
        };
        tree.attach(self.id, self.outbox.clone());
        if let Some(continuation) = continuation
            && let Err(error) = tree.continue_restored(continuation).await
        {
            self.error(error);
        }
    }

    /// Disposes the tree this connection is attached to, then answers
    /// `closed`, even when nothing was attached.
    async fn close(&self) {
        if self.attached().is_some() {
            let _turn = self.server.opening.acquire(self.root.clone()).await;
            // Another connection may have taken the tree over meanwhile.
            if let Some(tree) = self.attached() {
                self.server.dispose_tree(&self.root, &tree).await;
            }
        }
        self.send(ServerFrame::Closed);
    }

    /// The answer to a frame that needs a session when none is open.
    fn refuse(&self, frame: ClientFrame) {
        match frame {
            ClientFrame::Steer { steer_id, .. }
            | ClientFrame::SteerQueuedMessage { steer_id, .. } => {
                self.send(ServerFrame::SteerResult {
                    steer_id,
                    outcome: SteerOutcome::Rejected {
                        reason: NO_SESSION_HERE.to_owned(),
                    },
                });
            }
            ClientFrame::SetProvider { .. } => {
                self.reject(ClientFrameKind::SetProvider, NO_SESSION_HERE)
            }
            ClientFrame::CancelPendingSteer { .. }
            | ClientFrame::AbortSubagents {}
            | ClientFrame::AbortSubagent { .. } => {}
            frame => self.reject(frame.kind(), NO_SESSION),
        }
    }

    async fn dispatch(&self, tree: &Rc<Tree<H>>, frame: ClientFrame) {
        let session = tree.root().session();
        let kind = frame.kind();
        match frame {
            ClientFrame::Send {
                message_id,
                content,
            } => {
                let content = match resolve_message(&*self.resolver, content).await {
                    Ok(content) => content,
                    Err(error) => return self.content_error(error),
                };
                // The session reports the action's course as events; the
                // handle is not needed.
                if let Err(error) = session.send(content, message_id) {
                    self.reject(kind, error.to_string());
                }
            }
            ClientFrame::Steer { steer_id, content } => {
                let outcome = match resolve_message(&*self.resolver, content).await {
                    Ok(content) => session
                        .steer(content, steer_id.clone())
                        .map_err(|error| error.to_string()),
                    Err(error) => Err(error.message),
                };
                self.steer_result(steer_id, outcome);
            }
            ClientFrame::SteerQueuedMessage {
                message_id,
                steer_id,
            } => {
                let outcome = match session.steer_queued_message(&message_id, steer_id.clone()) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err(NOT_QUEUED.to_owned()),
                    Err(error) => Err(error.to_string()),
                };
                self.steer_result(steer_id, outcome);
            }
            ClientFrame::CancelPendingSteer { steer_id } => {
                session.cancel_pending_steer(&steer_id);
            }
            ClientFrame::DequeueMessage { message_id } => {
                session.dequeue_message(&message_id);
            }
            ClientFrame::SendQueuedMessage { message_id } => {
                session.send_queued_message(&message_id);
            }
            ClientFrame::ClearMessageQueue {} => {
                session.clear_message_queue();
            }
            ClientFrame::SetProvider { model, apply } => {
                let apply = apply.unwrap_or(ModelSwitchApply::NextTurn);
                if let Err(error) = tree.align_model(&self.server, model, apply).await {
                    self.error(error);
                }
            }
            ClientFrame::Abort {} => {
                let result = session.abort().await;
                self.send(ServerFrame::AbortResult { result });
            }
            ClientFrame::SyncTranscript {} => tree.sync(self.id),
            // The `closed` frames report what these stopped.
            ClientFrame::AbortSubagents {} => tree.abort_children_of(tree.root().id()).await,
            ClientFrame::AbortSubagent { subagent_id } => {
                if tree.is_child_of(&subagent_id, tree.root().id()) {
                    tree.abort_child(&subagent_id).await;
                }
            }
            ClientFrame::Retry {} | ClientFrame::Resume {} | ClientFrame::Compact {} => {
                let phase = session.phase();
                if phase != SessionPhase::Idle {
                    self.reject(kind, format!("Session is busy ({phase})"));
                    return;
                }
                let admitted = match kind {
                    ClientFrameKind::Retry => session.retry(),
                    ClientFrameKind::Resume => session.resume(),
                    _ => session.compact(),
                };
                if let Err(error) = admitted {
                    self.reject(kind, error.to_string());
                }
            }
            ClientFrame::EditAndSend { request } => {
                let operation_id = request.operation_id.clone();
                let outcome = match self.edit(session, request).await {
                    Ok(turn_id) => EditOutcome::Accepted { turn_id },
                    Err(reason) => EditOutcome::Rejected { reason },
                };
                self.send(ServerFrame::EditResult {
                    operation_id,
                    outcome,
                });
            }
            ClientFrame::ShellWrite { command_id, stdin } => {
                match tree.root().shell_write(&command_id, stdin).await {
                    Ok(status) => {
                        self.send(tools::shell_output(&status));
                        self.send(ServerFrame::ShellWriteResult { command_id });
                    }
                    Err(error) => self.error(error),
                }
            }
            ClientFrame::ShellAbort { command_id } => {
                match tree.root().shell_abort(&command_id).await {
                    Ok(status) => self.send(tools::shell_output(&status)),
                    Err(error) => self.error(error),
                }
            }
            ClientFrame::Open { .. } | ClientFrame::Close {} => {
                unreachable!("open and close are handled before dispatch")
            }
        }
    }
}

impl<H: AgentHarness> Connection<H> {
    /// An edit's acceptance (`message-editing.md` § Commit and idempotency):
    /// a repeated request is answered from its receipt, or shares the
    /// acceptance in flight, before any of its files is resolved; a new one
    /// resolves its content and returns once the replacement is durable.
    async fn edit(
        &self,
        session: &AgentSession,
        request: EditRequest<ClientContent>,
    ) -> Result<TurnId, String> {
        let digest = edit_digest(&request);
        let check = session
            .check_edit(&request.operation_id, &digest, &request.version)
            .map_err(|error| error.to_string())?;
        let receipt = match check {
            EditCheck::Accepted(receipt) => receipt,
            EditCheck::InFlight(acceptance) => accepted(acceptance)
                .await
                .map_err(|error| error.to_string())?,
            EditCheck::Proceed => {
                let content = resolve_edit(&*self.resolver, request.content)
                    .await
                    .map_err(|error| error.message)?;
                let submission = EditSubmission {
                    operation_id: request.operation_id,
                    target: request.target_block_id,
                    version: request.version,
                    content,
                    digest,
                };
                session
                    .edit_and_send(submission)
                    .await
                    .map_err(|error| error.to_string())?
            }
        };
        Ok(receipt.turn_id)
    }
}

impl<H: AgentHarness> Drop for Connection<H> {
    fn drop(&mut self) {
        self.detach();
    }
}
