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
//! (`media`), the fitting of an image as it enters a transcript (`images`),
//! a medium's size in pixels (`pixels`) and what an upload becomes
//! (`attachments`) (`crates-and-packages.md` § agent-store).

pub mod attachments;
pub mod changes;
pub mod images;
pub mod media;
pub mod pixels;
#[cfg(feature = "testing")]
pub mod testing;
pub mod versions;

use std::{collections::BTreeMap, rc::Rc};

use demi_conversation_socket_protocol::{JobModel, JobPhase, SubagentChild, SubagentJob, Versions};
use demi_host_interface::{Seen, WholeOutput};
use demi_shared_types::{
    AgentAccount, AgentMessage, AgentMessageEvent, Block, BlockId, CommandEnd, CommandId, CommandReport, CompletionId, ModelSelection, NodeId,
    OperationId, QueuedMessage, Sequence, SessionPhase, Timestamp, TurnId,
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

    /// Makes the latest leaf of the `version`th version (from 1) at the
    /// path's position `position` the current leaf, in one commit, and
    /// answers the path from that position on (`message-editing.md`
    /// § Versions). Nothing else of the checkpoint changes. A position the
    /// path does not reach, or a version its place does not hold, is
    /// refused without a change.
    fn switch(&self, position: usize, version: u32) -> LocalBoxFuture<'_, Result<Switched, StoreError>>;

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

    /// Every subagent of the tree, at any depth, live and closed, in the
    /// order of their numbers, each with the model selection its checkpoint
    /// saved: what a tree that opens keeps of its children
    /// (`subagents.md` § Protocol).
    fn subagents(&self) -> LocalBoxFuture<'_, Result<Vec<StoredSubagent>, StoreError>>;

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

    /// Changes the child `id`'s working directory to `cwd`, as `demi agent
    /// cwd` asks (`subagents.md` § Working directory).
    fn set_cwd<'a>(
        &'a self,
        id: &'a NodeId,
        cwd: &'a ChildDirectory,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>>;

    /// Marks the completion of the node's round `round` delivered, when it
    /// reached its parent by a path the parent's checkpoint cannot show. A
    /// completion of an earlier round marks nothing.
    fn mark_delivered<'a>(
        &'a self,
        id: &'a NodeId,
        round: u64,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>>;

    /// The conversation's next number of `sequence` (`runtime.md`
    /// § Identifiers the model sees). The store records the number after it
    /// before it answers, so no number is given twice and a crash can only
    /// leave a gap.
    fn next_number(&self, sequence: Sequence) -> LocalBoxFuture<'_, Result<u64, StoreError>>;

    /// What the conversation holds of its command `command`, which ended
    /// (`storage.md` § Command outputs): how it ended, and its output; none
    /// for a command it does not have.
    fn command_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<Option<StoredCommand>, StoreError>>;

    /// How the conversation's command `command` ended, as its record keeps
    /// it, without its output; none for a command it has no record of.
    fn command_end<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<Option<CommandEnd>, StoreError>>;

    /// The commands `node` ran that the conversation records running
    /// (`storage.md` § Command outputs), which the node takes up again when
    /// it is restored (`sessions-and-targets.md` § Recovery and
    /// persistence).
    fn running_commands<'a>(
        &'a self,
        node: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Vec<RunningCommand>, StoreError>>;

    /// Records that `node` has looked to `seen` in the output of the
    /// conversation's command `command`, which runs (`storage.md` § Command
    /// outputs); nothing for a command that does not run.
    fn record_place<'a>(
        &'a self,
        command: &'a CommandId,
        node: &'a NodeId,
        seen: Seen,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>>;

    /// How far `node` has looked in the output of the conversation's
    /// command `command`, which runs, as the record keeps it; none when it
    /// has not looked, or the command does not run.
    fn place<'a>(
        &'a self,
        command: &'a CommandId,
        node: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Option<Seen>, StoreError>>;
}

/// A command the conversation records running: its job on its Host, the
/// `shell` call that started it, that call's title and interval, when it
/// started,
/// and how far the node that ran it has looked in its output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningCommand {
    pub command: CommandId,
    pub job: String,
    pub tool_use_id: String,
    /// The title its reports name (`runtime.md` § Command reports); empty
    /// for a command recorded before records kept it.
    pub title: String,
    /// How often its call asked it to report, or only its end when none
    /// (`runtime.md` § Command reports): an interrupted call's command
    /// keeps it.
    pub interval_ms: Option<u32>,
    pub started: Timestamp,
    pub place: Seen,
}

/// What a conversation holds of an ended command: the title of the call
/// that started it, which its reports name, empty for one recorded before
/// records kept it; how it ended; and its output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCommand {
    pub title: String,
    pub end: CommandEnd,
    pub output: StoredOutput,
}

/// What a conversation holds of an ended command's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredOutput {
    /// The output.
    Stored { output: WholeOutput },
    /// Why the backend could not store it.
    NotStored(String),
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
    /// The instructions its profile added to its system prompt when it was
    /// spawned; none when its profile added none, or it has its parent's
    /// prompt. Restore and resume read them
    /// here, never from the user's profiles (`subagents.md` § Persistence).
    pub instructions: Option<String>,
    /// The Context its profile chose, in tokens, which its sessions use in
    /// place of the user's limit while its model's window offers it
    /// (`subagents.md` § Profiles); none for the user's limit. Kept here as
    /// the instructions are, so restore and resume use what it was spawned
    /// with.
    pub context_limit: Option<u32>,
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
    /// The number of the agent it was forked from; none for one not forked.
    pub forked_from: Option<u64>,
    /// The parent's call block that started it, so the child belongs to the
    /// branch that call is on (`message-editing.md` § Versions); none for the
    /// root, for a child started by no call, and for a child stored before
    /// versions existed, which is on every path.
    pub started_by: Option<BlockId>,
    /// A child's own working directory and where the conversation worked
    /// when it was set (`subagents.md` § Working directory); none for the
    /// root, whose directory is always the conversation's.
    pub cwd: Option<ChildDirectory>,
}

/// Where a conversation works: its primary Host's device, none for a Cloud
/// not made yet, and its directory there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub device: Option<String>,
    pub directory: String,
}

/// A child's own working directory, and the conversation's placement it was
/// set under (`subagents.md` § Working directory).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildDirectory {
    pub cwd: String,
    pub under: Placement,
}

impl ChildDirectory {
    /// Where the child works while the conversation works at `now`: its own
    /// directory while the conversation is where it was when the directory
    /// was set, and the conversation's directory once it moved, which no
    /// write to the child follows.
    pub fn effective<'a>(&'a self, now: &'a Placement) -> &'a str {
        if self.under == *now {
            &self.cwd
        } else {
            &now.directory
        }
    }
}

impl NodeRecord {
    /// A child as the `subagent` frames, the open's snapshot and the
    /// conversation's subagents route describe it (`subagents.md`
    /// § Protocol): running on `model` while it is live, else as it closed,
    /// working where the conversation's placement `now` puts it; with `now`
    /// unknown, in its own directory. The root, which no node spawned, has
    /// none.
    pub fn child(&self, model: JobModel, now: Option<&Placement>, on_path: bool) -> Option<SubagentChild> {
        let parent = self.parent.clone()?;
        Some(SubagentChild {
            subagent_id: self.id.clone(),
            number: self.number,
            parent_session_id: parent,
            description: self.description.clone(),
            profile: self.profile.clone(),
            model,
            round: self.round,
            forked_from: self.forked_from,
            phase: self
                .closed
                .as_ref()
                .map_or(JobPhase::Running, |close| close.phase.job_phase()),
            started_at: self.started_at,
            ended_at: self.closed.as_ref().map(|close| close.at),
            cwd: {
                let own = self.cwd.as_ref().expect("a child's record names its directory");
                now.map_or(own.cwd.as_str(), |now| own.effective(now)).to_owned()
            },
            on_path,
        })
    }

    /// The child as [`child`](Self::child) describes it, with the result of
    /// a completed round.
    pub fn job(&self, model: JobModel, now: Option<&Placement>, on_path: bool) -> Option<SubagentJob> {
        let result = match self.closed.as_ref().map(|close| &close.phase) {
            Some(ClosePhase::Completed { result }) => Some(result.clone()),
            _ => None,
        };
        Some(SubagentJob {
            child: self.child(model, now, on_path)?,
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
            context_limit: None,
            round: 1,
            started_at: now,
            can_spawn_subagents: true,
            closed: None,
            delivered: false,
            forked_from: None,
            started_by: None,
            cwd: None,
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
    /// The command reports that arrived and are not yet written into the
    /// transcript, in arrival order (`runtime.md` § Command reports).
    #[garde(dive)]
    pub reports: Vec<CommandReport>,
    /// The reports of lost resident commands, which open no turn and wait,
    /// in arrival order, for the node's next one (`runtime.md` § Lost
    /// commands).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(dive)]
    pub next_turn_reports: Vec<CommandReport>,
    /// The commands the session's `shell` calls left running, each with
    /// how often it reports (`runtime.md` § Command reports).
    #[garde(dive)]
    pub intervals: Vec<CommandInterval>,
    #[garde(dive)]
    pub model: ModelSelection,
    /// The receipts of the accepted edits.
    #[garde(dive)]
    pub edits: Vec<EditReceipt>,
    /// The account the agent took under its entry's rotation; absent before
    /// it took one and after a compaction (`providers.md` § Account
    /// rotation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub account: Option<AgentAccount>,
}

/// A command a `shell` call left running, and how often it reports while
/// it runs: every `interval_ms`, or, for a resident command, never; its end
/// is reported either way (`runtime.md` § Command reports).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandInterval {
    #[garde(skip)]
    pub command_id: CommandId,
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(inner(range(min = 1)))]
    pub interval_ms: Option<u32>,
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

/// A node's checkpoint as the store gives it back: its state and its
/// current path, with the versions at each place of the path that has
/// more than one.
#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    pub state: CheckpointState,
    pub transcript: Vec<Block>,
    pub versions: BTreeMap<usize, Versions>,
}

/// One save: only what changed since the last one. Positions are places on
/// the current path (`message-editing.md` § Versions). A stored row is never
/// deleted: from the cut, the lowest of `moved_from` and `block_count`,
/// the rows the path held leave it, and each block from there is a new row
/// on a new branch; below the cut, a changed block updates its row.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointUpdate {
    pub state: CheckpointState,
    /// The changed blocks by position, ascending.
    pub changed_blocks: Vec<(usize, Block)>,
    /// How many blocks the path has.
    pub block_count: usize,
    /// The lowest position a block was added or inserted at, or a rewrite
    /// began at, since the last save: a stored row at or past it leaves the
    /// path. None when blocks only changed in place.
    pub moved_from: Option<usize>,
    /// Whether the branch the cut begins is a version the user sees, as an
    /// edit or Regenerate makes; otherwise it stands in for what it
    /// replaces.
    pub version: bool,
}

/// A subagent as the store holds it (`subagents.md` § Protocol): its
/// record, the model selection its checkpoint saved, and whether the call
/// that started it is on its parent's current path, which a child no call
/// started is (`message-editing.md` § Versions).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredSubagent {
    pub record: NodeRecord,
    pub model: ModelSelection,
    pub started_on_path: bool,
}

/// What a switch of versions made current: the path from `from`, the
/// place whose version changed, with the versions there and after. A block
/// a compaction carried past its boundary shows the versions of the place
/// it was carried from, which is before it (`message-editing.md`
/// § Versions).
#[derive(Debug, Clone, PartialEq)]
pub struct Switched {
    pub from: usize,
    pub blocks: Vec<Block>,
    pub versions: BTreeMap<usize, Versions>,
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
