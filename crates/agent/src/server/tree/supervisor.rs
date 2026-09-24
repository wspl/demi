//! The supervisor operations of a conversation's tree (`subagents.md`): the
//! agent directory of its live nodes; starting children (spawn, resume and
//! restore) and closing them (a natural completion, a failure or an abort);
//! delivering their completions; messages between agents; and the `list`
//! and `show` reads. Every node supervises its own children the same way;
//! only the root's lifecycle is its client's.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use demi_agent_protocol::{JobPhase, ServerFrame, SubagentEvent, TranscriptPatch};
use demi_core::{
    AgentMessage, AgentMessageEvent, Block, BlockId, CompletionId, CompletionOutcome, NodeId,
    QueuedMessage, Sender, ToolCallBlock, ToolCallStatus, TurnId, UserContentBlock,
};
use demi_gates::Purpose;
use demi_shell::{RpcError, RpcPort};
use futures_util::future::LocalBoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};
use tokio_util::task::AbortOnDropHandle;

use super::Tree;
use crate::{
    AgentHarness, AgentSession, Node,
    node::{self, NodeRole, NodeSpec, Prompt},
    server::commands::with_agent_group,
    session::{AgentMessageError, Execution, SessionEvent, Settle, Subscription},
    store::{ClosePhase, NodeClose, NodeRecord},
};

/// The most live children one node has at once.
const MAX_LIVE_CHILDREN: usize = 8;
/// The bound of a completion's result and of `show`'s last assistant text,
/// in UTF-8 bytes.
const RESULT_MAX_BYTES: usize = 32 * 1024;
/// The most recent tool calls `show` lists.
const SHOW_RECENT_TOOLS: usize = 8;
/// Not a profile name: omitting `--profile` inherits the parent.
pub(crate) const INHERIT_PROFILE: &str = "default";
const INHERIT_LABEL: &str = "(inherit)";
const OWNER_CLOSING: &str = "owner session is closing";

/// A live child: its node, and what its supervision keeps about it.
pub(crate) struct Child<H: AgentHarness> {
    node: Rc<Node<H>>,
    /// When it joined the tree, which orders children of one round.
    order: u64,
    /// Set in the one step that decides the child closes; a message sent to
    /// it after that step is refused.
    closing: Cell<bool>,
    /// The failure of an action that failed for good, which closes it.
    failure: RefCell<Option<String>>,
    /// Wakes its supervision when an action failed.
    wake: Rc<Notify>,
    /// Set once its close is complete.
    closed: watch::Sender<bool>,
    telemetry: RefCell<Telemetry>,
    /// Its session's events as frames and telemetry.
    events: RefCell<Option<Subscription>>,
    supervision: RefCell<Option<AbortOnDropHandle<()>>>,
}

impl<H: AgentHarness> Child<H> {
    pub(crate) fn node(&self) -> &Rc<Node<H>> {
        &self.node
    }

    fn id(&self) -> &NodeId {
        self.node.id()
    }

    fn parent(&self) -> &NodeId {
        self.node
            .record()
            .parent
            .as_ref()
            .expect("a child's record names its parent")
    }

    /// Its place among its siblings: by spawn time, then as it joined.
    fn rank(&self) -> (u64, u64) {
        (self.node.record().round, self.order)
    }

    pub(super) fn stop_supervision(&self) {
        self.supervision.borrow_mut().take();
    }
}

/// What a start does, as its reservation records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum StartInput {
    Spawn {
        prompt: String,
        #[serde(deserialize_with = "Option::deserialize")]
        profile_name: Option<String>,
        description: String,
        is_spawn_forbidden: bool,
    },
    Resume {
        id: NodeId,
        message: String,
    },
}

/// A start's immutable reservation in the owner's command storage, at
/// `agent.start.<request id>` (`subagents.md` § Model-facing surface).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartReceipt {
    input: StartInput,
    node_id: NodeId,
    /// The round the start begins.
    spawned_at: u64,
}

/// How a child closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloseKind {
    Completed,
    Aborted,
    Error,
}

/// What a supervision finds when it looks at its child.
enum Decision {
    Wait,
    Close(CloseKind),
    /// Someone else closes the child, or the tree is being disposed.
    Stop,
}

impl<H: AgentHarness> Tree<H> {
    pub(crate) fn child(&self, id: &NodeId) -> Option<Rc<Child<H>>> {
        self.children.borrow().get(id).cloned()
    }

    fn is_live(&self, id: &NodeId) -> bool {
        self.children.borrow().contains_key(id)
    }

    /// The live children of `owner`, in spawn order.
    fn children_of(&self, owner: &NodeId) -> Vec<Rc<Child<H>>> {
        let mut children: Vec<Rc<Child<H>>> = self
            .children
            .borrow()
            .values()
            .filter(|child| child.parent() == owner)
            .cloned()
            .collect();
        children.sort_by_key(|child| child.rank());
        children
    }

    /// Whether `child` is a live child of `owner`.
    pub(crate) fn is_child_of(&self, child: &NodeId, owner: &NodeId) -> bool {
        self.child(child)
            .is_some_and(|child| child.parent() == owner)
    }

    /// The node that owns a start: the caller, live and not closing.
    fn owner(&self, caller: &NodeId) -> Result<Rc<Node<H>>, String> {
        if self.disposing.get() {
            return Err(OWNER_CLOSING.to_owned());
        }
        if caller == self.root.id() {
            return Ok(self.root.clone());
        }
        match self.child(caller) {
            Some(child) if !child.closing.get() => Ok(child.node.clone()),
            Some(_) => Err(OWNER_CLOSING.to_owned()),
            None => Err("this session is not in the agent directory".to_owned()),
        }
    }

    fn profile(&self, name: Option<&str>) -> Result<Option<&crate::Profile>, String> {
        let Some(name) = name else {
            return Ok(None);
        };
        if let Some(profile) = self.profiles.iter().find(|profile| profile.name == name) {
            return Ok(Some(profile));
        }
        let names: Vec<&str> = self
            .profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect();
        let available = if names.is_empty() {
            "none; omit --profile to inherit the parent".to_owned()
        } else {
            names.join(", ")
        };
        Err(format!(
            "unknown profile \"{name}\" (available: {available})"
        ))
    }

    fn check_capacity(&self, owner: &NodeId) -> Result<(), String> {
        if self.children_of(owner).len() >= MAX_LIVE_CHILDREN {
            return Err(format!(
                "at most {MAX_LIVE_CHILDREN} running subagents per session; abort one or wait for a result"
            ));
        }
        Ok(())
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.clock.now().as_millisecond()).unwrap_or(0)
    }

    /// Starts a child of `caller` for `demi agent spawn` or `resume`
    /// (`subagents.md` § Creation command ownership): reserves the request
    /// in the caller's command storage through `port`, then creates or
    /// reopens the child. The start runs in a task of the tree, so once its
    /// reservation is committed it runs to its end even when the call is
    /// cancelled; a reservation the cancelled call could not commit starts
    /// nothing. Starts of one owner take turns.
    pub(crate) async fn start(
        self: &Rc<Self>,
        caller: &NodeId,
        input: StartInput,
        request: String,
        port: &RpcPort,
    ) -> Result<NodeId, String> {
        let tree = self.clone();
        let caller = caller.clone();
        let port = port.clone();
        let started = self.lifecycle.spawn_local(async move {
            let _turn = tree.starts.acquire(caller.clone()).await;
            let owner = tree.owner(&caller)?;
            let _starting = Starting::new(&tree, &caller);
            let _lease = owner
                .lifecycle()
                .try_enter(Purpose::Maintenance)
                .ok_or("Cannot change children while a transcript edit is being prepared")?;
            let receipt = tree.reserve(&input, &request, &port).await?;
            tree.finish_start(&owner, receipt).await
        });
        started
            .await
            .map_err(|error| format!("the start of a subagent failed: {error}"))?
    }

    /// The start's reservation: the one already committed for `request`, or
    /// a new one.
    async fn reserve(
        &self,
        input: &StartInput,
        request: &str,
        port: &RpcPort,
    ) -> Result<StartReceipt, String> {
        // Computed before the read, which runs again after a conflict and
        // does no other IO.
        let fresh = match input {
            StartInput::Spawn { .. } => StartReceipt {
                input: input.clone(),
                node_id: self.new_node_id(),
                spawned_at: self.now_ms(),
            },
            StartInput::Resume { id, .. } => {
                let previous = self
                    .store
                    .node(id)
                    .await
                    .map_err(|error| error.to_string())?;
                StartReceipt {
                    input: input.clone(),
                    node_id: id.clone(),
                    // A round strictly newer than the previous one.
                    spawned_at: self
                        .now_ms()
                        .max(previous.map_or(0, |record| record.round) + 1),
                }
            }
        };
        let key = format!("agent.start.{request}");
        port.update(&key, |current: Option<StartReceipt>| match current {
            Some(existing) if &existing.input == input => Ok(existing),
            Some(_) => Err(RpcError::Failed(
                "request-id already belongs to different agent arguments".to_owned(),
            )),
            None => Ok(fresh.clone()),
        })
        .await
        .map_err(|error| error.to_string())
    }

    /// Finishes a reserved start: a retry returns the child the reservation
    /// made, restoring it when it is live in the store but not in the tree;
    /// otherwise the child is spawned or reopened.
    async fn finish_start(
        self: &Rc<Self>,
        owner: &Rc<Node<H>>,
        receipt: StartReceipt,
    ) -> Result<NodeId, String> {
        if self.disposing.get() {
            return Err(OWNER_CLOSING.to_owned());
        }
        let stored = self
            .store
            .node(&receipt.node_id)
            .await
            .map_err(|error| error.to_string())?;
        if let Some(record) = stored {
            if record.parent.as_ref() != Some(owner.id()) {
                return Err("request-id references an agent owned by another session".to_owned());
            }
            let spawn = matches!(receipt.input, StartInput::Spawn { .. });
            if spawn || record.round == receipt.spawned_at {
                let id = record.id.clone();
                if record.closed.is_none() && !self.is_live(&id) {
                    self.start_child(owner, record, None).await?;
                }
                return Ok(id);
            }
            if record.round > receipt.spawned_at {
                return Err("resume request has been superseded by a later round".to_owned());
            }
        }
        match receipt.input {
            StartInput::Spawn {
                prompt,
                profile_name,
                description,
                is_spawn_forbidden,
            } => {
                if !owner.record().can_spawn_subagents {
                    return Err("this session may not spawn subagents".to_owned());
                }
                self.check_capacity(owner.id())?;
                let profile = self.profile(profile_name.as_deref())?;
                let can_spawn = !is_spawn_forbidden
                    && profile.is_none_or(|profile| profile.can_spawn_subagents);
                let record = NodeRecord {
                    id: receipt.node_id,
                    parent: Some(owner.id().clone()),
                    description,
                    profile: profile_name,
                    round: receipt.spawned_at,
                    can_spawn_subagents: can_spawn,
                    closed: None,
                    delivered: false,
                };
                let brief = self.text_message(prompt);
                self.start_child(owner, record, Some(brief)).await
            }
            StartInput::Resume { id, message } => {
                self.reopen(owner, id, message, receipt.spawned_at).await
            }
        }
    }

    /// Revives an archived child of `owner` in one commit, a new round with
    /// the message queued, and restores it from its preserved transcript.
    async fn reopen(
        self: &Rc<Self>,
        owner: &Rc<Node<H>>,
        id: NodeId,
        message: String,
        round: u64,
    ) -> Result<NodeId, String> {
        if self.is_live(&id) {
            return Err(format!(
                "subagent \"{id}\" is still running; send it a message instead"
            ));
        }
        self.check_capacity(owner.id())?;
        let stored = self
            .store
            .node(&id)
            .await
            .map_err(|error| error.to_string())?;
        let Some(record) = stored
            .filter(|record| record.closed.is_some() && record.parent.as_ref() == Some(owner.id()))
        else {
            return Err(format!(
                "no archived subagent \"{id}\" (see `demi agent list`)"
            ));
        };
        if !record.delivered {
            return Err("The previous completion is not saved by the parent yet; retry resume after receiving it".to_owned());
        }
        // A profile the harness no longer declares leaves the archive as it
        // is.
        self.profile(record.profile.as_deref())?;
        self.store
            .reopen_node(&id, round, self.text_message(message))
            .await
            .map_err(|error| error.to_string())?;
        let live = NodeRecord {
            round,
            closed: None,
            delivered: false,
            ..record
        };
        self.start_child(owner, live, None).await
    }

    fn new_node_id(&self) -> NodeId {
        NodeId::try_from(self.ids.next_id()).expect("an id source never gives an empty id")
    }

    fn text_message(&self, text: String) -> QueuedMessage {
        QueuedMessage {
            id: TurnId::try_from(self.ids.next_id()).expect("an id source never gives an empty id"),
            content: vec![UserContentBlock::Text { text }],
        }
    }

    /// Everything spawn, reopen and restore share: the child's node from the
    /// assembly, new with its brief queued in the create commit or from the
    /// store; then its place in the directory and its frames, what it has
    /// yet to run, its own children, and its supervision.
    async fn start_child(
        self: &Rc<Self>,
        owner: &Rc<Node<H>>,
        record: NodeRecord,
        first_message: Option<QueuedMessage>,
    ) -> Result<NodeId, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        let deps = &server.deps;
        let profile = self.profile(record.profile.as_deref())?;
        let _activity = self.admission.enter(Purpose::Demand).await;
        let runtime = owner
            .session()
            .fork_runtime()
            .await
            .map_err(|error| error.to_string())?;
        let model = profile
            .and_then(|profile| profile.model.clone())
            .unwrap_or_else(|| owner.session().model());
        let prompt = match profile.and_then(|profile| profile.system_prompt.clone()) {
            Some(prompt) => Prompt::Profile(prompt),
            None => owner.prompt().clone(),
        };
        let inherited = match profile.and_then(|profile| profile.commands.clone()) {
            Some(narrow) => Rc::new(narrow(owner.inherited_commands())),
            None => owner.inherited_commands().clone(),
        };
        let can_spawn = record.can_spawn_subagents;
        let commands = with_agent_group(&inherited, &server, can_spawn, &self.profiles)
            .map_err(|error| error.to_string())?;
        let preamble = subagent_preamble(&record.id, owner.id(), can_spawn);
        let assembled = node::assemble(NodeSpec {
            record,
            role: NodeRole::Child,
            root: self.root.id().clone(),
            cwd: owner.cwd().to_owned(),
            model,
            runtime,
            harness: deps.harness.clone(),
            prompt,
            preamble_suffix: Some(preamble),
            inherited,
            commands: Rc::new(commands),
            first_message,
            store: self.store.clone(),
            shells: deps.shells.clone(),
            admission: self.admission.clone(),
            ids: deps.ids.clone(),
            clock: deps.clock.clone(),
            config: deps.config.session,
        })
        .await
        .map_err(|error| error.to_string())?;
        let child = self.attach_child(assembled.node);
        let id = child.id().clone();
        if let Some(continuation) = assembled.continuation
            && let Err(error) = child.node.continue_from(continuation).await
        {
            self.report(format!("subagent {id} did not save its start: {error}"));
        }
        self.restore_children(&child.node).await;
        self.supervise(&child);
        Ok(id)
    }

    /// Registers a new live child, sends its `started` frame and its
    /// transcript, and forwards its session's changes from then on.
    fn attach_child(&self, node: Node<H>) -> Rc<Child<H>> {
        let node = Rc::new(node);
        let now = self.clock.now().as_millisecond();
        let child = Rc::new_cyclic(|child: &Weak<Child<H>>| {
            let events = node.session().subscribe({
                let child = child.clone();
                let sink = self.sink.clone();
                let clock = self.clock.clone();
                let id = node.id().clone();
                move |event| match event {
                    SessionEvent::TranscriptChanged { patches, revision } => {
                        if let Some(child) = child.upgrade() {
                            let now = clock.now().as_millisecond();
                            let session = child.node.session();
                            child.telemetry.borrow_mut().observe(
                                patches,
                                |index| session.is_text_at(index),
                                now,
                            );
                        }
                        sink.emit(ServerFrame::SubagentTranscriptPatch {
                            subagent_id: id.clone(),
                            patches: patches.clone(),
                            revision: *revision,
                            failures: None,
                        });
                    }
                    SessionEvent::ActionFailed { report } => {
                        if let Some(child) = child.upgrade() {
                            child.failure.replace(Some(report.message.clone()));
                            child.wake.notify_one();
                        }
                    }
                    _ => {}
                }
            });
            Child {
                node: node.clone(),
                order: self.joined.get(),
                closing: Cell::new(false),
                failure: RefCell::new(None),
                wake: Rc::new(Notify::new()),
                closed: watch::Sender::new(false),
                telemetry: RefCell::new(Telemetry::new(now)),
                events: RefCell::new(Some(events)),
                supervision: RefCell::new(None),
            }
        });
        self.joined.set(self.joined.get() + 1);
        self.children
            .borrow_mut()
            .insert(child.id().clone(), child.clone());
        self.bump();
        for frame in child_frames(&child) {
            self.sink.emit(frame);
        }
        child
    }

    /// Brings `owner`'s children back from the store (`subagents.md`
    /// § Persistence): a closed child whose completion never reached the
    /// owner is delivered now, a live one is restored and continues, and a
    /// live one that cannot be rebuilt is deleted with its subtree.
    pub(super) fn restore_children<'a>(
        self: &'a Rc<Self>,
        owner: &'a Rc<Node<H>>,
    ) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            let _turn = self.starts.acquire(owner.id().clone()).await;
            let records = match self.store.children(owner.id()).await {
                Ok(records) => records,
                Err(error) => {
                    self.report(format!(
                        "the children of {} were not restored: {error}",
                        owner.id()
                    ));
                    return;
                }
            };
            for record in records {
                if self.disposing.get() {
                    return;
                }
                if self.is_live(&record.id) {
                    continue;
                }
                if let Some(close) = &record.closed {
                    if !record.delivered {
                        self.deliver(owner, &record, close).await;
                    }
                    continue;
                }
                let id = record.id.clone();
                if let Err(error) = self.start_child(owner, record, None).await {
                    tracing::warn!(node = %id, %error, "a live subagent could not be rebuilt and is deleted");
                    if let Err(error) = self.store.delete_node(&id).await {
                        self.report(format!("subagent {id} was not deleted: {error}"));
                    }
                }
            }
        })
    }

    /// Watches `child` until it closes: it closes with its result once it is
    /// quiescent, and with the failure once an action failed for good.
    fn supervise(self: &Rc<Self>, child: &Rc<Child<H>>) {
        let task = tokio::task::spawn_local(supervision(Rc::downgrade(self), Rc::downgrade(child)));
        *child.supervision.borrow_mut() = Some(AbortOnDropHandle::new(task));
    }

    /// Looks at `child` and decides, in one step with no await, whether it
    /// closes: a message accepted before this step keeps it open, one sent
    /// after it is refused.
    fn decide(&self, child: &Child<H>) -> Decision {
        if child.closing.get() || self.disposing.get() {
            return Decision::Stop;
        }
        let kind = if child.failure.borrow().is_some() {
            CloseKind::Error
        } else {
            let status = child.node.session().status();
            let quiescent = status.settle == Settle::Settled
                && !status.wakeups
                && !status.agent_input
                && self.children_of(child.id()).is_empty()
                && !self.starting.borrow().contains_key(child.id());
            if !quiescent {
                return Decision::Wait;
            }
            CloseKind::Completed
        };
        child.closing.set(true);
        Decision::Close(kind)
    }

    /// Runs `child`'s close in a task of the tree, which dispose waits for.
    /// The caller has set `closing`.
    fn begin_close(self: &Rc<Self>, child: Rc<Child<H>>, kind: CloseKind) {
        let tree = self.clone();
        self.lifecycle
            .spawn_local(async move { tree.close(&child, kind).await });
    }

    /// Aborts a live child and its subtree, and returns once it is closed.
    pub(crate) async fn abort_child(self: &Rc<Self>, id: &NodeId) {
        let Some(child) = self.child(id) else {
            return;
        };
        if self.disposing.get() {
            return;
        }
        if !child.closing.replace(true) {
            child.stop_supervision();
            self.begin_close(child.clone(), CloseKind::Aborted);
        }
        let mut closed = child.closed.subscribe();
        // The sender lives in the child this call holds.
        let _ = closed.wait_for(|closed| *closed).await;
    }

    /// Aborts every live child of `owner`, each with its subtree.
    pub(crate) async fn abort_children_of(self: &Rc<Self>, owner: &NodeId) {
        for child in self.children_of(owner) {
            self.abort_child(child.id()).await;
        }
    }

    /// Closes a child (`subagents.md` § Result, § Abort): an abort or a
    /// failure first closes its subtree and stops what it still runs; its
    /// session saves its final checkpoint, the close is committed, the child
    /// leaves the directory with a `closed` frame, and its completion goes to
    /// its parent, whose save marks it delivered.
    async fn close(self: &Rc<Self>, child: &Rc<Child<H>>, kind: CloseKind) {
        let record = child.node.record().clone();
        let owner = self.node(child.parent());
        let _lease = match &owner {
            Some(owner) => Some(owner.lifecycle().enter(Purpose::Maintenance).await),
            None => None,
        };
        let session = child.node.session();
        if kind != CloseKind::Completed {
            // Nothing runs again: the completions of its subtree wait in
            // its final checkpoint for a later round.
            session.hold();
            stop_all(session).await;
            // A start of its own under way ends first, so its child is
            // closed with the others.
            let _turn = self.starts.acquire(child.id().clone()).await;
            Box::pin(self.abort_children_of(child.id())).await;
        }
        let phase = match kind {
            CloseKind::Completed => ClosePhase::Completed {
                result: bounded(&session.last_assistant_text()).to_owned(),
            },
            CloseKind::Aborted => ClosePhase::Aborted,
            CloseKind::Error => ClosePhase::Error {
                failure: child.failure.borrow().clone().unwrap_or_default(),
            },
        };
        if let Err(error) = session.dispose().await {
            self.report(format!(
                "subagent {} did not save its final checkpoint: {error}",
                record.id
            ));
        }
        child.events.borrow_mut().take();
        let close = NodeClose {
            phase,
            at: self.clock.now(),
        };
        let committed = self.store.close_node(&record.id, close.clone()).await;
        self.children.borrow_mut().remove(&record.id);
        self.bump();
        match committed {
            Ok(()) => {
                let closed = NodeRecord {
                    closed: Some(close.clone()),
                    ..record.clone()
                };
                self.sink.emit(ServerFrame::Subagent {
                    event: SubagentEvent::Closed,
                    job: closed.job().expect("a child has a job"),
                });
                if let Some(owner) = &owner {
                    self.deliver(owner, &record, &close).await;
                }
            }
            // The child stays live and quiescent in the store: the next
            // restore closes it again.
            Err(error) => self.report(format!("subagent {} did not close: {error}", record.id)),
        }
        child.closed.send_replace(true);
    }

    /// Hands a closed round's completion to its parent; the parent's save
    /// marks it delivered. A parent that is closing leaves it undelivered for
    /// its restore.
    async fn deliver(&self, owner: &Node<H>, record: &NodeRecord, close: &NodeClose) {
        let message = completion(record, close);
        match owner.session().accept_agent_message(message).await {
            Ok(()) | Err(AgentMessageError::Closed) => {}
            Err(error) => self.report(format!(
                "the completion of subagent {} was not delivered: {error}",
                record.id
            )),
        }
    }

    /// Sends a message from `caller` to any live agent of the tree, or to
    /// its parent (`subagents.md` § One communication operation); returns
    /// the recipient once the message is accepted durably.
    pub(crate) async fn send_message(
        &self,
        caller: &NodeId,
        target: &str,
        content: String,
    ) -> Result<NodeId, String> {
        let sender = self
            .node(caller)
            .ok_or("this session is not in the agent directory")?;
        let no_live = |id: &str| {
            format!(
                "no live agent \"{id}\" (see `demi agent list`; an archived child is revived only by its parent via resume)"
            )
        };
        let recipient_id = if target == "parent" {
            sender
                .record()
                .parent
                .clone()
                .ok_or("the root session has no parent")?
        } else {
            NodeId::try_from(target).map_err(|_| no_live(target))?
        };
        if &recipient_id == caller {
            return Err("cannot message your own session".to_owned());
        }
        let recipient = if &recipient_id == self.root.id() {
            self.root.clone()
        } else {
            match self.child(&recipient_id) {
                Some(child) if !child.closing.get() => child.node.clone(),
                _ => return Err(no_live(recipient_id.as_str())),
            }
        };
        let description = if sender.record().parent.is_none() {
            "root session".to_owned()
        } else {
            sender.record().description.clone()
        };
        let message = AgentMessage {
            id: BlockId::try_from(self.ids.next_id())
                .expect("an id source never gives an empty id"),
            sender: Sender {
                id: caller.clone(),
                description,
                round: sender.record().round,
            },
            recipient_id: recipient_id.clone(),
            timestamp: self.clock.now(),
            content,
            event: AgentMessageEvent::Message {},
        };
        recipient
            .session()
            .accept_agent_message(message)
            .await
            .map_err(|error| error.to_string())?;
        Ok(recipient_id)
    }

    /// Each live child's `started` frame and transcript, depth first in
    /// spawn order: what a connection that attaches or syncs receives.
    pub(super) fn replay(&self) -> Vec<ServerFrame> {
        let mut frames = Vec::new();
        self.replay_children(self.root.id(), &mut frames);
        frames
    }

    fn replay_children(&self, owner: &NodeId, frames: &mut Vec<ServerFrame>) {
        for child in self.children_of(owner) {
            frames.extend(child_frames(&child));
            self.replay_children(child.id(), frames);
        }
    }

    /// The whole tree from the root down, with each node's archived
    /// children after its live ones (`subagents.md` § `demi agent list`).
    pub(crate) async fn listing(&self) -> Result<Listing, String> {
        let now = self.clock.now().as_millisecond();
        let root = self.list_node(self.root.id().clone(), None, now).await?;
        Ok(Listing { root })
    }

    fn list_node(
        &self,
        id: NodeId,
        child: Option<Rc<Child<H>>>,
        now: i64,
    ) -> LocalBoxFuture<'_, Result<ListNode, String>> {
        Box::pin(async move {
            let mut children = Vec::new();
            for live in self.children_of(&id) {
                let live_id = live.id().clone();
                children.push(self.list_node(live_id, Some(live), now).await?);
            }
            let mut archived: Vec<NodeRecord> = self
                .store
                .children(&id)
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|record| record.closed.is_some() && !self.is_live(&record.id))
                .collect();
            archived.sort_by_key(|record| {
                std::cmp::Reverse(record.closed.as_ref().map(|close| close.at))
            });
            for record in archived {
                let close = record.closed.as_ref().expect("an archived child is closed");
                children.push(ListNode {
                    kind: EntryKind::Archived,
                    phase: close.phase.job_phase(),
                    closed_ago_ms: Some(age(now, close.at.as_millisecond())),
                    line: None,
                    children: Vec::new(),
                    parent: record.parent.clone(),
                    description: record.description.clone(),
                    profile: record.profile.clone(),
                    id: record.id,
                });
            }
            Ok(match child {
                Some(child) => {
                    let record = child.node.record();
                    ListNode {
                        id,
                        parent: record.parent.clone(),
                        kind: EntryKind::Live,
                        description: record.description.clone(),
                        profile: record.profile.clone(),
                        phase: JobPhase::Running,
                        closed_ago_ms: None,
                        line: Some(list_line(&child, now)),
                        children,
                    }
                }
                None => ListNode {
                    id,
                    parent: None,
                    kind: EntryKind::Root,
                    description: String::new(),
                    profile: None,
                    phase: JobPhase::Running,
                    closed_ago_ms: None,
                    line: None,
                    children,
                },
            })
        })
    }

    /// A bounded snapshot of a live child, as JSON and as text
    /// (`subagents.md` § `demi agent show`); none for the root and for an id
    /// that is not live.
    pub(crate) fn show(&self, id: &NodeId) -> Option<(AgentSnapshot, String)> {
        let child = self.child(id)?;
        let now = self.clock.now().as_millisecond();
        let snapshot = snapshot(&child, now);
        let text = show_text(&snapshot);
        Some((snapshot, text))
    }
}

/// A start under way past its owner's check: while it lasts, the owner is
/// not quiescent. It ends with a change of the tree, so the owner's
/// supervision looks again.
struct Starting<H: AgentHarness> {
    tree: Rc<Tree<H>>,
    owner: NodeId,
}

impl<H: AgentHarness> Starting<H> {
    fn new(tree: &Rc<Tree<H>>, owner: &NodeId) -> Self {
        *tree.starting.borrow_mut().entry(owner.clone()).or_default() += 1;
        Self {
            tree: tree.clone(),
            owner: owner.clone(),
        }
    }
}

impl<H: AgentHarness> Drop for Starting<H> {
    fn drop(&mut self) {
        {
            let mut starting = self.tree.starting.borrow_mut();
            let count = starting
                .get_mut(&self.owner)
                .expect("a start under way is counted");
            *count -= 1;
            if *count == 0 {
                starting.remove(&self.owner);
            }
        }
        self.tree.bump();
    }
}

/// The supervision of one child: waits for any change of its session's
/// status, of the tree's children, or of its failure, and closes it once
/// [`Tree::decide`] says so. It holds neither the tree nor the child while
/// it waits.
async fn supervision<H: AgentHarness>(tree: Weak<Tree<H>>, child: Weak<Child<H>>) {
    let (mut status, mut changes, wake) = {
        let (Some(live_tree), Some(live_child)) = (tree.upgrade(), child.upgrade()) else {
            return;
        };
        (
            live_child.node.session().status_watch(),
            live_tree.changes.subscribe(),
            live_child.wake.clone(),
        )
    };
    loop {
        status.mark_unchanged();
        changes.mark_unchanged();
        {
            let (Some(live_tree), Some(live_child)) = (tree.upgrade(), child.upgrade()) else {
                return;
            };
            match live_tree.decide(&live_child) {
                Decision::Wait => {}
                Decision::Stop => return,
                Decision::Close(kind) => {
                    live_tree.begin_close(live_child, kind);
                    return;
                }
            }
        }
        tokio::select! {
            changed = status.changed() => if changed.is_err() { return },
            changed = changes.changed() => if changed.is_err() { return },
            () = wake.notified() => {}
        }
    }
}

/// Stops everything a session would still do: its running action, which
/// records the stop, then the waiting ones and the scheduled wakeups.
async fn stop_all(session: &AgentSession) {
    while session.abort().await.target.is_some() {}
}

/// A child's `started` frame and its transcript.
fn child_frames<H: AgentHarness>(child: &Child<H>) -> [ServerFrame; 2] {
    let transcript = child.node.session().transcript();
    [
        ServerFrame::Subagent {
            event: SubagentEvent::Started,
            job: child.node.record().job().expect("a child has a job"),
        },
        ServerFrame::SubagentTranscriptReset {
            subagent_id: child.id().clone(),
            blocks: transcript.blocks,
            revision: transcript.version.revision,
            failures: None,
        },
    ]
}

/// A closed round's completion message (`subagents.md` § Message
/// identity): its id names the round, and its body is the result, or the
/// failure text, which can be empty.
fn completion(record: &NodeRecord, close: &NodeClose) -> AgentMessage {
    let round = CompletionId {
        child: record.id.clone(),
        round: record.round,
    };
    let (content, outcome) = match &close.phase {
        ClosePhase::Completed { result } => (result.clone(), CompletionOutcome::Completed),
        ClosePhase::Aborted => (String::new(), CompletionOutcome::Aborted),
        ClosePhase::Error { failure } => (failure.clone(), CompletionOutcome::Failed),
    };
    AgentMessage {
        id: round.block_id(),
        sender: Sender {
            id: record.id.clone(),
            description: record.description.clone(),
            round: record.round,
        },
        recipient_id: record
            .parent
            .clone()
            .expect("a child's record names its parent"),
        timestamp: close.at,
        content,
        event: AgentMessageEvent::Completion { outcome },
    }
}

/// `text` cut at a character boundary to at most [`RESULT_MAX_BYTES`].
fn bounded(text: &str) -> &str {
    &text[..text.floor_char_boundary(RESULT_MAX_BYTES)]
}

/// The child's preamble, after the one it inherits (`subagents.md` § Child
/// context).
fn subagent_preamble(child: &NodeId, parent: &NodeId, can_spawn: bool) -> String {
    let spawning = if can_spawn {
        "`demi agent spawn` spawns your own children."
    } else {
        "This session may not spawn subagents."
    };
    [
        format!("You are a subagent: a child agent session (id {child}) spawned by parent agent session {parent}. Your transcript starts empty; the task brief in the first user message is your entire context."),
        "When you end your turn with nothing pending — no queued messages, no scheduled wakeups, no running children of your own — the session ends and your last assistant text is returned to the parent as the result. Write it for the parent agent, in the shape the task brief asked for.".to_owned(),
        spawning.to_owned(),
        "`demi agent send <id|parent>` delivers useful interim information, questions, or blockers through internal steering or an idle wakeup. It reads the message only from stdin (use a quoted heredoc). Your final answer is delivered automatically; do not send a duplicate final result. `demi agent list` renders the whole agent tree with your position.".to_owned(),
        "You are not talking to the product user; do not address them.".to_owned(),
    ]
    .join("\n")
}

/// What a child's supervision observed of its transcript: when it last did
/// something, its recent tool calls, and when it last wrote text.
struct Telemetry {
    last_event_at: i64,
    tools: Vec<ToolRecord>,
    last_text_at: Option<i64>,
}

struct ToolRecord {
    tool_use_id: String,
    title: String,
    started_at: i64,
    ended_at: Option<i64>,
    status: ToolStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ToolStatus {
    Executing,
    Completed,
    Error,
}

impl Telemetry {
    fn new(now: i64) -> Self {
        Self {
            last_event_at: now,
            tools: Vec::new(),
            last_text_at: None,
        }
    }

    /// Records a batch of patches at `now`: a tool call's start and end, and
    /// assistant text as it arrives.
    fn observe(&mut self, patches: &[TranscriptPatch], is_text: impl Fn(usize) -> bool, now: i64) {
        for patch in patches {
            match patch {
                TranscriptPatch::Add {
                    value: Block::ToolCall(call),
                    ..
                } => {
                    self.tools.push(ToolRecord {
                        tool_use_id: call.tool_use_id.clone(),
                        title: tool_title(call),
                        started_at: now,
                        ended_at: None,
                        status: ToolStatus::Executing,
                    });
                    self.trim();
                    self.last_event_at = now;
                }
                TranscriptPatch::Add {
                    value: Block::Text(_),
                    ..
                } => self.text(now),
                TranscriptPatch::AppendText { index, .. } if is_text(*index as usize) => {
                    self.text(now);
                }
                TranscriptPatch::ReplaceBlock {
                    value: Block::ToolCall(call),
                    ..
                } if call.status != ToolCallStatus::Executing => {
                    if let Some(record) = self.tools.iter_mut().find(|record| {
                        record.tool_use_id == call.tool_use_id && record.ended_at.is_none()
                    }) {
                        record.ended_at = Some(now);
                        record.status = if call.status == ToolCallStatus::Error {
                            ToolStatus::Error
                        } else {
                            ToolStatus::Completed
                        };
                    }
                    self.last_event_at = now;
                }
                _ => {}
            }
        }
    }

    fn text(&mut self, now: i64) {
        self.last_text_at = Some(now);
        self.last_event_at = now;
    }

    /// Keeps at most [`SHOW_RECENT_TOOLS`] calls, dropping the oldest ended
    /// ones: a call in flight is never dropped.
    fn trim(&mut self) {
        while self.tools.len() > SHOW_RECENT_TOOLS {
            let Some(index) = self.tools.iter().position(|tool| tool.ended_at.is_some()) else {
                return;
            };
            self.tools.remove(index);
        }
    }

    fn in_flight(&self) -> Option<&ToolRecord> {
        self.tools.iter().rev().find(|tool| tool.ended_at.is_none())
    }
}

/// A call's title: the trimmed `description` it declared, else the tool's
/// name. The input is model output, so anything else falls back.
fn tool_title(call: &ToolCallBlock) -> String {
    #[derive(Deserialize)]
    struct Titled {
        description: String,
    }
    serde_json::from_str::<Titled>(&call.input)
        .ok()
        .map(|titled| demi_core::trim(&titled.description).to_owned())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| call.tool_name.clone())
}

/// How long ago `then` was, in milliseconds; never negative.
fn age(now: i64, then: i64) -> u64 {
    u64::try_from(now.saturating_sub(then)).unwrap_or(0)
}

/// A duration as `list` and `show` print it: `45s`, `4m`, `1m5s`, `2h` or
/// `1h3m`, rounded to the nearest second.
fn duration(ms: u64) -> String {
    let seconds = (ms + 500) / 1000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return match seconds % 60 {
            0 => format!("{minutes}m"),
            rest => format!("{minutes}m{rest}s"),
        };
    }
    match minutes % 60 {
        0 => format!("{}h", minutes / 60),
        rest => format!("{}h{rest}m", minutes / 60),
    }
}

/// One node of `demi agent list`.
struct ListNode {
    id: NodeId,
    parent: Option<NodeId>,
    kind: EntryKind,
    description: String,
    profile: Option<String>,
    phase: JobPhase,
    closed_ago_ms: Option<u64>,
    /// A live child's status line.
    line: Option<String>,
    children: Vec<ListNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum EntryKind {
    Root,
    Live,
    Archived,
}

/// `demi agent list`'s tree, rendered for one caller.
pub(crate) struct Listing {
    root: ListNode,
}

/// One node of `demi agent list --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TreeEntry {
    subagent_id: NodeId,
    parent_session_id: Option<NodeId>,
    kind: EntryKind,
    description: String,
    profile: Option<String>,
    phase: JobPhase,
    closed_ago_ms: Option<u64>,
    #[serde(rename = "self")]
    is_caller: bool,
}

impl Listing {
    /// The tree as lines, the caller's marked.
    pub(crate) fn render(&self, caller: &NodeId) -> String {
        let mut lines = Vec::new();
        render_node(&self.root, "", true, caller, &mut lines);
        lines.join("\n")
    }

    /// The same nodes as a flat list, in the same order.
    pub(crate) fn entries(&self, caller: &NodeId) -> Vec<TreeEntry> {
        let mut entries = Vec::new();
        let mut stack = vec![&self.root];
        while let Some(node) = stack.pop() {
            entries.push(TreeEntry {
                subagent_id: node.id.clone(),
                parent_session_id: node.parent.clone(),
                kind: node.kind,
                description: node.description.clone(),
                profile: node.profile.clone(),
                phase: node.phase,
                closed_ago_ms: node.closed_ago_ms,
                is_caller: &node.id == caller,
            });
            stack.extend(node.children.iter().rev());
        }
        entries
    }
}

fn render_node(
    node: &ListNode,
    prefix: &str,
    last: bool,
    caller: &NodeId,
    lines: &mut Vec<String>,
) {
    let marker = if &node.id == caller { " ← you" } else { "" };
    let body = match node.kind {
        EntryKind::Root => format!("● {}  (root session){marker}", node.id),
        EntryKind::Archived => {
            let ago = node
                .closed_ago_ms
                .map(|ago| format!(" {} ago", duration(ago)))
                .unwrap_or_default();
            format!(
                "○ {}  archived ({}{ago})  {}",
                node.id,
                node.phase,
                quoted(&node.description)
            )
        }
        EntryKind::Live => format!(
            "● {}{marker}",
            node.line.as_deref().expect("a live node has a line")
        ),
    };
    let child_prefix = match node.kind {
        EntryKind::Root => {
            lines.push(body);
            String::new()
        }
        _ => {
            let branch = if last { "└─" } else { "├─" };
            lines.push(format!("{prefix}{branch}{body}"));
            format!("{prefix}{}", if last { "  " } else { "│ " })
        }
    };
    for (index, child) in node.children.iter().enumerate() {
        render_node(
            child,
            &child_prefix,
            index + 1 == node.children.len(),
            caller,
            lines,
        );
    }
}

fn quoted(description: &str) -> String {
    if description.is_empty() {
        "(no description)".to_owned()
    } else {
        format!("\"{description}\"")
    }
}

/// A live child's line in `demi agent list`.
fn list_line<H: AgentHarness>(child: &Child<H>, now: i64) -> String {
    let snapshot = snapshot(child, now);
    [
        snapshot.subagent_id.to_string(),
        snapshot.phase.to_string(),
        format!("up {}", duration(snapshot.elapsed_ms)),
        format!("last-event {} ago", duration(snapshot.last_event_ms)),
        format!(
            "profile={}",
            snapshot.profile.as_deref().unwrap_or(INHERIT_LABEL)
        ),
        quoted(&snapshot.description),
        format!("execution={}", snapshot.execution.as_str()),
        format!("activity={}", snapshot.activity),
    ]
    .join("  ")
}

/// `demi agent show --json`'s agent: every duration in milliseconds before
/// the query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSnapshot {
    subagent_id: NodeId,
    parent_session_id: NodeId,
    description: String,
    profile: Option<String>,
    phase: JobPhase,
    elapsed_ms: u64,
    last_event_ms: u64,
    execution: Execution,
    activity: String,
    /// How long the current execution state has lasted.
    execution_for_ms: u64,
    tools: Vec<ToolSnapshot>,
    last_assistant_text: String,
    last_assistant_text_ago_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ToolSnapshot {
    title: String,
    status: ToolStatus,
    duration_ms: u64,
    ended_ago_ms: Option<u64>,
}

fn snapshot<H: AgentHarness>(child: &Child<H>, now: i64) -> AgentSnapshot {
    let record = child.node.record();
    let session = child.node.session();
    let telemetry = child.telemetry.borrow();
    let execution = session.execution();
    let in_flight = telemetry.in_flight();
    let activity = match (execution, in_flight) {
        (Execution::ToolExecuting, Some(tool)) => tool.title.clone(),
        (Execution::ProviderStreaming, _) => "streaming".to_owned(),
        (execution, _) => execution.as_str().to_owned(),
    };
    let execution_since = match (execution, in_flight) {
        (Execution::ToolExecuting, Some(tool)) => tool.started_at,
        _ => telemetry.last_event_at,
    };
    AgentSnapshot {
        subagent_id: record.id.clone(),
        parent_session_id: child.parent().clone(),
        description: record.description.clone(),
        profile: record.profile.clone(),
        phase: JobPhase::Running,
        elapsed_ms: age(now, record.started_at().as_millisecond()),
        last_event_ms: age(now, telemetry.last_event_at),
        execution,
        activity,
        execution_for_ms: age(now, execution_since),
        tools: telemetry
            .tools
            .iter()
            .map(|tool| ToolSnapshot {
                title: tool.title.clone(),
                status: tool.status,
                duration_ms: age(tool.ended_at.unwrap_or(now), tool.started_at),
                ended_ago_ms: tool.ended_at.map(|ended| age(now, ended)),
            })
            .collect(),
        last_assistant_text: bounded(&session.last_assistant_text()).to_owned(),
        last_assistant_text_ago_ms: telemetry.last_text_at.map(|at| age(now, at)),
    }
}

/// `demi agent show`'s text.
fn show_text(snapshot: &AgentSnapshot) -> String {
    let description = if snapshot.description.is_empty() {
        "(none)"
    } else {
        &snapshot.description
    };
    let mut lines = vec![
        format!("id: {}", snapshot.subagent_id),
        format!("parent: {}", snapshot.parent_session_id),
        format!("description: {description}"),
        format!(
            "profile: {}",
            snapshot.profile.as_deref().unwrap_or(INHERIT_LABEL)
        ),
        format!("phase: {}", snapshot.phase),
        format!("elapsed: {}", duration(snapshot.elapsed_ms)),
        format!(
            "execution: {} (for {})",
            snapshot.execution.as_str(),
            duration(snapshot.execution_for_ms)
        ),
        format!("last-event: {} ago", duration(snapshot.last_event_ms)),
        format!("activity: {}", snapshot.activity),
    ];
    if !snapshot.tools.is_empty() {
        lines.push(format!(
            "recent tool calls (last {}):",
            snapshot.tools.len()
        ));
        for tool in &snapshot.tools {
            lines.push(match tool.ended_ago_ms {
                None => format!(
                    "  [executing for {}] {}",
                    duration(tool.duration_ms),
                    tool.title
                ),
                Some(ended) => format!(
                    "  [{} in {}, ended {} ago] {}",
                    tool.status.as_str(),
                    duration(tool.duration_ms),
                    duration(ended),
                    tool.title
                ),
            });
        }
    }
    match snapshot.last_assistant_text_ago_ms {
        Some(ago) if !snapshot.last_assistant_text.is_empty() => {
            lines.push(format!("last assistant text ({} ago):", duration(ago)));
            lines.push(snapshot.last_assistant_text.clone());
        }
        _ => lines.push("last assistant text: (none yet)".to_owned()),
    }
    format!("{}\n", lines.join("\n"))
}

impl ToolStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Executing => "executing",
            Self::Completed => "completed",
            Self::Error => "error",
        }
    }
}
