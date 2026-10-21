//! The tree store contract: how the agent keeps a conversation's tree in the
//! conversation's database.
//!
//! One store holds one conversation's tree: every node's record, with its
//! parent link, and its checkpoint. The backend realizes it over the
//! conversation's database; tests use
//! [`MemoryTreeStore`](crate::testing::MemoryTreeStore).
//!
//! Creating, saving, closing, reopening and deleting a node are each one atomic
//! commit, whatever the realization. Parent and child are related by a column,
//! never by a key path.
//!
//! Nothing about a node depends on a Host: a Host executes, the store
//! remembers, so a node is readable while its target is offline. A child is
//! rebuilt on restore and resume from what it was spawned with (its profile's
//! name, the instructions the profile added, the spawn restriction) and its
//! state row, never from the user's profiles.
//!
//! What the commits guarantee:
//!
//! - Create queues the first message in the checkpoint, so a child the process
//!   loses before its first turn still has its brief.
//! - A close follows the node's final checkpoint. A process that ends between
//!   the two leaves a quiescent live node, and the next restore closes it with
//!   the same result.
//! - A closed round stays undelivered until the parent's checkpoint carries its
//!   completion. That save marks exactly that child round delivered in the same
//!   commit, so a stale completion cannot mark a newer round delivered.
//! - A closed child is archived, never pruned: it lives exactly as long as its
//!   root, so a revivable ID stays revivable. Reopening one is a commit too,
//!   which starts the next round with the message queued.
//!
//! Beside the contract, the crate holds:
//!
//! - the media rules over the store's blob namespace (`media`);
//! - the fitting of an image as it enters a transcript (`images`);
//! - a medium's size in pixels (`pixels`);
//! - what an upload becomes (`attachments`).
//!
//! The crate depends on neither the transcript, the session nor the tools, and
//! knows no provider's runtime. The product's store decides where media bytes
//! go; this crate defines only what is stored.

pub mod attachments;
pub mod changes;
pub mod images;
pub mod media;
pub mod pixels;
#[cfg(feature = "testing")]
pub mod testing;
pub mod versions;

use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    rc::Rc,
};

use demi_conversation_socket_protocol::{JobModel, JobPhase, Subagent, TurnSpeed, Versions};
use demi_host_interface::{Seen, UnsentStart, WholeOutput};
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
    /// saved; none when the node has none.
    ///
    /// Its transcript is the window a live session holds: the current path's
    /// blocks from its last `compaction_boundary` on, or the whole path when it
    /// has none. No row before it is read.
    ///
    /// # Errors
    ///
    /// Corrupt data stops the load.
    fn load(&self) -> LocalBoxFuture<'_, Result<Option<Checkpoint>, StoreError>>;

    /// The blocks at the positions `range` of the current path, all of which it
    /// holds, with the versions at those positions.
    ///
    /// It is what a session reads of the blocks before its window when its work
    /// needs them.
    fn blocks(&self, range: Range<usize>) -> LocalBoxFuture<'_, Result<Rows, StoreError>>;

    /// The position of the block `id` on the current path; none when the
    /// path does not hold it.
    fn position<'a>(&'a self, id: &'a BlockId) -> LocalBoxFuture<'a, Result<Option<usize>, StoreError>>;

    /// The position of the current path's last block of `kind` before
    /// `end`; none when there is none.
    fn last_of(&self, kind: Kind, end: usize) -> LocalBoxFuture<'_, Result<Option<usize>, StoreError>>;

    /// The position of the current path's first block of `kind` at or
    /// after `from`; none when there is none.
    fn next_of(&self, kind: Kind, from: usize) -> LocalBoxFuture<'_, Result<Option<usize>, StoreError>>;

    /// The blocks of the current path in `range` that are of one of
    /// `kinds`, each with its position, in order, found through the index
    /// of block types: no other block of the range is read.
    fn blocks_of(&self, kinds: Vec<Kind>, range: Range<usize>) -> LocalBoxFuture<'_, Result<Vec<(usize, Block)>, StoreError>>;

    /// The position of the `user` block that opened the turn `turn` on
    /// the current path; none when the path holds none.
    fn turn_position<'a>(&'a self, turn: &'a TurnId) -> LocalBoxFuture<'a, Result<Option<usize>, StoreError>>;

    /// What the session keeps of the blocks of the current path before
    /// `end` without holding them, read in one transaction.
    fn earlier(&self, end: usize) -> LocalBoxFuture<'_, Result<EarlierRows, StoreError>>;

    /// The current path's first `length` blocks as rows a new node can start
    /// with ([`AgentTreeStore::create_node_after`]).
    ///
    /// They are named by the row at their end, so a later switch or edit of
    /// this node leaves them as they are now. None for no blocks. A node whose
    /// path starts with another node's rows names none of its blocks this way.
    fn prefix(&self, length: usize) -> LocalBoxFuture<'_, Result<Option<StoredPrefix>, StoreError>>;

    /// Makes the latest leaf of the `version`th version (from 1) at the path's
    /// position `position` the current leaf, in one commit.
    ///
    /// Answers the path from that position on. When that position is before
    /// `held_from`, where the caller's window starts, it also answers what the
    /// caller needs to hold the path from the last `compaction_boundary` at or
    /// before it. All of it is read in the switch's transaction, so nothing
    /// after its commit can fail. Nothing else of the checkpoint changes.
    ///
    /// # Errors
    ///
    /// A position the path does not reach, or a version its place does not
    /// hold, is refused without a change.
    fn switch(&self, position: usize, version: u32, held_from: usize) -> LocalBoxFuture<'_, Result<Switched, StoreError>>;

    /// The conversation owner's blob namespace, where the session stores the
    /// media that enter its transcript and reads back those its requests send.
    fn blobs(&self) -> &dyn media::BlobStore;
}

/// A conversation's tree: its node records and each node's checkpoint.
pub trait AgentTreeStore {
    fn node<'a>(
        &'a self,
        id: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Option<NodeRecord>, StoreError>>;

    /// A node's direct children in spawn order, which is the order of their
    /// numbers, live and archived alike; a side question is no child.
    fn children<'a>(
        &'a self,
        parent: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Vec<NodeRecord>, StoreError>>;

    /// Every subagent of the tree, at any depth, live and closed, in the order
    /// of their numbers.
    ///
    /// It is what a tree that opens keeps of its children
    /// (`crates/agent-server/src/server/tree.rs`). A side question is no
    /// subagent.
    fn subagents(&self) -> LocalBoxFuture<'_, Result<Vec<StoredSubagent>, StoreError>>;

    /// Every side question of the tree, live and closed, in the order they were
    /// asked.
    fn sides(&self) -> LocalBoxFuture<'_, Result<Vec<NodeRecord>, StoreError>>;

    /// The node's record and its first checkpoint in one commit, so that a
    /// node the process loses before its first turn still has the message
    /// queued in it. A node that exists is refused.
    fn create_node(
        &self,
        record: NodeRecord,
        initial: CheckpointUpdate,
    ) -> LocalBoxFuture<'_, Result<(), StoreError>>;

    /// As [`create_node`](Self::create_node), for a node whose path starts with
    /// `prefix`, rows of this tree, or with `root` of the tree of that
    /// conversation root.
    ///
    /// The rows of another conversation's tree are copied as the node's first
    /// rows, in the database, without reading them into the process. The rows
    /// of this tree are not copied: they stay their node's, the new node's path
    /// starts with them, and every read of the node's history reads them from
    /// there. They are settled, and a save of the node that would rewrite one
    /// is refused. `initial` holds the blocks after them.
    fn create_node_after<'a>(
        &'a self,
        record: NodeRecord,
        root: Option<&'a NodeId>,
        prefix: &'a StoredPrefix,
        initial: CheckpointUpdate,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>>;

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
    /// starts at `started_at`, and the reviving message, if any, queued in
    /// its checkpoint; a side question's Retry revives it with none. A
    /// `reprofile` replaces the profile, with what it chose, that the node
    /// runs on from this round on.
    fn reopen_node<'a>(
        &'a self,
        id: &'a NodeId,
        round: u64,
        started_at: Timestamp,
        message: Option<QueuedMessage>,
        reprofile: Option<Reprofile>,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>>;

    /// Changes the child `id`'s working directory to `cwd`, as `demi agent
    /// cwd` asks (`crates/agent-server/src/node/mod.rs`).
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

    /// The conversation's next number of `sequence`. The store records the
    /// number after it before it answers, so no number is given twice and a
    /// crash can only leave a gap.
    fn next_number(&self, sequence: Sequence) -> LocalBoxFuture<'_, Result<u64, StoreError>>;

    /// What the conversation holds of its command `command`, which ended
    /// (`crates/backend-database/src/command_outputs.rs`): how it ended,
    /// and its output; none for a command it does not have.
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
    /// (`crates/backend-database/src/command_outputs.rs`).
    ///
    /// The node takes them up again when it is restored
    /// (`crates/backend-remote-host/src/device_jobs.rs`).
    fn running_commands<'a>(
        &'a self,
        node: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Vec<RunningCommand>, StoreError>>;

    /// Every node that ran a command the conversation records running, in
    /// one read, so a restore finds the few closed children with commands
    /// to take up without asking each child.
    fn running_nodes(&self) -> LocalBoxFuture<'_, Result<BTreeSet<NodeId>, StoreError>>;

    /// Records that `node` has looked to `seen` in the output of the
    /// conversation's command `command`, which runs
    /// (`crates/backend-database/src/command_outputs.rs`).
    ///
    /// Nothing for a command that does not run.
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

/// A command the conversation records running.
///
/// It holds its job on its Host, the `shell` call that started it, that call's
/// title and interval, when it started, and how far the node that ran it has
/// looked in its output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningCommand {
    pub command: CommandId,
    pub job: String,
    pub tool_use_id: String,
    /// The turn of the call; none for a command recorded before records
    /// kept it.
    pub turn: Option<TurnId>,
    /// The title its reports name (`crates/agent-session/src/reports.rs`).
    pub title: String,
    /// Its call's script, which the pages' view of it carries with its
    /// title.
    pub script: String,
    /// How often its call asked it to report, or only its end when none
    /// (`crates/agent-session/src/reports.rs`): an interrupted call's command
    /// keeps it.
    pub interval_ms: Option<u32>,
    pub started: Timestamp,
    pub place: Seen,
    /// What it starts once its Host is back, while its start waits for it
    /// and was never sent (`crates/backend-host-access/src/access.rs`).
    pub unsent: Option<UnsentStart>,
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
    /// conversation's agent sequence in spawn order. A side question is no
    /// agent and takes none: 0 here, and nothing in the store.
    pub number: u64,
    pub parent: Option<NodeId>,
    /// A short title; empty for the root.
    pub description: String,
    /// The name of the profile the node was spawned with; none for the root
    /// and for a node that inherits its parent's setup.
    pub profile: Option<String>,
    /// The instructions added to its system prompt when it was spawned: its
    /// profile's, or its parent's own when it inherited.
    ///
    /// None when none were added. Every round, restore and resume read them
    /// here, never from the user's profiles or the parent now.
    pub instructions: Option<String>,
    /// The Context its profile chose, in tokens.
    ///
    /// Its sessions use it in place of the user's limit while its model's
    /// window offers it (`crates/agent-server/src/server/tree/profiles.rs`);
    /// none for the user's limit. Kept here as the instructions are, so restore
    /// and resume use what it was spawned with.
    pub context_limit: Option<u32>,
    /// The model selection a child's rounds run on: its profile's, or its
    /// parent's at its spawn. It is the one owner of what a child runs on.
    ///
    /// The checkpoint's selection is only what its history was last
    /// requested with, so a child whose record names another selection
    /// switches to this one at its next request, whatever started it.
    /// None for the root, whose model the conversation's record holds, and
    /// for a side question, which keeps its checkpoint's.
    pub model: Option<ModelSelection>,
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
    /// branch that call is on.
    ///
    /// None for the root, for a child started by no call, and for a child
    /// stored before versions existed, which is on every path.
    pub started_by: Option<BlockId>,
    /// A child's own working directory and where the conversation worked
    /// when it was set (`crates/agent-server/src/node/mod.rs`); none for the
    /// root, whose directory is always the conversation's.
    pub cwd: Option<ChildDirectory>,
    /// What a side question's node asks about; none for every agent.
    ///
    /// Its parent is the root, it works in the conversation's directory as the
    /// root does, and its can-spawn is the root's, so that its commands and its
    /// system prompt are the root's.
    pub side: Option<SideOf>,
}

/// The profile a resume moves a node to: the fields of its record a profile
/// chooses, which replace the ones the node was spawned with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reprofile {
    pub name: String,
    pub model: ModelSelection,
    pub instructions: Option<String>,
    pub context_limit: Option<u32>,
    pub can_spawn_subagents: bool,
}

/// What a side question asks about.
///
/// It holds:
///
/// - the root's text block its history ends at;
/// - the character offsets of the asked words in the block's text as the page
///   renders it;
/// - the words, as a comment keeps them;
/// - the user's question;
/// - when it was asked, the start of its first round.
///
/// A side question about a picture has the picture, and its place among the
/// block's pictures as `start` and `end`, as a comment on a picture does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideOf {
    pub block: BlockId,
    pub start: u64,
    pub end: u64,
    pub quote: String,
    pub picture: Option<demi_shared_types::MediaSource>,
    pub question: String,
    pub asked_at: Timestamp,
}

/// Where a conversation works: its primary Host's device, none for a Cloud
/// not made yet, and its directory there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub device: Option<String>,
    pub directory: String,
}

/// A child's own working directory, and the conversation's placement it was
/// set under (`crates/agent-server/src/node/mod.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildDirectory {
    pub cwd: String,
    pub under: Placement,
}

impl ChildDirectory {
    /// Where the child works while the conversation works at `now`.
    ///
    /// It is its own directory while the conversation is where it was when the
    /// directory was set, and the conversation's directory once it moved, which
    /// no write to the child follows.
    pub fn effective<'a>(&'a self, now: &'a Placement) -> &'a str {
        if self.under == *now {
            &self.cwd
        } else {
            &now.directory
        }
    }
}

impl NodeRecord {
    /// A child as the conversation's agents describe it
    /// (`crates/agent-server/src/server/tree.rs`).
    ///
    /// It runs on `model` while it is live, else as it closed, working where
    /// the conversation's placement `now` puts it; with `now` unknown, in its
    /// own directory. The root, which no node spawned, has none.
    pub fn child(&self, model: JobModel, now: Option<&Placement>, on_path: bool) -> Option<Subagent> {
        let parent = self.parent.clone()?;
        Some(Subagent {
            number: self.number,
            parent,
            description: self.description.clone(),
            profile: self.profile.clone(),
            model,
            round: self.round,
            forked_from: self.forked_from,
            state: self
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
            model: None,
            round: 1,
            started_at: now,
            can_spawn_subagents: true,
            closed: None,
            delivered: false,
            forked_from: None,
            started_by: None,
            cwd: None,
            side: None,
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
    Completed,
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
            Self::Completed => JobPhase::Completed,
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
    /// transcript, in arrival order (`crates/agent-session/src/reports.rs`).
    #[garde(dive)]
    pub reports: Vec<CommandReport>,
    /// The reports of lost resident commands, which open no turn and wait,
    /// in arrival order, for the node's next one (`crates/agent-session/src/reports.rs`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(dive)]
    pub next_turn_reports: Vec<CommandReport>,
    /// The commands the session's `shell` calls left running, each with
    /// how often it reports (`crates/agent-session/src/reports.rs`).
    #[garde(dive)]
    pub intervals: Vec<CommandInterval>,
    #[garde(dive)]
    pub model: ModelSelection,
    /// The receipts of the accepted edits.
    #[garde(dive)]
    pub edits: Vec<EditReceipt>,
    /// The account the agent took under its entry's rotation; absent before
    /// it took one and after a compaction (`crates/backend-providers/src/llm/rotation.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub account: Option<AgentAccount>,
    /// The speed of the agent's last turn that streamed for a second or
    /// more; absent before its first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub turn_speed: Option<TurnSpeed>,
}

/// A command a `shell` call left running, and how often it reports while
/// it runs: every `interval_ms`, or, for a resident command, never; its end
/// is reported either way (`crates/agent-session/src/reports.rs`).
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
/// transcript (`crates/agent-session/src/input.rs`). Its id and
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

/// The receipt of an accepted edit.
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
///
/// It holds its state and the blocks of its current path from `start` to its
/// end, with the versions at each of their places that has more than one. A
/// load starts at the path's last `compaction_boundary`; a Fork's seed holds
/// its whole path, from 0.
#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    pub state: CheckpointState,
    /// The position of `transcript`'s first block on the path.
    pub start: usize,
    pub transcript: Vec<Block>,
    /// By position on the path.
    pub versions: BTreeMap<usize, Versions>,
}

impl Checkpoint {
    /// How many blocks the path holds.
    pub fn block_count(&self) -> usize {
        self.start + self.transcript.len()
    }
}

/// Blocks of a node's current path as [`SessionStore::blocks`] reads them,
/// with the versions at their positions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rows {
    pub blocks: Vec<Block>,
    pub versions: BTreeMap<usize, Versions>,
}

/// What [`SessionStore::earlier`] reads of the blocks before a position:
/// the ids the session keeps of them, and the last block of each kind its
/// decisions read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EarlierRows {
    /// The turn of each `user` block, in order.
    pub user_turns: Vec<TurnId>,
    /// The id of each `tool_call` block, in order.
    pub calls: Vec<BlockId>,
    /// The last of [`Kind::Input`].
    pub last_input: Option<Block>,
    /// The last of [`Kind::UserInput`].
    pub last_user_input: Option<Block>,
    /// The last of [`Kind::Record`].
    pub last_record: Option<Block>,
}

/// A node's first blocks as rows a new node starts with
/// ([`SessionStore::prefix`]): the node, the row at their end, and how
/// many they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPrefix {
    pub node: NodeId,
    pub leaf: u64,
    pub length: usize,
}

/// A kind of block [`SessionStore::last_of`] looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A `compaction_boundary`, where a window starts.
    CompactionBoundary,
    /// A block of a turn's input, which names its turn: `user`, `context`,
    /// `wakeup`, `steer`, `agent_message` or `resume`.
    Input,
    /// What the user wrote: a `user` block or a `steer`.
    UserInput,
    /// A `user` block, where a request begins.
    User,
    /// A `tool_call` block.
    ToolCall,
    /// A tool call still executing.
    ExecutingCall,
    /// A `text` block.
    Text,
    /// A completed `text`, one a Fork may end at.
    CompletedText,
    /// An `error` block.
    Error,
    /// A `response` block.
    Response,
    /// A block that can be the record that ended a turn: any but a
    /// compaction's boundary and marker, an error that ended no turn, and
    /// the agent messages and command reports a turn takes after it ended.
    Record,
}

impl Kind {
    /// The `type` tags of its blocks as a row stores them.
    pub fn types(self) -> &'static [&'static str] {
        match self {
            Self::CompactionBoundary => &["compaction_boundary"],
            Self::Input => &["user", "context", "wakeup", "steer", "agent_message", "resume"],
            Self::UserInput => &["user", "steer"],
            Self::User => &["user"],
            Self::ToolCall | Self::ExecutingCall => &["tool_call"],
            Self::Text | Self::CompletedText => &["text"],
            Self::Error => &["error"],
            Self::Response => &["response"],
            Self::Record => &[
                "user", "context", "wakeup", "steer", "resume", "abort", "thinking", "redacted_thinking", "text",
                "tool_call", "response", "error", "account_switch",
            ],
        }
    }

    /// What a block of one of its [`types`](Self::types) must also hold,
    /// as an SQL condition on the block's JSON text `column`; none for a
    /// kind its types make.
    pub fn condition(self, column: &str) -> Option<String> {
        match self {
            Self::ExecutingCall => Some(format!("json_extract({column}, '$.status') = 'executing'")),
            Self::CompletedText => Some(format!("json_extract({column}, '$.forkable') IS 1")),
            Self::Record => Some(format!(
                "json_extract({column}, '$.outsideTurn') IS NOT 1
                 AND (json_extract({column}, '$.type') != 'wakeup' OR json_extract({column}, '$.placement') != 'steer')"
            )),
            _ => None,
        }
    }

    /// Whether `block` is of the kind: the block whose tag is one of
    /// [`types`](Self::types).
    pub fn holds(self, block: &Block) -> bool {
        match self {
            Self::CompactionBoundary => matches!(block, Block::CompactionBoundary(_)),
            Self::Input => matches!(
                block,
                Block::User(_)
                    | Block::Context(_)
                    | Block::Wakeup(_)
                    | Block::Steer(_)
                    | Block::AgentMessage(_)
                    | Block::Resume(_)
            ),
            Self::UserInput => matches!(block, Block::User(_) | Block::Steer(_)),
            Self::User => matches!(block, Block::User(_)),
            Self::ToolCall => matches!(block, Block::ToolCall(_)),
            Self::ExecutingCall => {
                matches!(block, Block::ToolCall(call) if call.status == demi_shared_types::ToolCallStatus::Executing)
            }
            Self::Text => matches!(block, Block::Text(_)),
            Self::CompletedText => matches!(block, Block::Text(text) if text.forkable),
            Self::Error => matches!(block, Block::Error(_)),
            Self::Response => matches!(block, Block::Response(_)),
            Self::Record => match block {
                Block::CompactionBoundary(_) | Block::CompactionMarker(_) | Block::AgentMessage(_) => false,
                Block::Error(error) => !error.outside_turn,
                Block::Wakeup(wakeup) => wakeup.placement != demi_shared_types::WakeupPlacement::Steer,
                _ => true,
            },
        }
    }
}

/// One save: only what changed since the last one.
///
/// Positions are places on the current path. A stored row is never deleted:
/// from the cut, the lowest of `moved_from` and `block_count`, the rows the
/// path held leave it, and each block from there is a new row on a new branch.
/// Below the cut, a changed block updates its row.
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

/// A subagent as the store holds it (`crates/agent-server/src/server/tree.rs`).
///
/// It holds its record and whether the call that started it is on its
/// parent's current path, which a child no call started is.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredSubagent {
    pub record: NodeRecord,
    pub started_on_path: bool,
}

impl StoredSubagent {
    /// What the subagent runs on, its record's selection.
    pub fn model(&self) -> &ModelSelection {
        self.record
            .model
            .as_ref()
            .expect("a stored subagent is a child, whose record names its model")
    }
}

/// What a switch of versions made current: the path from `from`, the place
/// whose version changed, with the versions there and after.
///
/// A block a compaction carried past its boundary shows the versions of the
/// place it was carried from, which is before it.
#[derive(Debug, Clone, PartialEq)]
pub struct Switched {
    pub from: usize,
    pub blocks: Vec<Block>,
    pub versions: BTreeMap<usize, Versions>,
    /// When `from` is before the caller's window: the path from the last
    /// `compaction_boundary` at or before `from` up to it.
    pub before: Option<SwitchedBefore>,
}

/// The path before a switch's place that the caller's window did not hold
/// ([`SessionStore::switch`]).
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchedBefore {
    /// Where the rows start: the last `compaction_boundary` at or before
    /// the place, or 0.
    pub start: usize,
    pub rows: Rows,
    /// What the session keeps of the blocks before `start`.
    pub earlier: EarlierRows,
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
