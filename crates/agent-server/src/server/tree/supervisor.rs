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

use demi_agent_session::{
    AgentMessageError, AgentSession, Execution, SessionEvent, Settle, Subscription,
};
use demi_agent_store::{ClosePhase, NodeClose, NodeRecord};
use demi_agent_tools::HostResolver;
use demi_conversation_socket_protocol::{JobPhase, ServerFrame, SubagentEvent, TranscriptPatch};
use demi_host_interface::CommandSet;
use demi_shared_gates::Purpose;
use demi_shared_types::{
    AgentMessage, AgentMessageEvent, Block, BlockId, CompletionId, CompletionOutcome, NodeId,
    Profile, QueuedMessage, Sender, Sequence, ToolCallBlock, ToolCallStatus, TurnId,
    UserContentBlock,
};
use futures_util::future::LocalBoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};
use tokio_util::task::AbortOnDropHandle;

use super::Tree;
use crate::{
    Node,
    node::{self, NodeRole, NodeSpec},
    server::commands::with_runtime_groups,
};

/// The most live children one node has at once.
const MAX_LIVE_CHILDREN: usize = 8;
/// The bound of a completion's result and of `show`'s last assistant text,
/// in UTF-8 bytes.
const RESULT_MAX_BYTES: usize = 32 * 1024;
/// The most recent tool calls `show` lists.
const SHOW_RECENT_TOOLS: usize = 8;
const INHERIT_LABEL: &str = "(inherit)";
const OWNER_CLOSING: &str = "owner session is closing";

/// A live child: its node, and what its supervision keeps about it.
pub(crate) struct Child<H: HostResolver> {
    node: Rc<Node<H>>,
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

impl<H: HostResolver> Child<H> {
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

    /// Its place among its siblings: spawn order, its number's.
    fn rank(&self) -> u64 {
        self.node.record().number
    }

    pub(super) fn stop_supervision(&self) {
        self.supervision.borrow_mut().take();
    }
}

/// What a start does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StartInput {
    Spawn {
        prompt: String,
        profile_name: Option<String>,
        description: String,
        is_spawn_forbidden: bool,
    },
    Resume {
        id: NodeId,
        message: String,
    },
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

impl<H: HostResolver> Tree<H> {
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

    fn profile(&self, name: Option<&str>) -> Result<Option<&Profile>, String> {
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

    /// Starts a child of `caller` for `demi agent spawn` or `resume`
    /// (`subagents.md` § Creation command ownership): creates or reopens the
    /// child in a task of the tree, so the start runs to its end even when
    /// the call is cancelled. Starts of one owner take turns. Answers the
    /// child's number.
    pub(crate) async fn start(
        self: &Rc<Self>,
        caller: &NodeId,
        input: StartInput,
    ) -> Result<u64, String> {
        let tree = self.clone();
        let caller = caller.clone();
        let started = self.lifecycle.spawn_local(async move {
            let _turn = tree.starts.acquire(caller.clone()).await;
            let owner = tree.owner(&caller)?;
            let _starting = Starting::new(&tree, &caller);
            let _lease = owner
                .lifecycle()
                .try_enter(Purpose::Maintenance)
                .ok_or("Cannot change children while a transcript edit is being prepared")?;
            if tree.disposing.get() {
                return Err(OWNER_CLOSING.to_owned());
            }
            match input {
                StartInput::Spawn {
                    prompt,
                    profile_name,
                    description,
                    is_spawn_forbidden,
                } => {
                    tree.spawn(
                        &owner,
                        prompt,
                        profile_name,
                        description,
                        is_spawn_forbidden,
                    )
                    .await
                }
                StartInput::Resume { id, message } => tree.reopen(&owner, id, message).await,
            }
        });
        started
            .await
            .map_err(|error| format!("the start of a subagent failed: {error}"))?
    }

    /// Spawns a child of `owner` with the next agent number and `prompt`
    /// queued. Answers its number.
    async fn spawn(
        self: &Rc<Self>,
        owner: &Rc<Node<H>>,
        prompt: String,
        profile_name: Option<String>,
        description: String,
        is_spawn_forbidden: bool,
    ) -> Result<u64, String> {
        if !owner.record().can_spawn_subagents {
            return Err("this session may not spawn subagents".to_owned());
        }
        self.check_capacity(owner.id())?;
        let profile = self.profile(profile_name.as_deref())?;
        let can_spawn =
            !is_spawn_forbidden && profile.is_none_or(|profile| profile.can_spawn_subagents);
        let number = self
            .store
            .next_number(Sequence::Agent)
            .await
            .map_err(|error| error.to_string())?;
        let record = NodeRecord {
            id: self.new_node_id(),
            number,
            parent: Some(owner.id().clone()),
            description,
            profile: profile_name,
            round: 1,
            started_at: self.clock.now(),
            can_spawn_subagents: can_spawn,
            closed: None,
            delivered: false,
        };
        let brief = self.text_message(prompt);
        self.start_child(owner, record, Some(brief)).await?;
        Ok(number)
    }

    /// Revives an archived child of `owner` in one commit, a new round with
    /// the message queued, and restores it from its preserved transcript.
    /// Answers its number.
    async fn reopen(
        self: &Rc<Self>,
        owner: &Rc<Node<H>>,
        id: NodeId,
        message: String,
    ) -> Result<u64, String> {
        let stored = self
            .store
            .node(&id)
            .await
            .map_err(|error| error.to_string())?;
        let Some(record) = stored.filter(|record| record.parent.as_ref() == Some(owner.id()))
        else {
            return Err("no such subagent of yours (see `demi agent list`)".to_owned());
        };
        let number = record.number;
        if self.is_live(&id) {
            return Err(format!(
                "subagent {number} is still running; send it a message instead"
            ));
        }
        if record.closed.is_none() {
            return Err(format!(
                "no archived subagent {number} (see `demi agent list`)"
            ));
        }
        self.check_capacity(owner.id())?;
        if !record.delivered {
            return Err("The previous completion is not saved by the parent yet; retry resume after receiving it".to_owned());
        }
        // A profile no plugin declares any more leaves the archive as it
        // is.
        self.profile(record.profile.as_deref())?;
        let round = record.round + 1;
        let started_at = self.clock.now();
        self.store
            .reopen_node(&id, round, started_at, self.text_message(message))
            .await
            .map_err(|error| error.to_string())?;
        let live = NodeRecord {
            round,
            started_at,
            closed: None,
            delivered: false,
            ..record
        };
        self.start_child(owner, live, None).await?;
        Ok(number)
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
        let activity = self.admission.enter(Purpose::Demand).await;
        let runtime = owner
            .session()
            .fork_runtime()
            .await
            .map_err(|error| error.to_string())?;
        let model = profile
            .and_then(|profile| profile.model.clone())
            .unwrap_or_else(|| owner.session().model());
        let instructions = match profile.and_then(|profile| profile.instructions.as_deref()) {
            Some(instructions) => Rc::from(instructions),
            None => owner.instructions().clone(),
        };
        let inherited = match profile {
            Some(profile) => Rc::new(narrowed(profile, owner.inherited_commands())),
            None => owner.inherited_commands().clone(),
        };
        let can_spawn = record.can_spawn_subagents;
        let commands = with_runtime_groups(&inherited, &server, can_spawn, &self.profiles)
            .map_err(|error| error.to_string())?;
        let preamble = subagent_preamble(record.number, owner.record().number, can_spawn);
        let feed = self.live.feed(Some(record.id.clone()));
        let assembled = node::assemble(NodeSpec {
            record,
            role: NodeRole::Child,
            root: self.root.id().clone(),
            cwd: owner.cwd().to_owned(),
            model,
            runtime,
            providers: deps.providers.clone(),
            hosts: deps.hosts.clone(),
            instructions,
            preamble: Some(preamble),
            context: deps.context.clone(),
            inherited,
            commands: Rc::new(commands),
            first_message,
            store: self.store.clone(),
            shells: deps.shells.clone(),
            feed,
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
        // Each start of the subtree enters the admission on its own; one held
        // across them would wait behind a reservation that waits for it.
        drop(activity);
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
                closing: Cell::new(false),
                failure: RefCell::new(None),
                wake: Rc::new(Notify::new()),
                closed: watch::Sender::new(false),
                telemetry: RefCell::new(Telemetry::new(now)),
                events: RefCell::new(Some(events)),
                supervision: RefCell::new(None),
            }
        });
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
    /// failure first stops what it still runs and then closes its subtree;
    /// its session saves its final checkpoint, the close is committed, the
    /// child leaves the directory with a `closed` frame, and its completion
    /// goes to its parent, whose save marks it delivered.
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

    /// Sends a message from `caller` to any live agent of the tree, which
    /// `target` names by its number, or to its parent (`subagents.md` § One
    /// communication operation); returns the recipient's number once the
    /// message is accepted durably.
    pub(crate) async fn send_message(
        &self,
        caller: &NodeId,
        target: &str,
        content: String,
    ) -> Result<u64, String> {
        let sender = self
            .node(caller)
            .ok_or("this session is not in the agent directory")?;
        let no_live = || {
            format!(
                "no live agent \"{target}\" (see `demi agent list`; an archived child is revived only by its parent via resume)"
            )
        };
        let recipient_id = if target == "parent" {
            sender
                .record()
                .parent
                .clone()
                .ok_or("the root session has no parent")?
        } else {
            let number: u64 = target.parse().map_err(|_| no_live())?;
            if number == self.root.record().number {
                self.root.id().clone()
            } else {
                self.child_numbered(number)
                    .ok_or_else(no_live)?
                    .id()
                    .clone()
            }
        };
        if &recipient_id == caller {
            return Err("cannot message your own session".to_owned());
        }
        let recipient = if &recipient_id == self.root.id() {
            self.root.clone()
        } else {
            match self.child(&recipient_id) {
                Some(child) if !child.closing.get() => child.node.clone(),
                _ => return Err(no_live()),
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
                number: sender.record().number,
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
        Ok(recipient.record().number)
    }

    /// Each live child's `started` frame and transcript, depth first in
    /// spawn order: what a connection that attaches or syncs receives.
    pub(super) fn replay(&self) -> Vec<ServerFrame> {
        self.descendants()
            .iter()
            .flat_map(|child| child_frames(child))
            .collect()
    }

    /// Every live child, depth first in spawn order.
    pub(super) fn descendants(&self) -> Vec<Rc<Child<H>>> {
        let mut children = Vec::new();
        self.collect_children(self.root.id(), &mut children);
        children
    }

    fn collect_children(&self, owner: &NodeId, children: &mut Vec<Rc<Child<H>>>) {
        for child in self.children_of(owner) {
            let id = child.id().clone();
            children.push(child);
            self.collect_children(&id, children);
        }
    }

    /// The whole tree from the root down, with each node's archived
    /// children after its live ones (`subagents.md` § `demi agent list`).
    pub(crate) async fn listing(&self) -> Result<Listing, String> {
        let now = self.clock.now().as_millisecond();
        let root = self.list_node(self.root.id().clone(), None, now).await?;
        Ok(Listing { root })
    }

    /// The node `id` with its children: the root when `child` is none, else
    /// that live child with its parent's number.
    fn list_node(
        &self,
        id: NodeId,
        child: Option<(Rc<Child<H>>, u64)>,
        now: i64,
    ) -> LocalBoxFuture<'_, Result<ListNode, String>> {
        Box::pin(async move {
            let number = match &child {
                Some((child, _)) => child.node.record().number,
                None => self.root.record().number,
            };
            let mut children = Vec::new();
            for live in self.children_of(&id) {
                let live_id = live.id().clone();
                children.push(self.list_node(live_id, Some((live, number)), now).await?);
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
                    number: record.number,
                    parent: Some(number),
                    description: record.description.clone(),
                    profile: record.profile.clone(),
                    id: record.id,
                });
            }
            Ok(match child {
                Some((child, parent)) => {
                    let record = child.node.record();
                    ListNode {
                        id,
                        number,
                        parent: Some(parent),
                        kind: EntryKind::Live,
                        description: record.description.clone(),
                        profile: record.profile.clone(),
                        phase: JobPhase::Running,
                        closed_ago_ms: None,
                        line: Some(list_line(&snapshot(&child, parent, now))),
                        children,
                    }
                }
                None => ListNode {
                    id,
                    number,
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

    /// A bounded snapshot of the live child the model knows by `number`, as
    /// JSON and as text (`subagents.md` § `demi agent show`); none for the
    /// root and for a number no live child has.
    pub(crate) fn show(&self, number: u64) -> Option<(AgentSnapshot, String)> {
        let child = self.child_numbered(number)?;
        let parent = self.node(child.parent())?.record().number;
        let now = self.clock.now().as_millisecond();
        let snapshot = snapshot(&child, parent, now);
        let text = show_text(&snapshot);
        Some((snapshot, text))
    }

    /// The live child of `owner` the model knows by `number`.
    pub(crate) fn live_child_of(&self, owner: &NodeId, number: u64) -> Option<NodeId> {
        self.child_numbered(number)
            .filter(|child| child.parent() == owner)
            .map(|child| child.id().clone())
    }

    /// The live child the model knows by `number`.
    fn child_numbered(&self, number: u64) -> Option<Rc<Child<H>>> {
        self.children
            .borrow()
            .values()
            .find(|child| child.node.record().number == number)
            .cloned()
    }

    /// The child of `owner` the model knows by `number`, live or archived;
    /// none when `owner` has no such child.
    pub(crate) async fn child_of(
        &self,
        owner: &NodeId,
        number: u64,
    ) -> Result<Option<NodeId>, String> {
        let children = self
            .store
            .children(owner)
            .await
            .map_err(|error| error.to_string())?;
        Ok(children
            .into_iter()
            .find(|record| record.number == number)
            .map(|record| record.id))
    }
}

/// A start under way past its owner's check: while it lasts, the owner is
/// not quiescent. It ends with a change of the tree, so the owner's
/// supervision looks again.
struct Starting<H: HostResolver> {
    tree: Rc<Tree<H>>,
    owner: NodeId,
}

impl<H: HostResolver> Starting<H> {
    fn new(tree: &Rc<Tree<H>>, owner: &NodeId) -> Self {
        *tree.starting.borrow_mut().entry(owner.clone()).or_default() += 1;
        Self {
            tree: tree.clone(),
            owner: owner.clone(),
        }
    }
}

impl<H: HostResolver> Drop for Starting<H> {
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
async fn supervision<H: HostResolver>(tree: Weak<Tree<H>>, child: Weak<Child<H>>) {
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
fn child_frames<H: HostResolver>(child: &Child<H>) -> [ServerFrame; 2] {
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
            number: record.number,
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
fn subagent_preamble(child: u64, parent: u64, can_spawn: bool) -> String {
    let spawning = if can_spawn {
        "`demi agent spawn` spawns your own children."
    } else {
        "This session may not spawn subagents."
    };
    [
        format!("You are a subagent: agent {child} of this conversation, spawned by agent {parent}. Your transcript starts empty; the task brief in the first user message is your entire context."),
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
        .map(|titled| demi_shared_types::trim(&titled.description).to_owned())
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
    number: u64,
    /// The parent's number; none for the root.
    parent: Option<u64>,
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
    subagent_id: u64,
    parent_session_id: Option<u64>,
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
                subagent_id: node.number,
                parent_session_id: node.parent,
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
        EntryKind::Root => format!("● {}  (root session){marker}", node.number),
        EntryKind::Archived => {
            let ago = node
                .closed_ago_ms
                .map(|ago| format!(" {} ago", duration(ago)))
                .unwrap_or_default();
            format!(
                "○ {}  archived ({}{ago})  {}",
                node.number,
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

/// A live child's line in `demi agent list`, from its snapshot.
fn list_line(snapshot: &AgentSnapshot) -> String {
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
    subagent_id: u64,
    parent_session_id: u64,
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

/// `child`'s snapshot; `parent` is its parent's number.
fn snapshot<H: HostResolver>(child: &Child<H>, parent: u64, now: i64) -> AgentSnapshot {
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
        subagent_id: record.number,
        parent_session_id: parent,
        description: record.description.clone(),
        profile: record.profile.clone(),
        phase: JobPhase::Running,
        elapsed_ms: age(now, record.started_at.as_millisecond()),
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

/// The parent's commands a child of `profile` keeps: all of them, or those
/// under the profile's paths (`subagents.md` § Profiles).
fn narrowed(profile: &Profile, commands: &CommandSet) -> CommandSet {
    match &profile.commands {
        None => commands.clone(),
        Some(kept) => commands.filter(|leaf| kept.iter().any(|path| leaf.starts_with(path))),
    }
}
