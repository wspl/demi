//! A conversation's side questions (`comments-and-ask.md` § Side questions):
//! each a node of the tree beside the root's children, never in the agent
//! directory. It starts from the root's history through the asked reply, a
//! Fork's seed, with the root's instructions, commands and model, so its
//! first request begins with what the root's next one would send and reads
//! it from the provider's cache; its first message holds the asked words, the
//! instruction that makes it a side question, and the question. It runs
//! beside the root's turn and never waits for it, closes once it is
//! quiescent, which delivers nothing to anyone, and opens again for a
//! follow-up or a Retry. The pages follow its transcript as a subagent's,
//! by its node.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use demi_agent_session::{SessionEvent, Settle, Subscription};
use demi_agent_store::{ClosePhase, NodeClose, NodeRecord, SideOf};
use demi_agent_tools::HostResolver;
use demi_agent_transcript::ForkEnd;
use demi_conversation_socket_protocol::{ServerFrame, index_u32, latest_page_start};
use demi_provider_common::ProviderRuntime;
use demi_shared_gates::Purpose;
use demi_shared_types::{NodeId, QueuedMessage, TurnId, UserContentBlock, markdown_quote};
use futures_util::future::LocalBoxFuture;
use tokio::sync::{Notify, watch};
use tokio_util::task::AbortOnDropHandle;

use super::{
    Tree, cut_path, note_activity,
    supervisor::OWNER_CLOSING,
    told::{self, Asked},
};
use crate::{
    Node,
    node::{self, NodeRole, NodeSpec, Origin},
    server::{AgentServer, commands::with_runtime_groups},
};

/// What makes a question a side question, between the asked words and the
/// question in its first message (`comments-and-ask.md` § Side questions).
pub const SIDE_INSTRUCTION: &str = "The user asks this aside from the conversation, about the words quoted above. Answer briefly, in a few sentences. Read and check what you need, but change nothing: write no file and run no command that changes the Host.";

/// Why a side question was not asked, followed up or retried.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AskError {
    /// The conversation's tree is not open.
    #[error("the conversation's tree is not open")]
    NotOpen,
    /// The asked block is not a completed text of the root's history.
    #[error("{0}")]
    Target(String),
    /// Another node of the conversation, or another side question, has the
    /// id.
    #[error("the id belongs to another side question or agent of the conversation")]
    Taken,
    /// The conversation has no side question of the id.
    #[error("no such side question")]
    Unknown,
    /// Its session refused, as a Retry with no failed turn to run again.
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    Failed(String),
}

/// What a closed side question's transcript ended with: its latest page,
/// which a page that attaches receives as its reset.
pub(crate) struct ClosedSide {
    asked_at: Option<demi_shared_types::Timestamp>,
    /// What it asked, which the conversation's agent learns of.
    asked: Option<Asked>,
    start: usize,
    length: usize,
    blocks: Vec<demi_shared_types::Block>,
    revision: u64,
}

impl ClosedSide {
    fn of(id: &NodeId, blocks: &[demi_shared_types::Block], revision: u64, of: Option<SideOf>) -> Self {
        let start = latest_page_start(blocks);
        Self {
            asked_at: of.as_ref().map(|of| of.asked_at),
            asked: of.map(|of| Asked::of(id, of, blocks, true)),
            start,
            length: blocks.len(),
            blocks: blocks[start..].to_vec(),
            revision,
        }
    }

    /// The frames that tell a page the side question is done: its
    /// transcript's latest page, idle, nothing pending.
    fn frames(&self, id: &NodeId) -> [ServerFrame; 4] {
        [
            ServerFrame::SubagentTranscriptReset {
                subagent_id: id.clone(),
                start: index_u32(self.start),
                length: index_u32(self.length),
                blocks: self.blocks.clone(),
                revision: self.revision,
                failures: None,
            },
            ServerFrame::Phase {
                subagent_id: Some(id.clone()),
                phase: demi_shared_types::SessionPhase::Idle,
            },
            ServerFrame::PendingCalls {
                subagent_id: Some(id.clone()),
                pending_calls: Vec::new(),
            },
            ServerFrame::Waiting {
                subagent_id: Some(id.clone()),
                waited_ms: None,
            },
        ]
    }
}

/// A live side question: its node, and what its supervision keeps.
pub(crate) struct Side<H: HostResolver> {
    node: Rc<Node<H>>,
    /// Set in the one step that decides it closes; a follow-up after that
    /// step waits for the close and opens it again.
    closing: Cell<bool>,
    /// The failure of an action that failed for good, which closes it.
    failure: RefCell<Option<String>>,
    /// Wakes its supervision when an action failed.
    wake: Rc<Notify>,
    /// Set once its close is complete.
    closed: watch::Sender<bool>,
    /// Its session's events as frames.
    events: RefCell<Option<Subscription>>,
    supervision: RefCell<Option<AbortOnDropHandle<()>>>,
}

impl<H: HostResolver> Side<H> {
    pub(crate) fn node(&self) -> &Rc<Node<H>> {
        &self.node
    }

    pub(super) fn stop_supervision(&self) {
        self.supervision.borrow_mut().take();
    }
}

/// The first message of the side question `of`: the asked words as a
/// quote, the instruction, then the question.
fn first_message(id: TurnId, of: &SideOf) -> QueuedMessage {
    let text = format!("{}\n\n{SIDE_INSTRUCTION}\n\n{}", markdown_quote(&of.quote), of.question);
    QueuedMessage {
        comments: Vec::new(),
        id,
        content: vec![UserContentBlock::Text { text }],
        automation: None,
    }
}

/// Whether two side questions ask the same of the same words, whenever.
fn asks_the_same(a: &SideOf, b: &SideOf) -> bool {
    (&a.block, a.start, a.end, &a.quote, &a.question) == (&b.block, b.start, b.end, &b.quote, &b.question)
}

impl<H: HostResolver> Tree<H> {
    /// The live side question `id`.
    pub(crate) fn side(&self, id: &NodeId) -> Option<Rc<Side<H>>> {
        self.sides.borrow().get(id).cloned()
    }

    /// Whether `id` is a side question's node, live or not: one that is
    /// asks nothing of the agent directory.
    pub(crate) fn is_side(&self, id: &NodeId) -> bool {
        self.sides.borrow().contains_key(id)
    }

    /// Asks the side question `id` about `of`, from the root's history
    /// through `of`'s block, in a task of the tree, so the start runs to its
    /// end even when the call is cancelled. The same question again under
    /// its id asks nothing more. Answers whether this call asked it.
    pub(crate) async fn ask(self: &Rc<Self>, id: NodeId, of: SideOf) -> Result<bool, AskError> {
        let tree = self.clone();
        let started = self.lifecycle.spawn_local(async move {
            let _turn = tree.starts.acquire(id.clone()).await;
            if tree.disposing.get() {
                return Err(AskError::Failed(OWNER_CLOSING.to_owned()));
            }
            let stored = tree.store.node(&id).await.map_err(|error| AskError::Failed(error.to_string()))?;
            if let Some(record) = stored {
                return match record.side {
                    Some(asked) if asks_the_same(&asked, &of) => Ok(false),
                    _ => Err(AskError::Taken),
                };
            }
            let server = tree.server.upgrade().ok_or_else(|| AskError::Failed("the agent server is gone".to_owned()))?;
            let root = tree.root.clone();
            // The history is the root's as the reply ended, captured in one
            // step; the root goes on with whatever it does.
            let mut seed = root
                .session()
                .prepare_fork(ForkEnd::Side(&of.block))
                .map_err(|error| AskError::Target(error.to_string()))?;
            // Its first message's turn is the side question's own id, which
            // names that message to a page whatever pages it holds.
            let turn = TurnId::try_from(id.as_str()).map_err(|error| AskError::Target(error.to_string()))?;
            seed.state.queue.push(first_message(turn, &of));
            let runtime = tree.side_runtime(&server, &seed.state.model).await?;
            let now = tree.clock.now();
            let record = NodeRecord {
                id: id.clone(),
                number: 0,
                parent: Some(root.id().clone()),
                description: String::new(),
                profile: None,
                instructions: None,
                context_limit: root.record().context_limit,
                round: 1,
                started_at: now,
                can_spawn_subagents: root.record().can_spawn_subagents,
                closed: None,
                delivered: false,
                forked_from: None,
                started_by: None,
                cwd: None,
                side: Some(SideOf { asked_at: now, ..of }),
            };
            let origin = Origin::Forked { record, checkpoint: seed };
            tree.start_side(&server, origin, runtime).await.map_err(AskError::Failed)?;
            Ok(true)
        });
        started.await.map_err(|error| AskError::Failed(format!("the side question did not start: {error}")))?
    }

    /// Sends the follow-up `text` to the side question `id`: its next turn,
    /// after its own history, opening it again when it closed.
    pub(crate) async fn ask_more(self: &Rc<Self>, id: NodeId, text: String) -> Result<(), AskError> {
        let tree = self.clone();
        let sent = self.lifecycle.spawn_local(async move {
            let _turn = tree.starts.acquire(id.clone()).await;
            let message = tree.text_message(text);
            if let Some(side) = tree.live_side(&id).await {
                return side
                    .node
                    .session()
                    .send_message(message)
                    .map(drop)
                    .map_err(|error| AskError::Refused(error.to_string()));
            }
            tree.reopen_side(&id, Some(message)).await.map(drop)
        });
        sent.await.map_err(|error| AskError::Failed(format!("the follow-up did not start: {error}")))?
    }

    /// Runs the side question `id`'s failed turn again, as a turn's Retry
    /// does, opening it again when it closed.
    pub(crate) async fn retry_side(self: &Rc<Self>, id: NodeId) -> Result<(), AskError> {
        let tree = self.clone();
        let retried = self.lifecycle.spawn_local(async move {
            let _turn = tree.starts.acquire(id.clone()).await;
            let side = match tree.live_side(&id).await {
                Some(side) => side,
                None => tree.reopen_side(&id, None).await?,
            };
            // Nothing awaits between the start and the Retry, so its
            // supervision has not looked at it yet.
            side.node
                .session()
                .retry()
                .map(drop)
                .map_err(|error| AskError::Refused(error.to_string()))
        });
        retried.await.map_err(|error| AskError::Failed(format!("the retry did not start: {error}")))?
    }

    /// The live side question `id` that takes input; once one that is
    /// closing has closed, none.
    async fn live_side(&self, id: &NodeId) -> Option<Rc<Side<H>>> {
        let side = self.side(id)?;
        if !side.closing.get() {
            return Some(side);
        }
        let mut closed = side.closed.subscribe();
        // The sender lives in the side this call holds.
        let _ = closed.wait_for(|closed| *closed).await;
        None
    }

    /// Makes the closed side question `id` live again, a new round with
    /// `message` queued, if any, and starts it from its checkpoint.
    async fn reopen_side(self: &Rc<Self>, id: &NodeId, message: Option<QueuedMessage>) -> Result<Rc<Side<H>>, AskError> {
        if self.disposing.get() {
            return Err(AskError::Failed(OWNER_CLOSING.to_owned()));
        }
        let failed = |error: demi_agent_store::StoreError| AskError::Failed(error.to_string());
        let record = self
            .store
            .node(id)
            .await
            .map_err(failed)?
            .filter(|record| record.side.is_some())
            .ok_or(AskError::Unknown)?;
        let server = self.server.upgrade().ok_or_else(|| AskError::Failed("the agent server is gone".to_owned()))?;
        if record.closed.is_some() {
            self.store
                .reopen_node(id, record.round + 1, self.clock.now(), message)
                .await
                .map_err(failed)?;
        }
        let origin = Origin::stored(self.store.as_ref(), id).await.map_err(failed)?.ok_or(AskError::Unknown)?;
        // Its own model: a follow-up keeps it when the conversation's
        // changes.
        let runtime = self.side_runtime(&server, origin.model()).await?;
        self.start_side(&server, origin, runtime).await.map_err(AskError::Failed)
    }

    /// A side question's own provider runtime for `model`, which the product
    /// builds for the entry: never a fork of the root's, which waits while a
    /// request of the root's holds it, and a side never waits for the
    /// root's turn.
    async fn side_runtime(
        &self,
        server: &AgentServer<H>,
        model: &demi_shared_types::ModelSelection,
    ) -> Result<Box<dyn ProviderRuntime>, AskError> {
        server
            .deps
            .providers
            .runtime(self.root.id(), model)
            .await
            .map_err(|error| AskError::Failed(error.to_string()))
    }

    /// Brings back the side questions the store holds live, which a
    /// restart or a disposal interrupted: each records its interruption,
    /// as the root does, and closes once quiescent.
    pub(super) fn restore_sides(self: &Rc<Self>) -> LocalBoxFuture<'_, ()> {
        Box::pin(async move {
            let Some(server) = self.server.upgrade() else {
                return;
            };
            let records = match self.store.sides().await {
                Ok(records) => records,
                Err(error) => {
                    self.report(format!("the side questions were not restored: {error}"));
                    return;
                }
            };
            for record in records {
                if self.disposing.get() {
                    return;
                }
                if record.closed.is_some() {
                    match self.store.session_store(&record.id).load().await {
                        Ok(Some(checkpoint)) => {
                            let closed = ClosedSide::of(&record.id, &checkpoint.transcript, 0, record.side.clone());
                            self.closed_sides.borrow_mut().insert(record.id.clone(), closed);
                        }
                        Ok(None) => {}
                        Err(error) => self.report(format!("side question {} was not read: {error}", record.id)),
                    }
                    continue;
                }
                let _turn = self.starts.acquire(record.id.clone()).await;
                if self.is_side(&record.id) {
                    continue;
                }
                let origin = match Origin::stored(self.store.as_ref(), &record.id).await {
                    Ok(Some(origin)) => origin,
                    Ok(None) => continue,
                    Err(error) => {
                        self.report(format!("side question {} was not restored: {error}", record.id));
                        continue;
                    }
                };
                let runtime = match self.side_runtime(&server, origin.model()).await {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        self.report(format!("side question {} was not restored: {error}", record.id));
                        continue;
                    }
                };
                if let Err(error) = self.start_side(&server, origin, runtime).await {
                    self.report(format!("side question {} was not restored: {error}", record.id));
                }
            }
        })
    }

    /// The side question's node from the assembly, with the root's
    /// instructions and commands and no preamble, so its system prompt and
    /// tools are the root's; its frames from then on, what it has yet to
    /// run, and its supervision.
    async fn start_side(
        self: &Rc<Self>,
        server: &Rc<AgentServer<H>>,
        origin: Origin,
        runtime: Box<dyn ProviderRuntime>,
    ) -> Result<Rc<Side<H>>, String> {
        let deps = &server.deps;
        let activity = self.admission.enter(Purpose::Demand).await;
        let id = origin.record().id.clone();
        let inherited = self.root.inherited_commands().clone();
        let commands = with_runtime_groups(&inherited, server, origin.record().can_spawn_subagents)
            .map_err(|error| error.to_string())?;
        let assembled = node::assemble(NodeSpec {
            origin,
            role: NodeRole::Side,
            root: self.root.id().clone(),
            server: Rc::downgrade(server),
            runtime,
            providers: deps.providers.clone(),
            hosts: deps.hosts.clone(),
            instructions: self.root.instructions().clone(),
            guide: deps.guide.clone(),
            preamble: None,
            context: deps.context.clone(),
            inherited,
            commands: Rc::new(commands),
            store: self.store.clone(),
            shells: deps.shells.clone(),
            feed: self.live.feed(Some(id.clone())),
            admission: self.admission.clone(),
            ids: deps.ids.clone(),
            clock: deps.clock.clone(),
            config: deps.config.session,
            interval_floor_ms: deps.config.interval_floor_ms,
            call_window_ms: deps.config.call_window_ms,
        })
        .await
        .map_err(|error| error.to_string())?;
        let side = self.attach_side(assembled.node);
        if let Some(continuation) = assembled.continuation
            && let Err(error) = side.node.continue_from(continuation).await
        {
            self.report(format!("side question {id} did not save its start: {error}"));
        }
        drop(activity);
        self.supervise_side(&side);
        Ok(side)
    }

    /// Registers a live side question, sends its frames, and forwards its
    /// session's changes from then on.
    fn attach_side(&self, node: Node<H>) -> Rc<Side<H>> {
        let node = Rc::new(node);
        let side = Rc::new_cyclic(|side: &Weak<Side<H>>| {
            let events = node.session().subscribe({
                let side = side.clone();
                let sink = self.sink.clone();
                let id = node.id().clone();
                let paths = self.paths.clone();
                let activity = self.activity.clone();
                move |event| {
                    cut_path(&paths, event);
                    note_activity(&activity, event);
                    match event {
                        SessionEvent::TranscriptChanged { patches, revision } => {
                            sink.emit(ServerFrame::SubagentTranscriptPatch {
                                subagent_id: id.clone(),
                                patches: patches.clone(),
                                revision: *revision,
                                failures: None,
                            });
                        }
                        SessionEvent::PendingCallsChanged { pending_calls } => sink.emit(ServerFrame::PendingCalls {
                            subagent_id: Some(id.clone()),
                            pending_calls: pending_calls.clone(),
                        }),
                        SessionEvent::WaitingChanged { waited_ms } => sink.emit(ServerFrame::Waiting {
                            subagent_id: Some(id.clone()),
                            waited_ms: *waited_ms,
                        }),
                        SessionEvent::PhaseChanged { phase } => sink.emit(ServerFrame::Phase {
                            subagent_id: Some(id.clone()),
                            phase: *phase,
                        }),
                        SessionEvent::ContextUsageChanged { usage } => sink.emit(ServerFrame::ContextUsage {
                            node: id.clone(),
                            usage: Some(usage.clone()),
                        }),
                        SessionEvent::ActionFailed { report } => {
                            if let Some(side) = side.upgrade() {
                                side.failure.replace(Some(report.message.clone()));
                                side.wake.notify_one();
                            }
                        }
                        _ => {}
                    }
                }
            });
            Side {
                node: node.clone(),
                closing: Cell::new(false),
                failure: RefCell::new(None),
                wake: Rc::new(Notify::new()),
                closed: watch::Sender::new(false),
                events: RefCell::new(Some(events)),
                supervision: RefCell::new(None),
            }
        });
        self.closed_sides.borrow_mut().remove(node.id());
        self.sides.borrow_mut().insert(node.id().clone(), side.clone());
        self.bump();
        for frame in side_frames(&side) {
            self.sink.emit(frame);
        }
        side
    }

    /// Each live side question's transcript, phase, calls being written and
    /// wait for the model: what a connection that attaches or syncs
    /// receives after the children's.
    /// The block of the side questions source for the request `node` is
    /// about to make, given the source's blocks `seen` since the last
    /// compaction (`comments-and-ask.md` § What the agent learns): only
    /// the root's, only when the request follows the user's new message or
    /// steer, and only the questions asked since the agent last heard of
    /// them, about replies still in its history, in the order asked.
    pub fn side_questions_context(&self, node: &NodeId, seen: &[&str]) -> Option<String> {
        if node != self.root.id() {
            return None;
        }
        let history = self.root.session().transcript().blocks;
        if !told::follows_input(&history) {
            return None;
        }
        let in_history = |of: &SideOf| history.iter().any(|block| block.id() == &of.block);
        let mut asked: Vec<Asked> = self
            .sides
            .borrow()
            .values()
            .filter_map(|side| {
                let of = side.node.record().side?;
                let session = side.node.session();
                let status = session.status();
                let ended = status.settle == Settle::Settled && !status.input;
                in_history(&of).then(|| Asked::of(side.node.id(), of, &session.transcript().blocks, ended))
            })
            .collect();
        asked.extend(
            self.closed_sides
                .borrow()
                .values()
                .filter_map(|closed| closed.asked.as_ref().filter(|asked| in_history(&asked.of)).cloned()),
        );
        asked.sort_by_key(|asked| asked.of.asked_at);
        told::untold(&asked, seen)
    }

    /// Every side question's state, the closed ones' as they ended, in the
    /// order they were asked: what a connection that attaches or syncs
    /// receives after the children's, so a side question that finished while
    /// the page was away shows done and whole.
    pub(super) fn replay_sides(&self) -> Vec<ServerFrame> {
        let mut frames: Vec<(Option<demi_shared_types::Timestamp>, Vec<ServerFrame>)> = self
            .sides
            .borrow()
            .values()
            .map(|side| (side.node.record().side.map(|of| of.asked_at), side_frames(side).to_vec()))
            .collect();
        frames.extend(
            self.closed_sides
                .borrow()
                .iter()
                .map(|(id, closed)| (closed.asked_at, closed.frames(id).to_vec())),
        );
        frames.sort_by_key(|(asked_at, _)| *asked_at);
        frames.into_iter().flat_map(|(_, frames)| frames).collect()
    }

    /// Whether every live side question does nothing by itself.
    pub(super) fn sides_quiescent(&self) -> bool {
        self.sides.borrow().is_empty()
    }

    fn supervise_side(self: &Rc<Self>, side: &Rc<Side<H>>) {
        let task = tokio::task::spawn_local(side_supervision(Rc::downgrade(self), Rc::downgrade(side)));
        *side.supervision.borrow_mut() = Some(AbortOnDropHandle::new(task));
    }

    /// Looks at `side` and decides, in one step with no await, whether it
    /// closes: once an action failed for good, or once it is quiescent, as
    /// a child is; a follow-up accepted before this step keeps it open.
    /// Answers the failure it closes with, if any.
    fn decide_side(&self, side: &Side<H>) -> Option<Option<String>> {
        if side.closing.get() || self.disposing.get() {
            return None;
        }
        let failure = side.failure.borrow().clone();
        if failure.is_none() {
            let status = side.node.session().status();
            let quiescent = status.settle == Settle::Settled && !status.input && !status.commands;
            if !quiescent {
                return None;
            }
        }
        side.closing.set(true);
        Some(failure)
    }

    /// Closes `side`: its session saves its final checkpoint, the close is
    /// committed, and it leaves the live ones. Its completion reaches no
    /// one.
    fn begin_close_side(self: &Rc<Self>, side: Rc<Side<H>>, failure: Option<String>) {
        let tree = self.clone();
        self.lifecycle.spawn_local(async move {
            let id = side.node.id().clone();
            if let Err(error) = side.node.session().dispose().await {
                tree.report(format!("side question {id} did not save its final checkpoint: {error}"));
            }
            let ended = {
                let transcript = side.node.session().transcript();
                ClosedSide::of(&id, &transcript.blocks, transcript.version.revision, side.node.record().side)
            };
            side.events.borrow_mut().take();
            let phase = match failure {
                Some(failure) => ClosePhase::Error { failure },
                None => ClosePhase::Completed { result: String::new() },
            };
            let close = NodeClose { phase, at: tree.clock.now() };
            if let Err(error) = tree.store.close_node(&id, close).await {
                tree.report(format!("side question {id} did not close: {error}"));
            }
            tree.sides.borrow_mut().remove(&id);
            tree.closed_sides.borrow_mut().insert(id.clone(), ended);
            side.closed.send_replace(true);
            tree.bump();
        });
    }

    /// Stops what every live side question runs and ends their shells, as
    /// a transition of the Host the conversation cannot work without does
    /// (`Tree::interrupt`); each closes once quiescent.
    pub(super) async fn interrupt_sides(&self, hold: &str) {
        let sides: Vec<Rc<Side<H>>> = self.sides.borrow().values().cloned().collect();
        for side in sides {
            side.node.session().stop_running(hold).await;
            side.node.end_shells().await;
        }
    }

    /// Lets go of every live side question, each saving its final
    /// checkpoint and staying live in the store for the next open.
    pub(super) async fn dispose_sides(&self) {
        let sides: Vec<Rc<Side<H>>> = self.sides.borrow_mut().drain().map(|(_, side)| side).collect();
        for side in &sides {
            side.stop_supervision();
        }
        for side in sides {
            if let Err(error) = side.node.session().dispose().await {
                tracing::error!(node = %side.node.id(), %error, "the final checkpoint was not saved");
            }
        }
    }
}

/// The supervision of one side question: waits for any change of its
/// session's status or of its failure, and closes it once
/// [`Tree::decide_side`] says so.
async fn side_supervision<H: HostResolver>(tree: Weak<Tree<H>>, side: Weak<Side<H>>) {
    let (mut status, wake) = {
        let Some(live) = side.upgrade() else {
            return;
        };
        (live.node.session().status_watch(), live.wake.clone())
    };
    loop {
        status.mark_unchanged();
        {
            let (Some(live_tree), Some(live_side)) = (tree.upgrade(), side.upgrade()) else {
                return;
            };
            if let Some(failure) = live_tree.decide_side(&live_side) {
                live_tree.begin_close_side(live_side, failure);
                return;
            }
        }
        tokio::select! {
            changed = status.changed() => if changed.is_err() { return },
            () = wake.notified() => {}
        }
    }
}

/// A side question's transcript from its latest page, its phase, the calls
/// it is writing and its wait for the model.
fn side_frames<H: HostResolver>(side: &Side<H>) -> [ServerFrame; 4] {
    let session = side.node.session();
    let transcript = session.transcript();
    let id = side.node.id().clone();
    let start = latest_page_start(&transcript.blocks);
    [
        ServerFrame::SubagentTranscriptReset {
            subagent_id: id.clone(),
            start: index_u32(start),
            length: index_u32(transcript.blocks.len()),
            blocks: transcript.blocks[start..].to_vec(),
            revision: transcript.version.revision,
            failures: None,
        },
        ServerFrame::Phase {
            subagent_id: Some(id.clone()),
            phase: session.phase(),
        },
        ServerFrame::PendingCalls {
            subagent_id: Some(id.clone()),
            pending_calls: session.pending_calls(),
        },
        ServerFrame::Waiting {
            subagent_id: Some(id),
            waited_ms: session.waited_ms(),
        },
    ]
}
