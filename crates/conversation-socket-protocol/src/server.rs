//! What the backend sends (`rendering.md` § Server frames, `subagents.md`
//! § Protocol). A page accepts fields it does not know, so these types are
//! tolerant.

use std::collections::BTreeMap;

use demi_shared_types::{
    Block, BlockId, CommandId, ContextUsage, MAX_SAFE_INTEGER, NodeId, Nullable, OperationId, PendingCall, PendingSteer,
    ProviderErrorDiagnostics, ProviderFailureFacts, QueuedMessage, RequestId, SessionPhase,
    Timestamp, TurnId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::{BlockVersions, ClientFrameKind, TranscriptPatch, TranscriptVersion};

/// What the providers read out of the error blocks a frame carries, by block
/// id (`backend.md` § Failure facts). Attached when the frame is sent, never
/// stored.
pub type Failures = BTreeMap<BlockId, ProviderFailureFacts>;

/// A frame the backend sends on a conversation's socket. Every frame for one
/// attachment goes through one outbox in causal order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
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
    /// The blocks from `start` to the end, whole, with the transcript's
    /// length and version (`rendering.md` § Where a reset starts).
    TranscriptReset {
        #[garde(skip)]
        start: u32,
        #[garde(skip)]
        length: u32,
        #[serde(with = "demi_shared_types::client_blocks")]
        #[schemars(with = "Vec<Block>")]
        #[garde(dive)]
        blocks: Vec<Block>,
        #[garde(dive)]
        version: TranscriptVersion,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "Failures")]
        #[garde(skip)]
        failures: Option<Failures>,
        /// The versions of its blocks that have others
        /// (`message-editing.md` § Versions).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[garde(dive)]
        versions: Vec<BlockVersions>,
    },
    /// The patches of one batch; `revision` is one past the previous frame's.
    TranscriptPatch {
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
    /// A session's phase: the root's, or a live subagent's, which a page
    /// opening the conversation receives for each live child too.
    Phase {
        /// The subagent whose phase it is; absent for the root.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "NodeId")]
        #[garde(skip)]
        subagent_id: Option<NodeId>,
        #[garde(skip)]
        phase: SessionPhase,
    },
    /// The estimate of a node's next request with the window its
    /// thresholds use: every node's session, a subagent's as the root's,
    /// reports it on open, after each response and compaction pass, and
    /// when an action ends.
    ContextUsage {
        /// The agent whose context it is: the root, or a live subagent.
        #[garde(skip)]
        node: NodeId,
        /// None when an open cannot tell it without reading a blob
        /// (`compaction.md` § Context estimate): the page then shows no
        /// figure rather than one from before.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<ContextUsage>")]
        #[garde(dive)]
        usage: Option<ContextUsage>,
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
    /// The complete list of the calls the model is writing
    /// (`rendering.md` § Calls being written).
    PendingCalls {
        /// The subagent whose calls they are; absent for the root's.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "NodeId")]
        #[garde(skip)]
        subagent_id: Option<NodeId>,
        #[garde(dive)]
        pending_calls: Vec<PendingCall>,
    },
    /// How long a node had waited for the model's answer when the frame
    /// left, or null when it waits for none (`rendering.md` § Waiting for the
    /// model).
    Waiting {
        /// The subagent that waits; absent for the root.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "NodeId")]
        #[garde(skip)]
        subagent_id: Option<NodeId>,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<u64>")]
        #[garde(range(max = MAX_SAFE_INTEGER))]
        waited_ms: Option<u64>,
    },
    /// Answers `steer`.
    SteerResult {
        #[garde(skip)]
        steer_id: BlockId,
        #[garde(dive)]
        outcome: SteerOutcome,
    },
    /// Answers `cancel_pending_steer` and `dequeue_message`, after the list
    /// frame the removal changed (`runtime.md` § Pending steers).
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
    /// A command's live view (`shell-tool.md` § Live output): the same for
    /// every page, whatever any page or the model read.
    ShellOutput {
        /// The subagent whose command it is; absent for the root's.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "NodeId")]
        #[garde(skip)]
        subagent_id: Option<NodeId>,
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
        /// every `error` frame it sends (`backend.md` § Request IDs and the
        /// server log); the agent's frames leave it to the socket.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        #[garde(skip)]
        request_id: Option<RequestId>,
    },
    /// A subagent started or closed.
    Subagent {
        #[garde(skip)]
        event: SubagentEvent,
        #[garde(dive)]
        job: SubagentJob,
    },
    /// Every child of the tree, finished ones included, which the page
    /// takes whole: sent on each open, after the root's state
    /// (`subagents.md` § Protocol).
    Subagents {
        #[garde(dive)]
        subagents: Vec<SubagentSummary>,
    },
    SubagentTranscriptReset {
        #[garde(skip)]
        subagent_id: NodeId,
        /// The index of the first block, the start of the child's latest
        /// page, and how many blocks its transcript holds.
        #[garde(skip)]
        start: u32,
        #[garde(skip)]
        length: u32,
        #[serde(with = "demi_shared_types::client_blocks")]
        #[schemars(with = "Vec<Block>")]
        #[garde(dive)]
        blocks: Vec<Block>,
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
    SubagentTranscriptPatch {
        #[garde(skip)]
        subagent_id: NodeId,
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
    /// The connection is detached.
    Closed,
    /// Nothing: the connection sent no other frame for 30 seconds. The
    /// backend's socket sends it, not the tree, so that a page can tell a
    /// quiet connection from a dead one (`rendering.md` § Order and delivery).
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

/// A frame the backend or the session refused, as the page words it
/// (`localization.md` § Text from the backend): the code its text is looked
/// up by, one of the agent's [`AgentCode`]s or of the backend's
/// `ErrorCode`s, the values that text needs, and the refusal in English, for
/// the log and for a code the page has no words for.
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

/// The codes of the agent's own refusals and failures, which the page words
/// (`localization.md` § Text from the backend): a [`Refusal`]'s, and an
/// `error` frame's that no provider failure gave; the backend's are
/// `ErrorCode`s.
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

/// Whether a subagent started, changed while it runs, as when `demi agent
/// cwd` moved it, or closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubagentEvent {
    Started,
    Changed,
    Closed,
}

serde_plain::derive_display_from_serialize!(SubagentEvent);
serde_plain::derive_fromstr_from_deserialize!(SubagentEvent);

/// One child as an open's snapshot lists it: what a job says of it but its
/// result, which the page reads with its transcript when it shows it, and
/// the revision of its transcript, which tells the page whether the
/// transcript it holds is still current (`subagents.md` § Protocol).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSummary {
    #[serde(flatten)]
    #[garde(dive)]
    pub child: SubagentChild,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
}

/// One child agent as the parent's connection sees it: the child, and on a
/// `completed` close its result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct SubagentJob {
    #[serde(flatten)]
    #[garde(dive)]
    pub child: SubagentChild,
    /// Only on a `completed` close: the child's last assistant text, at most
    /// 32 KiB.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub result: Option<String>,
}

/// What a job and a summary say of one child.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct SubagentChild {
    #[garde(skip)]
    pub subagent_id: NodeId,
    /// The number the model knows it by, which `demi agent spawn` printed
    /// and a message links it with as `agent:<number>`.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub number: u64,
    /// The node that spawned it: the web app keys nested views by it.
    #[garde(skip)]
    pub parent_session_id: NodeId,
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
    #[garde(skip)]
    pub phase: JobPhase,
    /// When the round started.
    #[garde(skip)]
    pub started_at: Timestamp,
    /// When it closed; null while it runs.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    #[garde(skip)]
    pub ended_at: Option<Timestamp>,
    /// Its working directory on the primary Host now, where its `shell`
    /// calls start (`subagents.md` § Working directory); a `changed` frame
    /// tells a change while it runs.
    #[garde(length(min = 1))]
    pub cwd: String,
    /// Whether the call that started it is on its parent's current path
    /// (`message-editing.md` § Versions): the Agents window and `agent:`
    /// links show only the children that are, and a `changed` frame tells
    /// when a switch of versions changes it.
    #[garde(skip)]
    pub on_path: bool,
}

/// What a child's round runs on (`subagents.md` § What an agent runs on),
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
        /// A stop waits for its runner to say that it ended (`runner.md`
        /// § Cancellation and completion): the pages tag it *Stopping…*.
        /// Present only then.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[garde(skip)]
        stopping: bool,
        /// The name of the paired device whose runner its start waits for
        /// (`sessions-and-targets.md` § Host operations): the pages show
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
    /// Its runner lost it (`shell-tool.md` § Lost commands).
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

/// A command as the pages see it, whatever its status (`shell-tool.md`
/// § Live output).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct CommandView {
    #[garde(skip)]
    pub command_id: CommandId,
    /// The `shell` call that started it, in the subagent's transcript
    /// when the frame names one.
    #[garde(skip)]
    pub tool_use_id: String,
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
