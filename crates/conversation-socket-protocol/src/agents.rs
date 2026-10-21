//! A conversation's agents as its metadata lists them: the root, each subagent
//! and each side question, every one a node of its tree and an agent session
//! (`docs/decisions/0009-agent-sessions-and-sync.md`).
//!
//! The list is a collection kept in sync by its [`Version`]: the `agents`
//! frame carries it whole, or the entries changed since a revision the page
//! holds, and each `agent` frame one changed entry with the revision one past
//! the previous. A page that meets a gap asks for the changes since the
//! revision it holds (`agents_since`). An entry carries no result: a child's
//! result is its transcript's last answer, and its parent's transcript holds
//! it as delivered.
//!
//! An entry also carries the agent's waiting inputs, its queue and its
//! pending steers, so a command that changes them settles on the entry, never
//! on its answer (`docs/decisions/0023-a-command-settles-on-the-state-it-changed.md`).

use demi_shared_types::{
    BlockId, ContextUsage, MAX_SAFE_INTEGER, MediaSource, NodeId, Nullable, PendingCall, PendingSteer, QueuedMessage,
    SessionPhase, Timestamp, TurnId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{AgentCost, JobModel, JobPhase, Version};

/// One agent of the conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct AgentEntry {
    /// Its node's id; the root's is the conversation's.
    #[garde(skip)]
    pub id: NodeId,
    #[garde(dive)]
    pub role: AgentRole,
    /// Its session's phase; idle for one that is not live.
    #[garde(skip)]
    pub phase: SessionPhase,
    /// How long it had waited for the model's answer when the entry left,
    /// or null when it waits for none. The page counts from its own clock
    /// when the entry arrives, minus the wait, so a difference between the
    /// two clocks never shows. It lives while the session does and is never
    /// kept.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u64>")]
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub waited_ms: Option<u64>,
    /// The action whose start began the wait: a message's id, or the id the
    /// request of a resume, a retry or a side question's follow-up named,
    /// when the action started as it was admitted. The page that sent it
    /// counts the wait from its own press, through every delivery of the
    /// entry. Null while it waits for none, for a wait within a turn, as
    /// after a call's result or a steer sent now, and for an action that
    /// waited behind another.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<TurnId>")]
    #[garde(skip)]
    pub wait_action: Option<TurnId>,
    /// The calls its model is writing, while its session lives; never kept.
    #[garde(dive)]
    pub pending_calls: Vec<PendingCall>,
    /// The estimate of its next request with the window its thresholds use;
    /// null for one that is not live, or that cannot tell it without
    /// reading a blob, so no figure from before shows.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ContextUsage>")]
    #[garde(dive)]
    pub context: Option<ContextUsage>,
    /// What its own ledger rows cost, kept as they are written.
    #[garde(dive)]
    pub cost: AgentCost,
    /// The messages waiting for its turn to end, in the order they run;
    /// empty for one that is not live. Never kept.
    #[garde(dive)]
    pub queue: Vec<QueuedMessage>,
    /// The steers it accepted that wait for its next step, in order; empty
    /// for one that is not live.
    #[garde(dive)]
    pub pending_steers: Vec<PendingSteer>,
    /// The message its running turn started with, until the turn writes it
    /// into the transcript; null otherwise. Never kept.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<QueuedMessage>")]
    #[garde(dive)]
    pub starting: Option<QueuedMessage>,
}

/// What an agent is in its tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentRole {
    /// The main agent, agent 0.
    Root,
    Subagent(#[garde(dive)] Subagent),
    Side(#[garde(dive)] SideQuestion),
}

/// A subagent: who started it, on what, and where its round is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct Subagent {
    /// The number the model knows it by, which `demi agent spawn` printed
    /// and a message links it with as `agent:<number>`.
    #[garde(range(min = 1, max = MAX_SAFE_INTEGER))]
    pub number: u64,
    /// The node that spawned it.
    #[garde(skip)]
    pub parent: NodeId,
    /// The spawn's `--description`, or empty.
    #[garde(skip)]
    pub description: String,
    /// The profile's name; null for the inherit profile.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub profile: Option<String>,
    /// What the round runs on, from the profile or the parent.
    #[garde(dive)]
    pub model: JobModel,
    /// The round: 1 for the spawn's, one more at each resume.
    #[garde(range(min = 1, max = MAX_SAFE_INTEGER))]
    pub round: u64,
    /// The number of the agent it was forked from; null for a child that
    /// was not forked.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u64>")]
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub forked_from: Option<u64>,
    /// Where its round is: running, or how it closed.
    #[garde(skip)]
    pub state: JobPhase,
    /// When the round started.
    #[garde(skip)]
    pub started_at: Timestamp,
    /// When it closed; null while it runs.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    #[garde(skip)]
    pub ended_at: Option<Timestamp>,
    /// Its working directory on the primary Host now, where its `shell`
    /// calls start (`crates/agent-server/src/node/mod.rs`).
    #[garde(length(min = 1))]
    pub cwd: String,
    /// Whether the call that started it is on its parent's current path:
    /// the Agents window and `agent:` links show only the children that
    /// are.
    #[garde(skip)]
    pub on_path: bool,
}

/// A side question: where it asks, what, and how its card shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct SideQuestion {
    /// The root's text block the asked words are in.
    #[garde(skip)]
    pub block_id: BlockId,
    /// The asked words' character offsets in the block's text as the page
    /// renders it, as a comment's.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub start: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub end: u64,
    /// The asked words; a picture's alt text or name for a side question
    /// about a picture.
    #[garde(skip)]
    pub quote: String,
    /// The picture a side question about a picture asks of, whose place
    /// among the block's pictures `start` and `end` are, as a comment on a
    /// picture's; absent for words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub picture: Option<MediaSource>,
    /// The first question; a follow-up is a `user` block of its transcript.
    #[garde(skip)]
    pub question: String,
    /// The small model's title; null until it wrote one, when the card says
    /// *Ask*.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub title: Option<String>,
    /// Whether its card is folded into the margin's mark.
    #[garde(skip)]
    pub folded: bool,
    #[garde(skip)]
    pub asked_at: Timestamp,
}

/// The blocks of a node's transcript a page holds, as it names them when it
/// subscribes: the version it holds them at, and the first block of its
/// latest run that may have changed since, at `from`, as `edge`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HeldEdge {
    #[garde(dive)]
    pub version: Version,
    #[garde(skip)]
    pub from: u32,
    #[garde(skip)]
    pub edge: BlockId,
}
