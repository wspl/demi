//! The tree store contract (`runtime.md` § Tree store, `subagents.md`
//! § Persistence): how the agent keeps a conversation's tree in the
//! conversation's database. One store holds one conversation's tree: every
//! node's record, with its parent link, and its checkpoint. The backend
//! realizes it over the conversation's database; tests use
//! [`MemoryTreeStore`](crate::testing::MemoryTreeStore).
//!
//! Creating, saving, closing, reopening and deleting a node are each one
//! atomic commit, whatever the realization.

mod command_state;
pub mod media;

use std::rc::Rc;

use demi_agent_protocol::{JobPhase, SubagentJob};
use demi_core::{
    AgentMessage, AgentMessageEvent, Block, CompletionId, ModelSelection, NodeId, OperationId,
    QueuedMessage, SessionPhase, Timestamp, TurnId, WakeupId,
};
use futures_util::future::LocalBoxFuture;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub(crate) use command_state::CommandStateHistory;
pub use command_state::{
    BoundaryEdge, CommandStateError, CommandStateSnapshot, CommandStorageKey, CommandVersion,
    SessionBoundary,
};

/// One node's checkpoint: what its session saves and restores from.
pub trait SessionStore {
    /// Commits `update` in one transaction. Right before the transaction the
    /// store checks `guard`, after any other work such as publishing media;
    /// the update took effect exactly when this returns `Ok`.
    fn save<'a>(
        &'a self,
        update: CheckpointUpdate,
        guard: &'a CommitGuard,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>>;

    /// The node's checkpoint, decoded and checked; none when the node has
    /// none. Corrupt data stops the load.
    fn load(&self) -> LocalBoxFuture<'_, Result<Option<Checkpoint>, StoreError>>;
}

/// A conversation's tree: its node records and each node's checkpoint.
pub trait AgentTreeStore {
    fn node<'a>(
        &'a self,
        id: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Option<NodeRecord>, StoreError>>;

    /// A node's direct children in spawn order, live and archived alike.
    fn children<'a>(
        &'a self,
        parent: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Vec<NodeRecord>, StoreError>>;

    /// The node's record and its first checkpoint in one commit, so that a
    /// node the process loses before its first turn still has the message
    /// queued in it. A node that exists is refused.
    fn create_node(
        &self,
        record: NodeRecord,
        initial: CheckpointUpdate,
    ) -> LocalBoxFuture<'_, Result<(), StoreError>>;

    /// The node's checkpoint store. A save also marks delivered, in the same
    /// commit, every child completion the saved checkpoint carries
    /// ([`CheckpointUpdate::carried_completions`]).
    fn session_store(&self, id: &NodeId) -> Rc<dyn SessionStore>;

    /// Closes a node, one commit after its final checkpoint; its completion
    /// starts undelivered.
    fn close_node<'a>(
        &'a self,
        id: &'a NodeId,
        close: NodeClose,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>>;

    /// Makes a closed node live again in one commit: a new round, and the
    /// reviving message queued in its checkpoint.
    fn reopen_node<'a>(
        &'a self,
        id: &'a NodeId,
        round: u64,
        message: QueuedMessage,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>>;

    /// Marks the completion of the node's round `round` delivered, when it
    /// reached its parent by a path the parent's checkpoint cannot show. A
    /// completion of an earlier round marks nothing.
    fn mark_delivered<'a>(
        &'a self,
        id: &'a NodeId,
        round: u64,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>>;

    /// Deletes the node and every descendant with all their rows.
    fn delete_node<'a>(&'a self, id: &'a NodeId) -> LocalBoxFuture<'a, Result<(), StoreError>>;
}

/// Why a store operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// The save serves an invocation that a history rewrite or dispose made
    /// stale, so nothing was written.
    #[error("the command storage handle is no longer current")]
    Invalidated,
    /// What the store holds is not a valid record or checkpoint.
    #[error("the stored agent tree is corrupt: {0}")]
    Corrupt(String),
    /// The operation failed, such as a transaction the database refused.
    #[error("{0}")]
    Failed(String),
}

/// What a save's transaction checks right before it commits: that no
/// invocation it serves has been made stale. An ordinary checkpoint save
/// serves none.
#[derive(Debug, Clone, Default)]
pub struct CommitGuard {
    lifetimes: Vec<CancellationToken>,
}

impl CommitGuard {
    /// A guard over the lifetimes of the invocations a save serves.
    pub fn new(lifetimes: Vec<CancellationToken>) -> Self {
        Self { lifetimes }
    }

    pub fn check(&self) -> Result<(), StoreError> {
        if self.lifetimes.iter().any(CancellationToken::is_cancelled) {
            return Err(StoreError::Invalidated);
        }
        Ok(())
    }
}

/// A node as the store holds it: identity and relationship, never runtime
/// state. The root has no parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRecord {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    /// A short title; empty for the root.
    pub description: String,
    /// The profile the node was spawned with; none for the root and for a
    /// node that inherits its parent's setup.
    pub profile: Option<String>,
    /// When the node's current round started, in milliseconds since the Unix
    /// epoch; it names one execution round of the node. The tree gives every
    /// round from its clock, and a store refuses a record whose round is not
    /// a [`Timestamp`].
    pub round: u64,
    pub can_spawn_subagents: bool,
    /// How the node closed; none while it is live.
    pub closed: Option<NodeClose>,
    /// Whether the completion of the closed round reached its parent.
    pub delivered: bool,
}

impl NodeRecord {
    /// When the node's current round started.
    pub fn started_at(&self) -> Timestamp {
        i64::try_from(self.round)
            .ok()
            .and_then(|millisecond| Timestamp::from_millisecond(millisecond).ok())
            .expect("a store refuses a round that is not a time")
    }

    /// A child as the `subagent` frames and the conversation's transcript
    /// route describe it (`subagents.md` § Protocol): running while it is
    /// live, else as it closed, with the result of a completed round. The
    /// root, which no node spawned, has none.
    pub fn job(&self) -> Option<SubagentJob> {
        let parent = self.parent.clone()?;
        let result = match self.closed.as_ref().map(|close| &close.phase) {
            Some(ClosePhase::Completed { result }) => Some(result.clone()),
            _ => None,
        };
        Some(SubagentJob {
            subagent_id: self.id.clone(),
            parent_session_id: parent,
            description: self.description.clone(),
            profile: self.profile.clone(),
            phase: self
                .closed
                .as_ref()
                .map_or(JobPhase::Running, |close| close.phase.job_phase()),
            started_at: self.started_at(),
            ended_at: self.closed.as_ref().map(|close| close.at),
            result,
        })
    }

    /// The record of a conversation's root, whose id is the conversation's.
    pub fn root(id: NodeId, now: Timestamp) -> Self {
        Self {
            id,
            parent: None,
            description: String::new(),
            profile: None,
            round: u64::try_from(now.as_millisecond()).unwrap_or(0),
            can_spawn_subagents: true,
            closed: None,
            delivered: false,
        }
    }
}

/// How and when a node closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeClose {
    pub phase: ClosePhase,
    pub at: Timestamp,
}

/// The phase a node closed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClosePhase {
    /// With the child's bounded last assistant text.
    Completed {
        result: String,
    },
    Aborted,
    /// With the failure's text.
    Error {
        failure: String,
    },
}

impl ClosePhase {
    /// The phase a closed job shows.
    pub fn job_phase(&self) -> JobPhase {
        match self {
            Self::Completed { .. } => JobPhase::Completed,
            Self::Aborted => JobPhase::Aborted,
            Self::Error { .. } => JobPhase::Error,
        }
    }
}

/// The state row of a node's checkpoint: everything the session saves
/// beside its transcript rows and command state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckpointState {
    /// `running` in a checkpoint the process died in, or that dispose wrote
    /// under a running turn.
    #[garde(skip)]
    pub phase: SessionPhase,
    /// The queued messages, in the order they run.
    #[garde(dive)]
    pub queue: Vec<QueuedMessage>,
    /// The agent messages waiting for a continuation boundary, in admission
    /// order.
    #[garde(dive)]
    pub agent_inputs: Vec<PendingAgentInput>,
    /// The yield wakeups not yet written into the transcript, fired or not.
    #[garde(dive)]
    pub wakeups: Vec<ScheduledWakeup>,
    #[garde(length(min = 1))]
    pub cwd: String,
    #[garde(dive)]
    pub model: ModelSelection,
    /// The name of the harness that saved it.
    #[garde(length(min = 1))]
    pub harness: String,
    /// The receipts of the accepted edits.
    #[garde(dive)]
    pub edits: Vec<EditReceipt>,
}

/// An agent message the session admitted and has not yet written into its
/// transcript (`subagents.md` § Durable ownership and replay). Its id and
/// body live only in the message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingAgentInput {
    /// The turn it was admitted for; a continuation writes it into its own.
    #[garde(skip)]
    pub turn_id: TurnId,
    /// The model selection current at admission, which its block records.
    #[garde(dive)]
    pub model: ModelSelection,
    #[garde(dive)]
    pub message: AgentMessage,
}

/// A yield wakeup (`runtime.md` § Yield wakeups).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledWakeup {
    #[garde(skip)]
    pub id: WakeupId,
    /// How long after the scheduling action ended it fires.
    #[garde(range(min = 1))]
    pub duration_ms: u32,
    /// When it is due, in wall-clock time; null until the action that
    /// scheduled it ended.
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(skip)]
    pub due_at: Option<Timestamp>,
}

/// The receipt of an accepted edit (`message-editing.md` § Commit and
/// idempotency).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditReceipt {
    #[garde(skip)]
    pub operation_id: OperationId,
    /// The SHA-256 of the request's RFC 8785 canonical JSON, as the browser
    /// sent it, in lowercase hexadecimal.
    #[garde(pattern(r"^[0-9a-f]{64}$"))]
    pub digest: String,
    /// The replacement's turn.
    #[garde(skip)]
    pub turn_id: TurnId,
}

/// A node's checkpoint as the store gives it back.
#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    pub state: CheckpointState,
    pub transcript: Vec<Block>,
    pub command_state: CommandStateSnapshot,
}

/// One save: only what changed since the last one.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointUpdate {
    pub state: CheckpointState,
    /// The command state, when it changed; a new node's first checkpoint
    /// carries the empty initial version.
    pub command_state: Option<CommandStateSnapshot>,
    /// The changed block rows by index, ascending.
    pub changed_blocks: Vec<(usize, Block)>,
    /// How many blocks the transcript has: rows at or beyond it are deleted.
    pub block_count: usize,
}

impl CheckpointUpdate {
    /// The child rounds whose completion receipts this save carries, as
    /// waiting agent input or as `agent_message` blocks, which the save marks
    /// delivered.
    pub fn carried_completions(&self) -> Result<Vec<CompletionId>, StoreError> {
        let waiting = self.state.agent_inputs.iter().map(|input| &input.message);
        let written = self
            .changed_blocks
            .iter()
            .filter_map(|(_, block)| match block {
                Block::AgentMessage(receipt) => Some(&receipt.message),
                _ => None,
            });
        let mut rounds = Vec::new();
        for message in waiting.chain(written) {
            let AgentMessageEvent::Completion { .. } = message.event else {
                continue;
            };
            let round: CompletionId = message
                .id
                .as_str()
                .parse()
                .map_err(|error| StoreError::Corrupt(format!("{error}")))?;
            if !rounds.contains(&round) {
                rounds.push(round);
            }
        }
        Ok(rounds)
    }
}
