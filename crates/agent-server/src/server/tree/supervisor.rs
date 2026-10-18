//! The supervisor operations of a conversation's tree (`subagents.md`): the
//! agent directory of its live nodes; starting children (spawn, fork, resume
//! and restore) and closing them (a natural completion, a failure or an abort);
//! delivering their completions; messages between agents; and the `list`
//! and `show` reads. Every node supervises its own children the same way;
//! only the root's lifecycle is its client's.

use tracing::Instrument as _;
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use demi_agent_session::{
    AgentMessageError, AgentSession, Execution, ForkError, ModelSwitch, SessionEvent, Settle,
    Subscription,
};
use demi_agent_store::{Checkpoint, ChildDirectory, ClosePhase, NodeClose, NodeRecord, Placement};
use demi_agent_transcript::{CutError, ForkEnd, last_answer, last_failure};
use demi_agent_tools::HostResolver;
use demi_conversation_socket_protocol::{
    JobModel, JobPhase, ServerFrame, SubagentEvent, SubagentJob, SubagentSummary, TranscriptPatch, index_u32,
    latest_page_start,
};
use demi_provider_common::ProviderRuntime;
use demi_shared_gates::Purpose;
use demi_shared_types::{
    AgentMessage, AgentMessageEvent, Block, BlockId, CommandId, CompletionId, CompletionOutcome,
    ModelSelection, NodeId, QueuedMessage, Sender, Sequence, Timestamp, ToolCallBlock,
    ToolCallStatus, TurnId, UserContentBlock,
    output::{self, Fields, Table, seconds},
};
use demi_host_interface::{ShellEnvironment, relative_to};
use futures_util::future::{LocalBoxFuture, join_all};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};
use tokio_util::task::AbortOnDropHandle;

use super::{StoppingShells, Tree, cut_path, note_activity, unended_views};
use super::profiles::Limits;
use crate::{
    Node,
    node::{self, Instructions, NodeRole, NodeSpec, Origin},
    server::{AgentServer, commands::with_runtime_groups, stored_fork_seed},
};

/// The bound of a completion's result and of `show`'s last assistant text,
/// in UTF-8 bytes.
const RESULT_MAX_BYTES: usize = 32 * 1024;
/// The most recent tool calls `show` lists.
const SHOW_RECENT_TOOLS: usize = 8;
/// Why the root's directory does not change (`subagents.md` § Working
/// directory).
const ROOT_CWD: &str = "The main agent works in the conversation's directory; move the conversation with `demi host move` to work elsewhere.";
/// Why a side question starts no subagent, sends no message and keeps its
/// directory (`comments-and-ask.md` § Side questions).
const SIDE_SPAWNS: &str = "a side question cannot start subagents";
const SIDE_SENDS: &str = "a side question cannot send messages to agents";
const SIDE_CWD: &str = "a side question works in the conversation's directory and cannot change it";
pub(super) const OWNER_CLOSING: &str = "owner session is closing";

/// A child of the tree, live or closed, as an open's snapshot lists it
/// (`subagents.md` § Protocol).
pub(crate) struct Summary {
    pub(super) record: NodeRecord,
    pub(super) runs_on: JobModel,
    /// Its transcript's revision when it closed; a live child's is its
    /// session's, and one read from the store has none it can tell, 0.
    pub(super) revision: u64,
    /// Whether the call that started it is on its parent's current path,
    /// as it was when the parent last was live; a live parent's session
    /// tells it now (`message-editing.md` § Versions).
    pub(super) started_on_path: bool,
    /// Whether the pages last heard that it is on the current path; none
    /// before a change of a path told them.
    pub(super) shown_on_path: Option<bool>,
}

/// A live child: its node, and what its supervision keeps about it.
pub(crate) struct Child<H: HostResolver> {
    node: Rc<Node<H>>,
    /// What its round runs on, resolved when the round started.
    runs_on: JobModel,
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

    fn parent(&self) -> NodeId {
        self.node.parent().expect("a child's record names its parent")
    }

    /// Its place among its siblings: spawn order, its number's.
    fn rank(&self) -> u64 {
        self.node.record().number
    }

    /// Its job, as `record`, its record now or as it closed, describes it,
    /// while the conversation works at `now`.
    fn job(&self, record: &NodeRecord, now: Option<&Placement>, on_path: bool) -> SubagentJob {
        record
            .job(self.runs_on.clone(), now, on_path)
            .expect("a child has a job")
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
        /// `--cwd`, read against the caller's directory; none for the
        /// caller's own.
        cwd: Option<String>,
    },
    Resume {
        id: NodeId,
        message: String,
    },
    /// `demi agent fork`: the caller's child the model knows by `source`.
    Fork {
        source: u64,
        message: String,
        profile_name: Option<String>,
        description: Option<String>,
    },
}

/// How a child closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloseKind {
    Completed,
    Aborted,
    Error,
}

/// Why a child's provider runtime could not be had.
pub(super) enum RuntimeError {
    /// The owner is closing, so it forks nothing.
    OwnerClosing,
    /// The product could not build a runtime for the child's provider
    /// entry, with why.
    Unbuilt(String),
}

impl RuntimeError {
    pub(super) fn reason(self) -> String {
        match self {
            Self::OwnerClosing => OWNER_CLOSING.to_owned(),
            Self::Unbuilt(reason) => reason,
        }
    }
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

    /// The agent `id` while it takes messages: the root, or a live child
    /// that is not closing.
    pub(crate) fn live_agent(&self, id: &NodeId) -> Option<Rc<Node<H>>> {
        if id == self.root.id() {
            return Some(self.root.clone());
        }
        self.child(id)
            .filter(|child| !child.closing.get())
            .map(|child| child.node.clone())
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
            .filter(|child| &child.parent() == owner)
            .cloned()
            .collect();
        children.sort_by_key(|child| child.rank());
        children
    }

    /// The live children of `owner` that are not closing, in spawn order,
    /// each by its agent number with its description: what a compaction of
    /// `owner` names as still working (`compaction.md` § What the model
    /// receives afterward).
    pub(crate) fn live_children_of(&self, owner: &NodeId) -> Vec<(u64, String)> {
        self.children_of(owner)
            .into_iter()
            .filter(|child| !child.closing.get())
            .map(|child| {
                let record = child.node.record();
                (record.number, record.description.clone())
            })
            .collect()
    }

    /// Whether `child` is a live child of `owner`.
    pub(crate) fn is_child_of(&self, child: &NodeId, owner: &NodeId) -> bool {
        self.child(child)
            .is_some_and(|child| &child.parent() == owner)
    }

    /// The node that owns a start: the caller, live and not closing.
    fn owner(&self, caller: &NodeId) -> Result<Rc<Node<H>>, String> {
        if self.disposing.get() {
            return Err(OWNER_CLOSING.to_owned());
        }
        if caller == self.root.id() {
            return Ok(self.root.clone());
        }
        if self.is_side(caller) {
            return Err(SIDE_SPAWNS.to_owned());
        }
        match self.child(caller) {
            Some(child) if !child.closing.get() => Ok(child.node.clone()),
            Some(_) => Err(OWNER_CLOSING.to_owned()),
            None => Err("this session is not in the agent directory".to_owned()),
        }
    }

    /// How many levels below the root `node` is: the root's children are
    /// at 1. A closed ancestor's record is the store's.
    async fn depth(&self, node: &Node<H>) -> Result<u32, String> {
        let mut depth = 0;
        let mut parent = node.record().parent.clone();
        while let Some(id) = parent {
            depth += 1;
            parent = if &id == self.root.id() {
                None
            } else if let Some(child) = self.child(&id) {
                child.node.record().parent.clone()
            } else {
                self.store
                    .node(&id)
                    .await
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("subagent {id} is not in the store"))?
                    .parent
            };
        }
        Ok(depth)
    }

    /// Check 5 of a spawn (`subagents.md` § Profiles): `owner` is above the
    /// deepest level the depth limit allows.
    async fn check_depth(&self, owner: &Node<H>, limits: Limits) -> Result<(), String> {
        let Some(max) = limits.max_depth else {
            return Ok(());
        };
        if self.depth(owner).await? >= max.get() {
            return Err(format!(
                "this agent is at the deepest level of subagents Settings > Subagents allows ({max}); do this work in this session."
            ));
        }
        Ok(())
    }

    /// Check 6 of a spawn, and of a resume: the user is under the limit of
    /// live subagents across all the conversations of `server`, the user's
    /// shard, counting in each tree the starts that passed this check and
    /// have not yet joined its directory. The slot answered holds the new
    /// child's place until it joins the directory or its start fails.
    fn reserve_live(
        self: &Rc<Self>,
        server: &AgentServer<H>,
        limits: Limits,
    ) -> Result<LiveSlot<H>, String> {
        if let Some(max) = limits.max_live {
            let here = self.live_count();
            // This tree is in the map while it starts a child: a disposing
            // tree starts none.
            let elsewhere: usize = server
                .trees
                .borrow()
                .values()
                .filter(|tree| !Rc::ptr_eq(tree, self))
                .map(|tree| tree.live_count())
                .sum();
            let live = here + elsewhere;
            if live >= max.get() as usize {
                return Err(format!(
                    "{live} subagents are live across your conversations, {here} of them in this one, and Settings > Subagents allows {max}; wait for one to finish, or do this work in this session."
                ));
            }
        }
        self.admitted.set(self.admitted.get() + 1);
        Ok(LiveSlot { tree: self.clone() })
    }

    /// The live subagents of this tree, at every depth, with the starts
    /// that hold a place among them.
    fn live_count(&self) -> usize {
        self.children.borrow().len() + self.admitted.get()
    }

    /// Starts a child of `caller` for `demi agent spawn`, `fork` or `resume`
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
                    cwd,
                } => {
                    tree.spawn(&owner, prompt, profile_name, description, cwd)
                        .await
                }
                StartInput::Resume { id, message } => tree.reopen(&owner, id, message).await,
                StartInput::Fork {
                    source,
                    message,
                    profile_name,
                    description,
                } => {
                    tree.fork(&owner, source, message, profile_name, description)
                        .await
                }
            }
        }
        .in_current_span());
        started
            .await
            .map_err(|error| format!("the start of a subagent failed: {error}"))?
    }

    /// Spawns a child of `owner` with the next agent number and `prompt`
    /// queued, under the user's current subagent settings and the profile
    /// `profile_name`, none for the inherit profile, working in `owner`'s
    /// directory or the one `cwd` names against it. A check that fails
    /// creates nothing. Answers its number.
    async fn spawn(
        self: &Rc<Self>,
        owner: &Rc<Node<H>>,
        prompt: String,
        profile_name: Option<String>,
        description: String,
        cwd: Option<String>,
    ) -> Result<u64, String> {
        if !owner.record().can_spawn_subagents {
            return Err("this session may not spawn subagents".to_owned());
        }
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        let setup = self
            .spawn_setup(&server, owner, profile_name.as_deref())
            .await?;
        let now = owner.placement().await.map_err(|error| error.to_string())?;
        let cwd = match cwd {
            Some(dir) => owner.existing_directory(&dir, &now).await?,
            None => owner.directory_at(&now),
        };
        self.check_depth(owner, setup.limits).await?;
        let slot = self.reserve_live(&server, setup.limits)?;
        let runtime = self
            .child_runtime(&server, owner, &setup.model)
            .await
            .map_err(RuntimeError::reason)?;
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
            instructions: setup.instructions,
            context_limit: setup.context_limit,
            round: 1,
            started_at: self.clock.now(),
            can_spawn_subagents: setup.can_spawn,
            closed: None,
            delivered: false,
            forked_from: None,
            started_by: owner.session().executing_call(),
            cwd: Some(ChildDirectory { cwd, under: now }),
            side: None,
        };
        let origin = Origin::New {
            record,
            model: setup.model,
            first_message: Some(self.text_message(prompt)),
        };
        self.start_child(&server, owner, origin, runtime, None, Some(slot))
            .await?;
        Ok(number)
    }

    /// Forks the child of `owner` the model knows by `source`
    /// (`subagents.md` § Fork): a new child of `owner` with the next agent
    /// number, whose history is the source's through its newest completed
    /// text, and whose next user message is `message`. The checks are a
    /// spawn's, and the profile `profile_name`, none for the inherit profile,
    /// sets what it runs on; `description` defaults to the source's with
    /// ` (fork)`. Its first request switches from the source's
    /// model to that profile's as a model switch does, compacting first with
    /// the source's model when the history is over the new model's
    /// threshold. The source is untouched. Answers its number.
    async fn fork(
        self: &Rc<Self>,
        owner: &Rc<Node<H>>,
        source: u64,
        message: String,
        profile_name: Option<String>,
        description: Option<String>,
    ) -> Result<u64, String> {
        if !owner.record().can_spawn_subagents {
            return Err("this session may not spawn subagents".to_owned());
        }
        let Some(source) = self.child_record_of(owner.id(), source).await? else {
            return Err(format!(
                "no subagent {source} of yours to fork (see `demi agent list`)"
            ));
        };
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        let setup = self
            .spawn_setup(&server, owner, profile_name.as_deref())
            .await?;
        let mut seed = self.fork_seed(&source).await?;
        // A fork starts where its parent works, as a spawn does.
        let now = owner.placement().await.map_err(|error| error.to_string())?;
        let cwd = ChildDirectory {
            cwd: owner.directory_at(&now),
            under: now,
        };
        self.check_depth(owner, setup.limits).await?;
        let slot = self.reserve_live(&server, setup.limits)?;
        // The child starts on the source's model, which compacts what the
        // profile's model cannot hold, and switches at its first request.
        let from = seed.state.model.clone();
        let runtime = self
            .child_runtime(&server, owner, &from)
            .await
            .map_err(RuntimeError::reason)?;
        let switch = if setup.model == from {
            None
        } else {
            let runtime = if setup.model.provider_id == from.provider_id {
                None
            } else {
                let built = self.child_runtime(&server, owner, &setup.model).await;
                Some(built.map_err(RuntimeError::reason)?)
            };
            Some(ModelSwitch {
                model: Box::new(setup.model),
                runtime,
            })
        };
        let number = self
            .store
            .next_number(Sequence::Agent)
            .await
            .map_err(|error| error.to_string())?;
        let description = description.unwrap_or_else(|| {
            format!("{} (fork)", source.description)
                .trim_start()
                .to_owned()
        });
        let record = NodeRecord {
            id: self.new_node_id(),
            number,
            parent: Some(owner.id().clone()),
            description,
            profile: profile_name,
            instructions: setup.instructions,
            context_limit: setup.context_limit,
            round: 1,
            started_at: self.clock.now(),
            can_spawn_subagents: setup.can_spawn,
            closed: None,
            delivered: false,
            forked_from: Some(source.number),
            started_by: owner.session().executing_call(),
            cwd: Some(cwd),
            side: None,
        };
        seed.state.queue.push(self.text_message(message));
        let origin = Origin::Forked {
            record,
            checkpoint: seed,
        };
        self.start_child(&server, owner, origin, runtime, switch, Some(slot))
            .await?;
        Ok(number)
    }

    /// The history a fork of the child `source` starts from: captured from
    /// its live session in one step, or read from its archived checkpoint
    /// (`subagents.md` § Fork).
    async fn fork_seed(&self, source: &NodeRecord) -> Result<Checkpoint, String> {
        let seed = match self.child(&source.id) {
            Some(child) => child.node.session().prepare_fork(ForkEnd::Newest),
            None => stored_fork_seed(self.store.as_ref(), &source.id, ForkEnd::Newest).await,
        };
        seed.map_err(|error| match error {
            ForkError::Target(CutError::NothingCompleted) => format!(
                "Agent {} has no completed message to fork from yet",
                source.number
            ),
            error => error.to_string(),
        })
    }

    /// The provider runtime of a child of `owner` that infers with `model`
    /// (`subagents.md` § Runtime): a fork of the owner's runtime when that
    /// serves the model's provider entry, else one the product builds for
    /// the entry. Either way the runtime is the child's own, and its session
    /// closes it.
    pub(super) async fn child_runtime(
        &self,
        server: &AgentServer<H>,
        owner: &Node<H>,
        model: &ModelSelection,
    ) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        let fork = owner
            .session()
            .fork_runtime_for(&model.provider_id)
            .await
            .map_err(|_| RuntimeError::OwnerClosing)?;
        match fork {
            Some(runtime) => Ok(runtime),
            None => server
                .deps
                .providers
                .runtime(self.root.id(), model)
                .await
                .map_err(|error| RuntimeError::Unbuilt(error.to_string())),
        }
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
        if !record.delivered {
            return Err("The previous completion is not saved by the parent yet; retry resume after receiving it".to_owned());
        }
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        let slot = self.reserve_live(&server, self.limits(&server).await?)?;
        // The runtime comes first: a child whose provider entry is gone
        // stays archived.
        let archived = self.stored(&id).await?;
        let runtime = self
            .child_runtime(&server, owner, archived.model())
            .await
            .map_err(RuntimeError::reason)?;
        let round = record.round + 1;
        let started_at = self.clock.now();
        self.store
            .reopen_node(&id, round, started_at, Some(self.text_message(message)))
            .await
            .map_err(|error| error.to_string())?;
        // The reopen queued the message in its checkpoint.
        let reopened = self.stored(&id).await?;
        self.start_child(&server, owner, reopened, runtime, None, Some(slot))
            .await?;
        Ok(number)
    }

    /// What the store holds of the child `id`, which it must hold.
    async fn stored(&self, id: &NodeId) -> Result<Origin, String> {
        Origin::stored(self.store.as_ref(), id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("subagent {id} is not in the store"))
    }

    pub(super) fn new_node_id(&self) -> NodeId {
        NodeId::try_from(self.ids.next_id()).expect("an id source never gives an empty id")
    }

    pub(super) fn text_message(&self, text: String) -> QueuedMessage {
        QueuedMessage {
            comments: Vec::new(),
            id: TurnId::try_from(self.ids.next_id()).expect("an id source never gives an empty id"),
            content: vec![UserContentBlock::Text { text }],
            automation: None,
        }
    }

    /// Everything spawn, fork, reopen and restore share: the child's node
    /// from the assembly, new with its first message queued in the create
    /// commit or from the store, on `runtime`, with what it was spawned
    /// with; then the model `switch` a fork lands at its first request, its
    /// place in the directory and its frames, what it has yet to run, its
    /// own children, and its supervision. A start's `slot` ends once the
    /// child is in the directory, which counts it from then on; a restore
    /// takes none, since it brings back what was live.
    async fn start_child(
        self: &Rc<Self>,
        server: &Rc<AgentServer<H>>,
        owner: &Rc<Node<H>>,
        origin: Origin,
        runtime: Box<dyn ProviderRuntime>,
        switch: Option<ModelSwitch>,
        slot: Option<LiveSlot<H>>,
    ) -> Result<NodeId, String> {
        let deps = &server.deps;
        // A fork's round runs on the model it switches to at its first
        // request.
        let model = switch.as_ref().map_or(origin.model(), |switch| &switch.model);
        let runs_on = deps.subagents.runs_on(model).await;
        let activity = self.admission.enter(Purpose::Demand).await;
        let record = origin.record();
        // A profile's child has the product's prompt with what its profile
        // added; the inherit profile's copies its parent's whole.
        let instructions = match &record.profile {
            Some(_) => Instructions {
                identity: deps.instructions.clone(),
                added: record.instructions.as_deref().map(Rc::from),
            },
            None => owner.instructions().clone(),
        };
        let inherited = owner.inherited_commands().clone();
        let can_spawn = record.can_spawn_subagents;
        let commands = with_runtime_groups(&inherited, server, can_spawn)
            .map_err(|error| error.to_string())?;
        let preamble = subagent_preamble(record.number, owner.record().number, can_spawn);
        let feed = self.live.feed(Some(record.id.clone()));
        let assembled = node::assemble(NodeSpec {
            origin,
            role: NodeRole::Child,
            root: self.root.id().clone(),
            server: Rc::downgrade(server),
            runtime,
            providers: deps.providers.clone(),
            hosts: deps.hosts.clone(),
            instructions,
            guide: deps.guide.clone(),
            preamble: Some(preamble),
            context: deps.context.clone(),
            inherited,
            commands: Rc::new(commands),
            store: self.store.clone(),
            shells: deps.shells.clone(),
            feed,
            admission: self.admission.clone(),
            ids: deps.ids.clone(),
            clock: deps.clock.clone(),
            config: deps.config.session,
            interval_floor_ms: deps.config.interval_floor_ms,
            call_window_ms: deps.config.call_window_ms,
        })
        .await
        .map_err(|error| error.to_string())?;
        if let Some(switch) = switch
            && let Err(refused) = assembled.node.session().update_model(switch)
        {
            // Only a closing session refuses a switch, and one just
            // assembled is not closing; its runtime is closed all the same.
            refused.discard().await;
        }
        let placement = self.placement().await;
        let child = self.attach_child(assembled.node, runs_on, placement.as_ref());
        drop(slot);
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

    /// Whether `record`'s child belongs to the conversation's current path
    /// (`message-editing.md` § Versions): the call that started it is on
    /// its parent's current path, and so is each ancestor's. A live
    /// parent's session tells whether it holds the call; a closed one's
    /// path is the one it closed with. A child no call started is on every
    /// path.
    fn on_path(&self, record: &NodeRecord) -> bool {
        let summaries = self.summaries.borrow();
        let mut at = Some(record.clone());
        while let Some(child) = at {
            let Some(parent) = &child.parent else {
                return true;
            };
            if let Some(call) = &child.started_by {
                let started = match self.node(parent) {
                    Some(live) => live.session().holds_block(call),
                    None => summaries
                        .get(&child.number)
                        .is_none_or(|summary| summary.started_on_path),
                };
                if !started {
                    return false;
                }
            }
            at = summaries
                .values()
                .find(|summary| &summary.record.id == parent)
                .map(|summary| summary.record.clone());
        }
        true
    }

    /// A path of the tree changed, by an edit, a Regenerate, a switch, a
    /// retry or any rewrite: each child whose place on the current path
    /// changed with it, at any depth, is told to the pages.
    pub(crate) async fn paths_changed(&self) {
        let records: Vec<NodeRecord> = self
            .summaries
            .borrow()
            .values()
            .map(|summary| summary.record.clone())
            .collect();
        let mut changed = Vec::new();
        for record in records {
            let on_path = self.on_path(&record);
            let mut summaries = self.summaries.borrow_mut();
            let Some(summary) = summaries.get_mut(&record.number) else {
                continue;
            };
            if summary.shown_on_path.unwrap_or(true) != on_path {
                changed.push((record, summary.runs_on.clone(), on_path));
            }
            summary.shown_on_path = Some(on_path);
        }
        if changed.is_empty() {
            return;
        }
        let placement = self.placement().await;
        for (record, runs_on, on_path) in changed {
            if let Some(job) = record.job(runs_on, placement.as_ref(), on_path) {
                self.sink.emit(ServerFrame::Subagent {
                    event: SubagentEvent::Changed,
                    job,
                });
            }
        }
    }

    /// The child `id` leaves the directory: whether each of its own
    /// children's start is on its path now stays as it closed.
    fn keep_started_on_path(&self, id: &NodeId, session: &AgentSession) {
        for summary in self.summaries.borrow_mut().values_mut() {
            if summary.record.parent.as_ref() == Some(id)
                && let Some(call) = &summary.record.started_by
            {
                summary.started_on_path = session.holds_block(call);
            }
        }
    }

    /// The conversation works elsewhere now: each live child's page hears
    /// where the child works, which its record and the new placement give
    /// (`subagents.md` § Working directory).
    pub(crate) async fn placement_changed(&self) {
        let now = self.placement().await;
        let children: Vec<Rc<Child<H>>> = self.children.borrow().values().cloned().collect();
        for child in children {
            self.sink.emit(ServerFrame::Subagent {
                event: SubagentEvent::Changed,
                job: child.job(&child.node.record(), now.as_ref(), self.on_path(&child.node.record())),
            });
        }
    }

    /// Where the conversation works now; none, with the failure logged,
    /// when it cannot be read, and a child's frame then names its own
    /// directory.
    pub(crate) async fn placement(&self) -> Option<Placement> {
        match self.root.placement().await {
            Ok(now) => Some(now),
            Err(error) => {
                tracing::warn!(root = %self.root.id(), %error, "where the conversation works could not be read");
                None
            }
        }
    }

    /// The working directory of the live agent `caller`: a child's own, the
    /// conversation's for the root.
    pub(crate) async fn directory_of(&self, caller: &NodeId) -> Result<String, String> {
        let node = self
            .live_agent(caller)
            .or_else(|| self.side(caller).map(|side| side.node().clone()))
            .ok_or("this session is not in the agent directory")?;
        let now = node.placement().await.map_err(|error| error.to_string())?;
        Ok(node.directory_at(&now))
    }

    /// Moves the live child `caller` to `dir`, read against its directory,
    /// from its next `shell` call on, and tells the pages; the root works
    /// in the conversation's directory, and a directory the primary Host
    /// does not have changes nothing (`subagents.md` § Working directory).
    /// Answers the new directory.
    pub(crate) async fn change_cwd(&self, caller: &NodeId, dir: &str) -> Result<String, String> {
        if caller == self.root.id() {
            return Err(ROOT_CWD.to_owned());
        }
        if self.is_side(caller) {
            return Err(SIDE_CWD.to_owned());
        }
        let child = self
            .child(caller)
            .ok_or("this session is not in the agent directory")?;
        let now = child.node.placement().await.map_err(|error| error.to_string())?;
        let cwd = child.node.existing_directory(dir, &now).await?;
        child
            .node
            .set_cwd(cwd.clone(), now.clone())
            .await
            .map_err(|error| error.to_string())?;
        let record = child.node.record();
        self.remember(&record, &child.runs_on, 0);
        self.sink.emit(ServerFrame::Subagent {
            event: SubagentEvent::Changed,
            job: child.job(&record, Some(&now), self.on_path(&record)),
        });
        Ok(cwd)
    }

    /// Registers a new live child, sends its `started` frame, where it works
    /// while the conversation works at `placement`, and its transcript, and
    /// forwards its session's changes from then on.
    fn attach_child(&self, node: Node<H>, runs_on: JobModel, placement: Option<&Placement>) -> Rc<Child<H>> {
        let node = Rc::new(node);
        let now = self.clock.now().as_millisecond();
        let child = Rc::new_cyclic(|child: &Weak<Child<H>>| {
            let events = node.session().subscribe({
                let child = child.clone();
                let sink = self.sink.clone();
                let clock = self.clock.clone();
                let id = node.id().clone();
                let paths = self.paths.clone();
                let activity = self.activity.clone();
                move |event| {
                    cut_path(&paths, event);
                    note_activity(&activity, event);
                    match event {
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
                        SessionEvent::PendingCallsChanged { pending_calls } => {
                            sink.emit(ServerFrame::PendingCalls {
                                subagent_id: Some(id.clone()),
                                pending_calls: pending_calls.clone(),
                            });
                        }
                        SessionEvent::WaitingChanged { waited_ms } => {
                            sink.emit(ServerFrame::Waiting {
                                subagent_id: Some(id.clone()),
                                waited_ms: *waited_ms,
                            });
                        }
                        SessionEvent::PhaseChanged { phase } => {
                            sink.emit(ServerFrame::Phase {
                                subagent_id: Some(id.clone()),
                                phase: *phase,
                            });
                        }
                        SessionEvent::ContextUsageChanged { usage } => {
                            sink.emit(ServerFrame::ContextUsage {
                                node: id.clone(),
                                usage: Some(usage.clone()),
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
                }
            });
            Child {
                node: node.clone(),
                runs_on,
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
        self.remember(&child.node.record(), &child.runs_on, 0);
        self.bump();
        for frame in child_frames(&child, placement, self.on_path(&child.node.record())) {
            self.sink.emit(frame);
        }
        child
    }

    /// Brings `owner`'s children back from the store (`subagents.md`
    /// § Persistence) from what each was spawned with, never from the
    /// user's profiles: a closed child whose completion never reached the
    /// owner is delivered now, and a live one is restored and continues. A
    /// live one whose provider runtime cannot be built, or whose stored
    /// state does not rebuild, closes as an error with the reason, its rows
    /// kept.
    pub(super) fn restore_children<'a>(
        self: &'a Rc<Self>,
        owner: &'a Rc<Node<H>>,
    ) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(server) = self.server.upgrade() else {
                return;
            };
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
                    self.take_up_stopping(&record).await;
                    if !record.delivered {
                        let origin = self.stored_origin(&record.id, &close.phase).await;
                        self.deliver(owner, &record, close, origin).await;
                    }
                    continue;
                }
                let id = record.id.clone();
                let origin = match self.stored(&id).await {
                    Ok(origin) => origin,
                    Err(error) => {
                        self.close_unrebuilt(owner, record, error).await;
                        continue;
                    }
                };
                let runtime = match self.child_runtime(&server, owner, origin.model()).await {
                    Ok(runtime) => runtime,
                    // The owner is going away; the child stays live in the
                    // store for the next restore.
                    Err(RuntimeError::OwnerClosing) => return,
                    Err(RuntimeError::Unbuilt(reason)) => {
                        let runs_on = server.deps.subagents.runs_on(origin.model()).await;
                        self.close_unbuilt(owner, record, runs_on, reason).await;
                        continue;
                    }
                };
                if let Err(error) = self.start_child(&server, owner, origin, runtime, None, None).await {
                    self.close_unrebuilt(owner, record, error).await;
                }
            }
        })
    }

    /// Takes up the commands the closed child `record` and its closed
    /// descendants still ran when the backend restarted or the tree was
    /// disposed (`subagents.md` § Abort): restore skips the children, not
    /// their commands, which show as stopping again, are asked to stop
    /// again, and end as their runners report.
    fn take_up_stopping<'a>(self: &'a Rc<Self>, record: &'a NodeRecord) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            let feed = self.live.feed(Some(record.id.clone()));
            let shells = self
                .root
                .take_up_closed(record, feed, |command| self.environment_of(command).is_some())
                .await;
            self.keep_stopping(record, shells);
            let children = match self.store.children(&record.id).await {
                Ok(children) => children,
                Err(error) => {
                    self.report(format!("the children of {} were not read: {error}", record.id));
                    return;
                }
            };
            for child in children.iter().filter(|child| child.closed.is_some()) {
                self.take_up_stopping(child).await;
            }
        })
    }

    /// Closes as failed the live child `record` of `owner` whose stored
    /// state does not rebuild, with what failed, `error`: its rows stay as
    /// they are, and its own live children close as an abort closes them
    /// (`message-editing.md` § Versions: a child is never deleted).
    async fn close_unrebuilt(&self, owner: &Node<H>, record: NodeRecord, error: String) {
        tracing::warn!(node = %record.id, %error, "a live subagent could not be rebuilt and closes as failed");
        let runs_on = self
            .summaries
            .borrow()
            .get(&record.number)
            .map(|summary| summary.runs_on.clone());
        let Some(runs_on) = runs_on else {
            self.report(format!("subagent {} has no summary and stays live in the store", record.id));
            return;
        };
        self.close_unbuilt(owner, record, runs_on, error).await;
    }

    /// Closes the stored live child `record` of `owner`, whose round runs
    /// on `runs_on` and which no provider runtime can be built for, as an
    /// error with `reason`: its live descendants close as aborted first, as
    /// an abort closes them, and `owner` receives the failed completion
    /// (`subagents.md` § Persistence).
    async fn close_unbuilt(
        &self,
        owner: &Node<H>,
        record: NodeRecord,
        runs_on: JobModel,
        reason: String,
    ) {
        self.abort_stored_children(&record.id).await;
        let close = NodeClose {
            phase: ClosePhase::Error { failure: reason },
            at: self.clock.now(),
        };
        if let Err(error) = self.store.close_node(&record.id, close.clone()).await {
            self.report(format!("subagent {} did not close: {error}", record.id));
            return;
        }
        let closed = NodeRecord {
            closed: Some(close.clone()),
            ..record
        };
        self.remember(&closed, &runs_on, 0);
        let placement = self.placement().await;
        self.sink.emit(ServerFrame::Subagent {
            event: SubagentEvent::Closed,
            job: closed
                .job(runs_on, placement.as_ref(), self.on_path(&closed))
                .expect("a child has a job"),
        });
        let origin = self.stored_origin(&closed.id, &close.phase).await;
        self.deliver(owner, &closed, &close, origin).await;
    }

    /// The block of the stored transcript of the closed child `id` its
    /// completion names, by how it closed `phase`: its last answer, or a
    /// failed child's failure record; none when it has none, or when its
    /// transcript does not load, which only costs the receipt its link.
    async fn stored_origin(&self, id: &NodeId, phase: &ClosePhase) -> Option<BlockId> {
        match self.store.session_store(id).load().await {
            Ok(checkpoint) => checkpoint.and_then(|checkpoint| match phase {
                ClosePhase::Error { .. } => last_failure(&checkpoint.transcript).map(|error| error.id.clone()),
                ClosePhase::Completed { .. } | ClosePhase::Aborted => {
                    last_answer(&checkpoint.transcript, 0).map(|text| text.id.clone())
                }
            }),
            Err(error) => {
                tracing::warn!(node = %id, %error, "a closed subagent's transcript did not load for its completion");
                None
            }
        }
    }

    /// Closes every live descendant the store holds of `parent`, deepest
    /// first, as aborted. Their completions stay undelivered in the store,
    /// for the round of `parent` that next restores its children.
    fn abort_stored_children<'a>(&'a self, parent: &'a NodeId) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            let children = match self.store.children(parent).await {
                Ok(children) => children,
                Err(error) => {
                    self.report(format!("the children of {parent} were not closed: {error}"));
                    return;
                }
            };
            for child in children.into_iter().filter(|child| child.closed.is_none()) {
                self.abort_stored_children(&child.id).await;
                let close = NodeClose {
                    phase: ClosePhase::Aborted,
                    at: self.clock.now(),
                };
                if let Err(error) = self.store.close_node(&child.id, close).await {
                    self.report(format!("subagent {} did not close: {error}", child.id));
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
    /// closes (`subagents.md` § Result): a message accepted before this step
    /// keeps it open, one sent after it is refused. A child is quiescent
    /// once nothing is left for it to do: no action runs or waits, no agent
    /// message or command report waits to be read, no child of its own is
    /// live and no command it started still runs. It closes with its last
    /// answer.
    fn decide(&self, child: &Child<H>) -> Decision {
        if child.closing.get() || self.disposing.get() {
            return Decision::Stop;
        }
        let kind = if child.failure.borrow().is_some() {
            CloseKind::Error
        } else {
            let session = child.node.session();
            let status = session.status();
            let quiescent = status.settle == Settle::Settled
                && !status.input
                && !status.commands
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
        let owner = self.node(&child.parent());
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
            // Its commands are asked to stop too, and the close does not
            // wait for their ends, which only their runners report: a
            // runner that is away, or a job it cannot end, would hold the
            // close as long. They stay stopping meanwhile.
            let shells = child.node.stop_shells().await;
            self.keep_stopping(&record, shells);
            // A start of its own under way ends first, so its child is
            // closed with the others.
            let _turn = self.starts.acquire(child.id().clone()).await;
            Box::pin(self.abort_children_of(child.id())).await;
        }
        // The completion names the block of the child's transcript it comes
        // from: the last answer, or a failed child's failure record.
        let answer = session.last_answer();
        let (phase, origin) = match kind {
            CloseKind::Completed => (
                ClosePhase::Completed {
                    result: bounded(answer.as_ref().map_or("", |(_, text)| text)).to_owned(),
                },
                answer.map(|(id, _)| id),
            ),
            CloseKind::Aborted => (ClosePhase::Aborted, answer.map(|(id, _)| id)),
            CloseKind::Error => (
                ClosePhase::Error {
                    failure: child.failure.borrow().clone().unwrap_or_default(),
                },
                session.last_failure(),
            ),
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
        let closed = NodeRecord {
            closed: Some(close.clone()),
            ..record.clone()
        };
        self.keep_started_on_path(&record.id, session);
        self.children.borrow_mut().remove(&record.id);
        // In the step it leaves the directory: an open's snapshot lists it
        // closed from then on, before its `closed` frame goes.
        if committed.is_ok() {
            self.remember(&closed, &child.runs_on, session.transcript_version().revision);
        }
        self.bump();
        match committed {
            Ok(()) => {
                let placement = self.placement().await;
                self.sink.emit(ServerFrame::Subagent {
                    event: SubagentEvent::Closed,
                    job: child.job(&closed, placement.as_ref(), self.on_path(&closed)),
                });
                if let Some(owner) = &owner {
                    self.deliver(owner, &record, &close, origin).await;
                }
            }
            // The child stays live and quiescent in the store: the next
            // restore closes it again.
            Err(error) => self.report(format!("subagent {} did not close: {error}", record.id)),
        }
        child.closed.send_replace(true);
    }

    /// Keeps the shells of the closed or closing child `record` while a
    /// command of theirs has not ended, and forgets them once each has
    /// (`subagents.md` § Abort).
    fn keep_stopping(self: &Rc<Self>, record: &NodeRecord, environments: Vec<Rc<dyn ShellEnvironment>>) {
        let commands: Vec<(Rc<dyn ShellEnvironment>, CommandId)> = environments
            .iter()
            .flat_map(|environment| {
                unended_views(environment.as_ref()).map(|view| (environment.clone(), view.command_id))
            })
            .collect();
        if commands.is_empty() {
            return;
        }
        let key = self.next_stopping.get();
        self.next_stopping.set(key + 1);
        let tree = Rc::downgrade(self);
        let ends = tokio::task::spawn_local(async move {
            // An environment that forgot a command answers an error: it
            // has ended either way.
            join_all(
                commands
                    .iter()
                    .map(|(environment, command)| environment.ended(command)),
            )
            .await;
            if let Some(tree) = tree.upgrade() {
                let shells = tree.stopping.borrow_mut().remove(&key);
                drop(shells);
            }
        });
        self.stopping.borrow_mut().insert(
            key,
            StoppingShells {
                subagent: record.id.clone(),
                environments,
                _ends: AbortOnDropHandle::new(ends),
            },
        );
    }

    /// Hands a closed round's completion, which comes from the block
    /// `origin` of the child's transcript, to its parent; the parent's save
    /// marks it delivered. A parent that is closing leaves it undelivered
    /// for its restore.
    async fn deliver(
        &self,
        owner: &Node<H>,
        record: &NodeRecord,
        close: &NodeClose,
        origin: Option<BlockId>,
    ) {
        let message = completion(record, close, origin);
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
    /// message is accepted durably. The message comes from the caller's call
    /// `call`, the `shell` call whose command sends it.
    pub(crate) async fn send_message(
        &self,
        caller: &NodeId,
        call: &str,
        target: &str,
        content: String,
    ) -> Result<u64, String> {
        if self.is_side(caller) {
            return Err(SIDE_SENDS.to_owned());
        }
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
            sender: Some(Sender {
                id: caller.clone(),
                number: sender.record().number,
                description,
                round: sender.record().round,
            }),
            recipient_id: recipient_id.clone(),
            timestamp: self.clock.now(),
            content,
            event: AgentMessageEvent::Message {},
            origin: sender.session().call_block(call),
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
    /// Each works where the conversation's placement `placement` puts it.
    pub(super) fn replay(&self, placement: Option<&Placement>) -> Vec<ServerFrame> {
        self.descendants()
            .iter()
            .flat_map(|child| child_frames(child, placement, self.on_path(&child.node.record())))
            .collect()
    }

    /// The summary of every child, in the order of their numbers, while the
    /// conversation works at `placement`: a live child's revision is its
    /// session's now.
    pub(super) fn subagent_summaries(&self, placement: Option<&Placement>) -> Vec<SubagentSummary> {
        let children = self.children.borrow();
        self.summaries
            .borrow()
            .values()
            .map(|summary| {
                let child = summary
                    .record
                    .child(summary.runs_on.clone(), placement, self.on_path(&summary.record))
                    .expect("a subagent's record has a child");
                let revision = children.get(&summary.record.id).map_or(summary.revision, |live| {
                    live.node.session().transcript_version().revision
                });
                SubagentSummary { child, revision }
            })
            .collect()
    }

    /// Every child as [`subagent_summaries`](Self::subagent_summaries)
    /// lists it, each with the result of a completed round: what the
    /// conversation's subagent list answers while the tree is live, so the
    /// open's `subagents` frame never takes back a child the list showed
    /// (`web-application.md` § Opening a conversation shows in two steps).
    pub fn subagent_jobs(&self, placement: Option<&Placement>) -> Vec<SubagentJob> {
        self.summaries
            .borrow()
            .values()
            .map(|summary| {
                summary
                    .record
                    .job(summary.runs_on.clone(), placement, self.on_path(&summary.record))
                    .expect("a subagent's record has a child")
            })
            .collect()
    }

    /// Keeps `record` as its child's summary now, with what its round runs
    /// on and, once it closed, its transcript's last revision.
    fn remember(&self, record: &NodeRecord, runs_on: &JobModel, revision: u64) {
        let mut summaries = self.summaries.borrow_mut();
        // What the pages heard of its place, and whether its start is on
        // its parent's path, stay; a new child's start is.
        let (started_on_path, shown_on_path) = summaries
            .get(&record.number)
            .map_or((true, None), |summary| (summary.started_on_path, summary.shown_on_path));
        summaries.insert(
            record.number,
            Summary {
                record: record.clone(),
                runs_on: runs_on.clone(),
                revision,
                started_on_path,
                shown_on_path,
            },
        );
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
        let placement = self.root.placement().await.map_err(|error| error.to_string())?;
        let root = self
            .list_node(self.root.id().clone(), None, &placement, now)
            .await?;
        Ok(Listing { root })
    }

    /// The node `id` with its children: the root when `child` is none, else
    /// that live child with its parent's number.
    fn list_node<'a>(
        &'a self,
        id: NodeId,
        child: Option<(Rc<Child<H>>, u64)>,
        placement: &'a Placement,
        now: i64,
    ) -> LocalBoxFuture<'a, Result<ListNode, String>> {
        Box::pin(async move {
            let number = match &child {
                Some((child, _)) => child.node.record().number,
                None => self.root.record().number,
            };
            let mut children = Vec::new();
            for live in self.children_of(&id) {
                let live_id = live.id().clone();
                children.push(
                    self.list_node(live_id, Some((live, number)), placement, now)
                        .await?,
                );
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
                    phase: Some(close.phase.job_phase()),
                    started_at: Some(record.started_at),
                    closed_at: Some(close.at),
                    live: None,
                    children: Vec::new(),
                    number: record.number,
                    parent: Some(number),
                    description: record.description.clone(),
                    profile: record.profile.clone(),
                    place: record.cwd.as_ref().map(|own| place(placement, own)),
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
                        phase: Some(JobPhase::Running),
                        started_at: Some(record.started_at),
                        closed_at: None,
                        place: Some(place(placement, record.cwd.as_ref().expect("a child's record names its directory"))),
                        live: Some(self.snapshot(&child, parent, now, placement)),
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
                    phase: None,
                    started_at: None,
                    closed_at: None,
                    place: None,
                    live: None,
                    children,
                },
            })
        })
    }

    /// A bounded snapshot of the child the model knows by `number`, live or
    /// archived, as JSON and as text (`subagents.md` § `demi agent show`);
    /// none for the root, for a child whose start is under way and for a
    /// number no child of the tree has.
    pub(crate) async fn show(&self, number: u64) -> Result<Option<(ShownAgent, String)>, String> {
        let now = self.clock.now().as_millisecond();
        let placement = self.root.placement().await.map_err(|error| error.to_string())?;
        if let Some(child) = self.child_numbered(number) {
            let Some(parent) = self.node(&child.parent()) else {
                return Ok(None);
            };
            let snapshot = self.snapshot(&child, parent.record().number, now, &placement);
            let text = show_text(&snapshot);
            return Ok(Some((ShownAgent::Live(snapshot), text)));
        }
        let Some((record, parent)) = self.stored_child(number).await? else {
            return Ok(None);
        };
        let Some(close) = record.closed else {
            return Ok(None);
        };
        let (result, failure) = match close.phase {
            ClosePhase::Completed { ref result } => (Some(bounded(result).to_owned()), None),
            ClosePhase::Error { ref failure } => (None, Some(bounded(failure).to_owned())),
            ClosePhase::Aborted => (None, None),
        };
        let archived = ArchivedSnapshot {
            id: record.number,
            parent,
            description: record.description,
            profile: record.profile,
            status: close.phase.job_phase(),
            started_at: record.started_at,
            closed_at: close.at,
            result,
            failure,
            cwd: record
                .cwd
                .as_ref()
                .expect("a child's record names its directory")
                .effective(&placement)
                .to_owned(),
        };
        let text = archived_text(&archived);
        Ok(Some((ShownAgent::Archived(archived), text)))
    }

    /// The stored child the model knows by `number`, live or archived, with
    /// its parent's number: the tree read from the root down, as `list`
    /// reads it.
    async fn stored_child(&self, number: u64) -> Result<Option<(NodeRecord, u64)>, String> {
        let root = self.root.record();
        let mut owners = vec![(root.id.clone(), root.number)];
        while let Some((owner, owner_number)) = owners.pop() {
            let children = self
                .store
                .children(&owner)
                .await
                .map_err(|error| error.to_string())?;
            for child in children {
                if child.number == number {
                    return Ok(Some((child, owner_number)));
                }
                owners.push((child.id, child.number));
            }
        }
        Ok(None)
    }

    /// The live child of `owner` the model knows by `number`.
    pub(crate) fn live_child_of(&self, owner: &NodeId, number: u64) -> Option<NodeId> {
        self.child_numbered(number)
            .filter(|child| &child.parent() == owner)
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
        let record = self.child_record_of(owner, number).await?;
        Ok(record.map(|record| record.id))
    }

    /// The record of the child of `owner` the model knows by `number`, live
    /// or archived.
    async fn child_record_of(
        &self,
        owner: &NodeId,
        number: u64,
    ) -> Result<Option<NodeRecord>, String> {
        let children = self
            .store
            .children(owner)
            .await
            .map_err(|error| error.to_string())?;
        Ok(children.into_iter().find(|record| record.number == number))
    }
}

/// A start's place among the user's live subagents, from the check
/// of the live limit until its child is in the directory or the start
/// fails ([`Tree::reserve_live`]).
struct LiveSlot<H: HostResolver> {
    tree: Rc<Tree<H>>,
}

impl<H: HostResolver> Drop for LiveSlot<H> {
    fn drop(&mut self) {
        let admitted = &self.tree.admitted;
        admitted.set(admitted.get() - 1);
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
    // A start holds the child's waiting input until this first look, which
    // closes a child that is quiescent once its own children are back,
    // before its input could open a turn (`subagents.md` § Persistence).
    let mut held = true;
    loop {
        status.mark_unchanged();
        changes.mark_unchanged();
        {
            let (Some(live_tree), Some(live_child)) = (tree.upgrade(), child.upgrade()) else {
                return;
            };
            match live_tree.decide(&live_child) {
                Decision::Wait if held => {
                    held = false;
                    live_child.node.session().release();
                }
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
/// records the stop, then the waiting ones.
pub(super) async fn stop_all(session: &AgentSession) {
    while session.abort().await.target.is_some() {}
}

/// A child's `started` frame, its transcript, its phase, the calls it is
/// writing and its wait for the model.
fn child_frames<H: HostResolver>(child: &Child<H>, placement: Option<&Placement>, on_path: bool) -> [ServerFrame; 5] {
    let session = child.node.session();
    let transcript = session.transcript();
    [
        ServerFrame::Subagent {
            event: SubagentEvent::Started,
            job: child.job(&child.node.record(), placement, on_path),
        },
        {
            // The page holds none of a child's blocks until it shows it
            // (`rendering.md` § Where a reset starts).
            let start = latest_page_start(&transcript.blocks);
            ServerFrame::SubagentTranscriptReset {
                subagent_id: child.id().clone(),
                start: index_u32(start),
                length: index_u32(transcript.blocks.len()),
                blocks: transcript.blocks[start..].to_vec(),
                revision: transcript.version.revision,
                failures: None,
            }
        },
        ServerFrame::Phase {
            subagent_id: Some(child.id().clone()),
            phase: session.phase(),
        },
        ServerFrame::PendingCalls {
            subagent_id: Some(child.id().clone()),
            pending_calls: session.pending_calls(),
        },
        ServerFrame::Waiting {
            subagent_id: Some(child.id().clone()),
            waited_ms: session.waited_ms(),
        },
    ]
}

/// A closed round's completion message (`subagents.md` § Message
/// identity): its id names the round, and its body is the result, or the
/// failure text, which can be empty; it comes from the block `origin` of
/// the child's transcript.
fn completion(record: &NodeRecord, close: &NodeClose, origin: Option<BlockId>) -> AgentMessage {
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
        sender: Some(Sender {
            id: record.id.clone(),
            number: record.number,
            description: record.description.clone(),
            round: record.round,
        }),
        recipient_id: record
            .parent
            .clone()
            .expect("a child's record names its parent"),
        timestamp: close.at,
        content,
        event: AgentMessageEvent::Completion { outcome },
        origin,
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
        "A turn you end with your answer returns that answer to the parent as the result and ends the session once nothing you started still runs: no queued or unread messages or command reports, no running children or commands of your own. To wait for your commands or children, end your turn: their end wakes you. Write the answer for the parent agent, in the shape the task brief asked for.".to_owned(),
        "Report a command you leave running for your parent, such as a server, to the parent at once with `demi agent send parent`: name what runs and its command number, which the parent stops with `demi shell stop <commandId>` when done. Your result waits until that command ends.".to_owned(),
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

/// The instant `millisecond`, a time the tree's clock gave.
fn at(millisecond: i64) -> Timestamp {
    // The clock's own times are inside the range of times.
    Timestamp::from_millisecond(millisecond).unwrap_or(Timestamp::UNIX_EPOCH)
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
    /// A child's phase, how an archived one closed; none for the root.
    phase: Option<JobPhase>,
    started_at: Option<Timestamp>,
    closed_at: Option<Timestamp>,
    /// A child's working directory as the listing names it
    /// ([`place`]); none for the root.
    place: Option<String>,
    /// A live child's snapshot.
    live: Option<AgentSnapshot>,
    children: Vec<ListNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum EntryKind {
    Root,
    Live,
    Archived,
}

/// `demi agent list`'s tree.
pub(crate) struct Listing {
    root: ListNode,
}

/// One agent of `demi agent list --json`: the table's row and more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TreeEntry {
    id: u64,
    /// Null for the root.
    parent: Option<u64>,
    kind: EntryKind,
    /// A child's phase, how an archived one closed; null for the root.
    status: Option<JobPhase>,
    /// Null for the parent's.
    profile: Option<String>,
    description: String,
    /// A child's working directory, relative to the conversation's when
    /// inside it; null for the root.
    cwd: Option<String>,
    /// Null for the root.
    started_at: Option<Timestamp>,
    /// A live child's facts; null otherwise.
    execution: Option<Execution>,
    activity: Option<String>,
    elapsed_ms: Option<u64>,
    last_event_at: Option<Timestamp>,
    /// Milliseconds since the last event.
    idle_ms: Option<u64>,
    execution_for_ms: Option<u64>,
    /// An archived child's close; null otherwise.
    closed_at: Option<Timestamp>,
    /// Whether it is the caller.
    #[serde(rename = "self")]
    is_caller: bool,
}

impl Listing {
    /// The nodes from the root down, each node's archived children after
    /// its live ones; the archived only with `all`.
    pub(crate) fn entries(&self, caller: &NodeId, all: bool) -> Vec<TreeEntry> {
        let mut entries = Vec::new();
        let mut stack = vec![&self.root];
        while let Some(node) = stack.pop() {
            if node.kind == EntryKind::Archived && !all {
                continue;
            }
            let live = node.live.as_ref();
            entries.push(TreeEntry {
                id: node.number,
                parent: node.parent,
                kind: node.kind,
                status: node.phase,
                profile: node.profile.clone(),
                description: node.description.clone(),
                cwd: node.place.clone(),
                started_at: node.started_at,
                execution: live.map(|live| live.execution),
                activity: live.map(|live| live.activity.clone()),
                elapsed_ms: live.map(|live| live.elapsed_ms),
                last_event_at: live.map(|live| live.last_event_at),
                idle_ms: live.map(|live| live.idle_ms),
                execution_for_ms: live.map(|live| live.execution_for_ms),
                closed_at: node.closed_at,
                is_caller: &node.id == caller,
            });
            stack.extend(node.children.iter().rev());
        }
        entries
    }
}

/// `demi agent list`'s table of `entries`.
pub(crate) fn list_text(entries: &[TreeEntry]) -> String {
    let mut table = Table::new([
        "id",
        "parent",
        "status",
        "profile",
        "execution",
        "elapsed (s)",
        "idle (s)",
        "description",
    ]);
    for entry in entries {
        table.row::<output::Cell>([
            entry.id.into(),
            entry.parent.into(),
            entry.status.map(|phase| phase.to_string()).into(),
            entry.profile.as_deref().into(),
            entry.execution.map(Execution::as_str).into(),
            entry.elapsed_ms.map(seconds).into(),
            entry.idle_ms.map(seconds).into(),
            entry.description.as_str().into(),
        ]);
    }
    table.render()
}

/// `demi agent show --json`'s agent: its instants, and its spans in
/// milliseconds up to the query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSnapshot {
    id: u64,
    parent: u64,
    description: String,
    profile: Option<String>,
    status: JobPhase,
    started_at: Timestamp,
    elapsed_ms: u64,
    last_event_at: Timestamp,
    /// Milliseconds since the last event.
    idle_ms: u64,
    execution: Execution,
    activity: String,
    /// How long the current execution state has lasted.
    execution_for_ms: u64,
    tools: Vec<ToolSnapshot>,
    last_assistant_text: String,
    last_assistant_text_at: Option<Timestamp>,
    /// Its working directory on the primary Host.
    cwd: String,
}

/// `demi agent show --json`'s agent: a live child's snapshot, or what an
/// archived child closed with; an archived child's status is how it closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum ShownAgent {
    Live(AgentSnapshot),
    Archived(ArchivedSnapshot),
}

/// An archived child as `demi agent show --json` gives it: how and when it
/// closed, and its result or failure with the bound of a completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArchivedSnapshot {
    id: u64,
    parent: u64,
    description: String,
    profile: Option<String>,
    /// How it closed: `completed`, `error` or `aborted`.
    status: JobPhase,
    started_at: Timestamp,
    closed_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<String>,
    /// Its working directory on the primary Host as it closed.
    cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ToolSnapshot {
    title: String,
    status: ToolStatus,
    started_at: Timestamp,
    duration_ms: u64,
    /// Null while it runs.
    ended_at: Option<Timestamp>,
}

impl<H: HostResolver> Tree<H> {
    /// `child`'s snapshot; `parent` is its parent's number. An idle child
    /// whose own children still run waits, as one whose commands do.
    /// It works where `placement`, the conversation's now, puts it.
    fn snapshot(&self, child: &Child<H>, parent: u64, now: i64, placement: &Placement) -> AgentSnapshot {
        let execution = match child.node.session().execution() {
            Execution::Idle if !self.children_of(child.id()).is_empty() => Execution::Waiting,
            execution => execution,
        };
        snapshot(child, parent, now, execution, placement)
    }
}

/// `child`'s snapshot in `execution`; `parent` is its parent's number.
fn snapshot<H: HostResolver>(
    child: &Child<H>,
    parent: u64,
    now: i64,
    execution: Execution,
    placement: &Placement,
) -> AgentSnapshot {
    let record = child.node.record();
    let session = child.node.session();
    let telemetry = child.telemetry.borrow();
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
        id: record.number,
        parent,
        description: record.description.clone(),
        profile: record.profile.clone(),
        status: JobPhase::Running,
        started_at: record.started_at,
        elapsed_ms: age(now, record.started_at.as_millisecond()),
        last_event_at: at(telemetry.last_event_at),
        idle_ms: age(now, telemetry.last_event_at),
        execution,
        activity,
        execution_for_ms: age(now, execution_since),
        tools: telemetry
            .tools
            .iter()
            .map(|tool| ToolSnapshot {
                title: tool.title.clone(),
                status: tool.status,
                started_at: at(tool.started_at),
                duration_ms: age(tool.ended_at.unwrap_or(now), tool.started_at),
                ended_at: tool.ended_at.map(at),
            })
            .collect(),
        last_assistant_text: bounded(session.last_answer().as_ref().map_or("", |(_, text)| text))
            .to_owned(),
        last_assistant_text_at: telemetry.last_text_at.map(at),
        cwd: record
            .cwd
            .as_ref()
            .expect("a child's record names its directory")
            .effective(placement)
            .to_owned(),
    }
}

/// `demi agent show`'s text of a live child.
fn show_text(snapshot: &AgentSnapshot) -> String {
    let mut fields = Fields::new();
    fields
        .field("ID", snapshot.id)
        .field("Parent", snapshot.parent)
        .field("Status", snapshot.status.to_string())
        .field("Profile", snapshot.profile.as_deref())
        .field("Description", snapshot.description.as_str())
        .field("Directory", snapshot.cwd.as_str())
        .field("Started", snapshot.started_at)
        .field("Elapsed (s)", seconds(snapshot.elapsed_ms))
        .field("Execution", snapshot.execution.as_str())
        .field("Execution for (s)", seconds(snapshot.execution_for_ms))
        .field("Last event", snapshot.last_event_at)
        .field("Idle (s)", seconds(snapshot.idle_ms))
        .field("Activity", snapshot.activity.as_str());
    let mut tools = Table::new(["status", "started", "duration (s)", "ended", "title"]);
    for tool in &snapshot.tools {
        tools.row::<output::Cell>([
            tool.status.as_str().into(),
            tool.started_at.into(),
            seconds(tool.duration_ms).into(),
            tool.ended_at.into(),
            tool.title.as_str().into(),
        ]);
    }
    fields
        .table("Recent tool calls", &tools)
        .field("Last assistant text at", snapshot.last_assistant_text_at)
        .text("Last assistant text", &snapshot.last_assistant_text);
    fields.render()
}

/// `demi agent show`'s text of an archived child.
fn archived_text(archived: &ArchivedSnapshot) -> String {
    let mut fields = Fields::new();
    fields
        .field("ID", archived.id)
        .field("Parent", archived.parent)
        .field("Status", archived.status.to_string())
        .field("Profile", archived.profile.as_deref())
        .field("Description", archived.description.as_str())
        .field("Directory", archived.cwd.as_str())
        .field("Started", archived.started_at)
        .field("Closed", archived.closed_at);
    match (&archived.result, &archived.failure) {
        (Some(result), _) => fields.text("Result", result),
        (None, Some(failure)) => fields.text("Failure", failure),
        (None, None) => &mut fields,
    };
    fields.render()
}

/// A child's working directory as `list` names it while the conversation
/// works at `placement`: relative to the conversation's directory when
/// inside it, `.` for that directory itself, else whole.
fn place(placement: &Placement, own: &ChildDirectory) -> String {
    let cwd = own.effective(placement);
    relative_to(&placement.directory, cwd).unwrap_or_else(|| cwd.to_owned())
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
