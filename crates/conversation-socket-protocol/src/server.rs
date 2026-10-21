//! What the backend sends (`crates/agent-server/src/server/tree.rs`). A page
//! accepts fields it does not know, so these types are tolerant.

use std::collections::BTreeMap;

use demi_shared_types::{
    Block, BlockId, CommandId, MAX_SAFE_INTEGER, NodeId, Nullable, OperationId, PendingSteer,
    ProviderErrorDiagnostics, ProviderFailureFacts, QueuedMessage, RequestId, TurnId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::{AgentEntry, BlockVersions, ClientFrameKind, TranscriptPatch, Version};

/// What the providers read out of the error blocks a frame carries, by block
/// id (`crates/backend-user-shard/src/conversation/failure_facts.rs`).
/// Attached when the frame is sent, never stored.
pub type Failures = BTreeMap<BlockId, ProviderFailureFacts>;

/// A frame the backend sends on a conversation's socket. Every frame for one
/// attachment goes through one outbox in causal order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ServerFrame {
    /// The connection is attached; the snapshot frames follow.
    Opened,
    /// Answers `edit_and_send` at durable acceptance.
    EditResult {
        #[garde(skip)]
        operation_id: OperationId,
        #[garde(dive)]
        outcome: EditOutcome,
    },
    /// A frame the backend refused, and why.
    Rejected {
        #[garde(skip)]
        command: ClientFrameKind,
        #[serde(flatten)]
        #[garde(skip)]
        refusal: Refusal,
        /// The request ID of the socket's upgrade, which the socket gives
        /// every `rejected` frame it sends, as it does an `error` frame;
        /// the agent's frames leave it to the socket.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        #[garde(skip)]
        request_id: Option<RequestId>,
    },
    /// The node `node`'s blocks from `start` to the end, in their light form,
    /// with its transcript's length and version: the answer to a
    /// subscription, and a subscribed node's transcript when it starts.
    TranscriptReset {
        #[garde(skip)]
        node: NodeId,
        #[garde(skip)]
        start: u32,
        #[garde(skip)]
        length: u32,
        #[serde(with = "demi_shared_types::client_blocks")]
        #[schemars(with = "Vec<Block>")]
        #[garde(dive)]
        blocks: Vec<Block>,
        #[garde(dive)]
        version: Version,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "Failures")]
        #[garde(skip)]
        failures: Option<Failures>,
        /// The versions of its blocks that have others.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[garde(dive)]
        versions: Vec<BlockVersions>,
    },
    /// The patches of one batch of the node `node`'s transcript; `revision`
    /// is one past the previous one of that node's.
    TranscriptPatch {
        #[garde(skip)]
        node: NodeId,
        #[garde(dive)]
        patches: Vec<TranscriptPatch>,
        #[garde(range(max = MAX_SAFE_INTEGER))]
        revision: u64,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "Failures")]
        #[garde(skip)]
        failures: Option<Failures>,
    },
    /// The conversation's agents at `version`: every one when `since` is
    /// absent, else those changed after the revision `since`, which the page
    /// holds of the same epoch (`crates/agent-server/src/server/tree.rs`).
    Agents {
        #[garde(dive)]
        version: Version,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(range(max = MAX_SAFE_INTEGER))]
        since: Option<u64>,
        #[garde(dive)]
        agents: Vec<AgentEntry>,
    },
    /// One agent's entry as it changed; `revision` is one past the previous
    /// change of the agents.
    Agent {
        #[garde(range(max = MAX_SAFE_INTEGER))]
        revision: u64,
        #[garde(dive)]
        agent: AgentEntry,
    },
    /// The whole queue.
    Queue {
        #[garde(dive)]
        queue: Vec<QueuedMessage>,
    },
    /// The complete list of pending steers.
    PendingSteers {
        #[garde(dive)]
        pending_steers: Vec<PendingSteer>,
    },
    /// The output of the request the root agent runs, or null when its turn
    /// has none: sent at once when a request starts or ends and when the
    /// turn ends, at most every 500 ms while it streams, and with each
    /// open's handshake.
    RequestOutput {
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<RequestOutput>")]
        #[garde(dive)]
        output: Option<RequestOutput>,
    },
    /// Answers `steer`.
    SteerResult {
        #[garde(skip)]
        steer_id: BlockId,
        #[garde(dive)]
        outcome: SteerOutcome,
    },
    /// Answers `cancel_pending_steer` and `dequeue_message`, after the list
    /// frame the removal changed.
    RemoveResult {
        /// The steer's or the queued message's id, as the request named it.
        #[schemars(length(min = 1))]
        #[garde(length(min = 1))]
        id: String,
        #[garde(skip)]
        outcome: RemoveOutcome,
    },
    /// Answers `abort`, in request order.
    AbortResult {
        #[garde(dive)]
        result: AbortResult,
    },
    /// A command's live view (`crates/agent-server/src/server/tree/live.rs`): the same for
    /// every page, whatever any page or the model read.
    ShellOutput {
        /// The node whose command it is.
        #[garde(skip)]
        node: NodeId,
        #[garde(dive)]
        status: Box<ShellStatus>,
    },
    /// Acknowledges `shell_write`.
    ShellWriteResult {
        #[garde(skip)]
        command_id: CommandId,
    },
    /// A transient provider failure is being retried after `delayMs`.
    RetryScheduled {
        #[garde(skip)]
        attempt: u32,
        #[garde(range(max = MAX_SAFE_INTEGER))]
        delay_ms: u64,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<String>")]
        #[garde(skip)]
        code: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "ProviderErrorDiagnostics")]
        #[garde(dive)]
        diagnostics: Option<ProviderErrorDiagnostics>,
    },
    /// A failed turn, a refused frame such as one with the code
    /// `invalid_frame`, or a failed save.
    Error {
        #[garde(skip)]
        message: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        #[garde(skip)]
        code: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "ProviderErrorDiagnostics")]
        #[garde(dive)]
        diagnostics: Option<ProviderErrorDiagnostics>,
        /// The request ID of the socket's upgrade, which the socket gives
        /// every `error` frame it sends
        /// (`crates/backend-http/src/request_id.rs`); the agent's frames
        /// leave it to the socket.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        #[garde(skip)]
        request_id: Option<RequestId>,
    },
    /// The connection is detached.
    Closed,
    /// Nothing: the connection sent no other frame for 30 seconds. The
    /// backend's socket sends it, not the tree, so that a page can tell a
    /// quiet connection from a dead one.
    Heartbeat,
    /// The answer to the page's `{ ping: id }`, sent at
    /// once and ahead of any other frame waiting.
    Pong {
        #[garde(skip)]
        id: u32,
    },
}

/// How an edit ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EditOutcome {
    /// The replacement is durable; its turn has this id.
    Accepted {
        #[garde(skip)]
        turn_id: TurnId,
    },
    Rejected {
        #[serde(flatten)]
        #[garde(skip)]
        refusal: Refusal,
    },
}

/// What became of a steer or a queued message the client asked to remove.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RemoveOutcome {
    /// The session held it, and holds it no more.
    Removed,
    /// The turn had taken it already, or the session never held it.
    GoneOn,
}

/// How a steer ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SteerOutcome {
    /// The steer is pending until the next continuation boundary.
    Accepted,
    Rejected {
        #[serde(flatten)]
        #[garde(skip)]
        refusal: Refusal,
    },
}

/// A frame the backend or the session refused, as the page words it: the code
/// its text is looked up by, one of the agent's [`AgentCode`]s or of the
/// backend's `ErrorCode`s, the values that text needs, and the refusal in
/// English, for the log and for a code the page has no words for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Refusal {
    pub code: String,
    /// The values the page's text for `code` needs, each a number or a
    /// string, such as `{ "thresholdPercent": 60 }`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, serde_json::Value>,
    pub reason: String,
}

impl Refusal {
    /// The refusal of `code`, which `reason` says in English.
    pub fn new(code: impl std::fmt::Display, reason: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            params: BTreeMap::new(),
            reason: reason.into(),
        }
    }

    /// The refusal with the value `name` its text is filled with.
    pub fn with_param(mut self, name: &str, value: impl Into<serde_json::Value>) -> Self {
        self.params.insert(name.to_owned(), value.into());
        self
    }
}

/// The codes of the agent's own refusals and failures, which the page words: a
/// [`Refusal`]'s, and an `error` frame's that no provider failure gave; the
/// backend's are `ErrorCode`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentCode {
    /// A second `open` on one connection.
    SessionAlreadyOpen,
    /// A frame that needs a session before the connection opened one.
    NoSession,
    /// A `compact` of a subagent that has ended.
    AgentEnded,
    /// A `send_saved` sent to the agent itself, which only the backend
    /// sends.
    SentByBackend,
    /// An action that needs the session idle while it works.
    SessionBusy,
    /// A `compact` of a context that is not full enough yet, below
    /// `thresholdPercent` of its window with `usedPercent` used.
    CompactionUnavailable,
    /// The session is closed.
    SessionClosed,
    /// A message edit is being prepared.
    EditInProgress,
    /// A steer while no turn runs.
    NothingToSteer,
    /// A steer while the running turn is being stopped.
    TurnStopping,
    /// A steer while the running turn is finishing.
    TurnFinishing,
    /// An edit whose operation id was used for another request.
    EditConflict,
    /// An edit while the session has work pending.
    EditBusy,
    /// An edit of a version of the conversation that changed since.
    EditStale,
    /// An edit of a block that is not a user message.
    EditTarget,
    /// An edit that keeps a file the edited message does not hold.
    EditFileMissing,
    /// An edit stopped before it was accepted.
    EditStopped,
    /// The frame's files could not be resolved, or the edit could not be
    /// prepared or saved.
    FrameFailed,
    /// The conversation's agent could not be opened or restored.
    OpenFailed,
    /// The session's work could not be saved.
    SaveFailed,
    /// A subagent could not be started, resumed or closed.
    SubagentFailed,
    /// A page's input did not reach its command.
    ShellInputFailed,
    /// A page's stop did not reach its command.
    ShellStopFailed,
    /// A Retry with no turn to run again.
    NothingToRetry,
    /// Compaction's summary request answered calls and no summary, twice.
    SummaryMissing,
    /// The system prompt of the conversation's model could not be made, as
    /// when its provider entry is gone.
    PromptUnavailable,
}

serde_plain::derive_display_from_serialize!(AgentCode);

/// What an `abort` stopped, and whether another `abort` would stop more.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate,
)]
#[serde(rename_all = "camelCase")]
pub struct AbortResult {
    /// Null when there was nothing to stop.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<AbortTarget>")]
    #[garde(skip)]
    pub target: Option<AbortTarget>,
    #[garde(skip)]
    pub can_abort_again: bool,
}

/// The one thing an `abort` stops, in the order it looks for one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AbortTarget {
    ActiveProviderStream,
    ActiveTool,
    ActiveCompaction,
    ActiveTurn,
    QueuedAction,
    QueuedMessage,
}

serde_plain::derive_display_from_serialize!(AbortTarget);
serde_plain::derive_fromstr_from_deserialize!(AbortTarget);

/// What a child's round runs on,
/// in the words the model menu shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct JobModel {
    /// The provider entry's id, which finds the entry and its account.
    #[garde(skip)]
    pub provider_id: String,
    /// The provider entry's name.
    #[garde(skip)]
    pub provider: String,
    /// The model's name.
    #[garde(skip)]
    pub model: String,
    /// The thinking effort in the vendor's word, such as `high`, or `off`;
    /// null for a model that offers no thinking setting.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub effort: Option<String>,
    /// Whether the round runs on the model's Fast tier.
    #[garde(skip)]
    pub fast: bool,
}

/// Where a child is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobPhase {
    Running,
    Completed,
    Aborted,
    Error,
}

serde_plain::derive_display_from_serialize!(JobPhase);
serde_plain::derive_fromstr_from_deserialize!(JobPhase);

/// Where a command is, with the pages' view of it: running, and whether a
/// stop waits for its runner's word or its start for its Host, exited with its code, stopped, or lost
/// with why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ShellStatus {
    Running {
        #[serde(flatten)]
        #[garde(dive)]
        command: CommandView,
        /// A stop waits for its runner to say that it ended (`crates/runner-shell/src/job.rs`): the pages tag it *Stopping…*.
        /// Present only then.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[garde(skip)]
        stopping: bool,
        /// The name of the paired device whose runner its start waits for
        /// (`crates/backend-host-access/src/access.rs`): the pages show
        /// *Waiting for MacBook Pro*. Present only then.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(skip)]
        waiting_for: Option<String>,
    },
    Exited {
        #[serde(flatten)]
        #[garde(dive)]
        command: CommandView,
        #[garde(skip)]
        exit_code: i32,
    },
    Aborted {
        #[serde(flatten)]
        #[garde(dive)]
        command: CommandView,
    },
    /// Its runner lost it (`crates/agent-session/src/reports.rs`).
    Lost {
        #[serde(flatten)]
        #[garde(dive)]
        command: CommandView,
        #[garde(skip)]
        reason: String,
    },
}

impl ShellStatus {
    /// The command and its view, whatever its status.
    pub fn command(&self) -> &CommandView {
        match self {
            Self::Running { command, .. }
            | Self::Exited { command, .. }
            | Self::Aborted { command }
            | Self::Lost { command, .. } => command,
        }
    }
}

/// The output of one provider request so far, estimated while it streams by
/// `OutputEstimate` (`crates/agent-transcript/src/estimate.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct RequestOutput {
    /// The request's id: another id is another request, whose count starts
    /// from zero.
    #[garde(skip)]
    pub request_id: String,
    /// The output tokens: an estimate of what streamed, then, once the
    /// response ended, the response's own count when it reported one.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub tokens: u64,
    /// How long the request had streamed since its first token when the
    /// frame left, up to its response's end once it ended; null before the
    /// first token. A page counts on from its own clock while the request
    /// streams, as it does for `waiting`.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u64>")]
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub streamed_ms: Option<u64>,
    /// The response ended: the count and the time no longer change.
    #[garde(skip)]
    pub ended: bool,
}

/// The speed of an agent's last turn that streamed for a second or more
/// (`docs/decisions/0012-live-output-tokens.md`): the output tokens of its
/// requests over the time they streamed, each from its first token to its
/// end, so the time between them, such as a tool's, does not count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TurnSpeed {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub tokens: u64,
    /// At least a second: a turn that streamed for less keeps the speed
    /// before it (`crates/agent-session/src/core.rs`).
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub streamed_ms: u64,
}

/// A command as the pages see it, whatever its status
/// (`crates/agent-server/src/server/tree/live.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct CommandView {
    #[garde(skip)]
    pub command_id: CommandId,
    /// The `shell` call that started it, in the subagent's transcript
    /// when the frame names one.
    #[garde(skip)]
    pub tool_use_id: String,
    /// That call's title, its `description`, as the command's record keeps
    /// it from its start, which its tab in the Running window shows whether
    /// or not the page holds the call.
    #[garde(skip)]
    pub title: String,
    /// That call's script, as the command's record keeps it, which its
    /// terminal opens with.
    #[garde(skip)]
    pub script: String,
    /// The last 4,096 characters of the pages' view of its output: its
    /// output in the order it reached the backend, with a note where the
    /// runner left some out.
    #[garde(skip)]
    pub tail: String,
    /// How many characters the view has held since the command started. A
    /// page adds only the characters beyond those it has shown.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub chars: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub running_ms: u64,
}
