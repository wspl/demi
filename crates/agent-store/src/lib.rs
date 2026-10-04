//! The tree store contract (`runtime.md` § Tree store, `subagents.md`
//! § Persistence): how the agent keeps a conversation's tree in the
//! conversation's database. One store holds one conversation's tree: every
//! node's record, with its parent link, and its checkpoint. The backend
//! realizes it over the conversation's database; tests use
//! [`MemoryTreeStore`](crate::testing::MemoryTreeStore).
//!
//! Creating, saving, closing, reopening and deleting a node are each one
//! atomic commit, whatever the realization.
//!
//! Beside the contract: the media rules over the store's blob namespace
//! (`media`), the fitting of an image as it enters a transcript (`images`)
//! and what an upload becomes (`attachments`) (`crates-and-packages.md`
//! § agent-store).

pub mod attachments;
pub mod images;
pub mod media;
#[cfg(feature = "testing")]
pub mod testing;

use std::rc::Rc;

use demi_conversation_socket_protocol::{JobPhase, SubagentJob};
use demi_host_interface::WholeOutput;
use demi_shared_types::{
    AgentMessage, AgentMessageEvent, Block, CommandId, CompletionId, ModelSelection, NodeId,
    OperationId, QueuedMessage, Sequence, SessionPhase, Timestamp, TurnId, WakeupId,
};
use futures_util::future::LocalBoxFuture;
use serde::{Deserialize, Serialize};

/// One node's checkpoint: what its session saves and restores from, and the
/// blob namespace its media live in.
pub trait SessionStore {
    /// Commits `update` in one transaction; the update took effect exactly
    /// when this returns `Ok`.
    fn save(&self, update: CheckpointUpdate) -> LocalBoxFuture<'_, Result<(), StoreError>>;

    /// The node's checkpoint, decoded and checked, its media by reference as
    /// saved; none when the node has none. Corrupt data stops the load.
    fn load(&self) -> LocalBoxFuture<'_, Result<Option<Checkpoint>, StoreError>>;

    /// The conversation owner's blob namespace (`runtime.md` § Media), where
    /// the session stores the media that enter its transcript and reads back
    /// those its requests send.
    fn blobs(&self) -> &dyn media::BlobStore;
}

/// A conversation's tree: its node records and each node's checkpoint.
pub trait AgentTreeStore {
    fn node<'a>(
        &'a self,
        id: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Option<NodeRecord>, StoreError>>;

    /// A node's direct children in spawn order, which is the order of their
    /// numbers, live and archived alike.
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

    /// Makes a closed node live again in one commit: a new round, which
    /// starts at `started_at`, and the reviving message queued in its
    /// checkpoint.
    fn reopen_node<'a>(
        &'a self,
        id: &'a NodeId,
        round: u64,
        started_at: Timestamp,
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

    /// The conversation's next number of `sequence` (`runtime.md`
    /// § Identifiers the model sees). The store records the number after it
    /// before it answers, so no number is given twice and a crash can only
    /// leave a gap.
    fn next_number(&self, sequence: Sequence) -> LocalBoxFuture<'_, Result<u64, StoreError>>;

    /// What the conversation holds of the output of its command `command`,
    /// which ended (`storage.md` § Command outputs); none for a command it
    /// does not have.
    fn command_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<Option<StoredOutput>, StoreError>>;
}

/// How many days a conversation keeps an ended command's output
/// (`storage.md` § Retention).
pub const COMMAND_OUTPUT_DAYS: i64 = 30;

/// What a conversation holds of an ended command's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredOutput {
    Stored(WholeOutput),
    /// Why the backend could not store it.
    NotStored(String),
    /// When the retention pass removed it.
    Removed(Timestamp),
}

/// Why a store operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// What the store holds is not a valid record or checkpoint.
    #[error("the stored agent tree is corrupt: {0}")]
    Corrupt(String),
    /// The operation failed, such as a transaction the database refused.
    #[error("{0}")]
    Failed(String),
}

/// A node as the store holds it: identity and relationship, never runtime
/// state. The root has no parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRecord {
    pub id: NodeId,
    /// The number the model knows the agent by: 0 for the root, then the
    /// conversation's agent sequence in spawn order (`runtime.md`
    /// § Identifiers the model sees).
    pub number: u64,
    pub parent: Option<NodeId>,
    /// A short title; empty for the root.
    pub description: String,
    /// The name of the profile the node was spawned with; none for the root
    /// and for a node that inherits its parent's setup.
    pub profile: Option<String>,
    /// The instructions its profile replaced its parent's with when it was
    /// spawned; none when it has its parent's. Restore and resume read them
    /// here, never from the user's profiles (`subagents.md` § Persistence).
    pub instructions: Option<String>,
    /// The node's current round: 1 for its first run, one more at each
    /// resume.
    pub round: u64,
    /// When the current round started.
    pub started_at: Timestamp,
    pub can_spawn_subagents: bool,
    /// How the node closed; none while it is live.
    pub closed: Option<NodeClose>,
    /// Whether the completion of the closed round reached its parent.
    pub delivered: bool,
}

impl NodeRecord {
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
            started_at: self.started_at,
            ended_at: self.closed.as_ref().map(|close| close.at),
            result,
        })
    }

    /// The record of a conversation's root, whose id is the conversation's:
    /// agent 0, in its first round.
    pub fn root(id: NodeId, now: Timestamp) -> Self {
        Self {
            id,
            number: 0,
            parent: None,
            description: String::new(),
            profile: None,
            instructions: None,
            round: 1,
            started_at: now,
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
/// beside its transcript rows.
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
    /// The receipts of the accepted edits.
    #[garde(dive)]
    pub edits: Vec<EditReceipt>,
    /// The waiting agent messages and due wakeups wait for the user's next
    /// action: the user stopped an action that wrote nothing, or the session
    /// restored from a turn the process died in (`runtime.md` § Stop). A
    /// checkpoint saved under a turn holds them by its phase alone.
    #[garde(skip)]
    pub held: bool,
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
    /// The SHA-256 of the request's RFC 8785 canonical JSON, as the web app
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
}

/// One save: only what changed since the last one.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointUpdate {
    pub state: CheckpointState,
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
