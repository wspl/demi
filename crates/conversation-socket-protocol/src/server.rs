//! What the backend sends (`runtime.md` § Server frames, `subagents.md`
//! § Protocol). A page accepts fields it does not know, so these types are
//! tolerant.

use std::collections::BTreeMap;

use demi_shared_types::{
    Block, BlockId, CommandId, ContextUsage, MAX_SAFE_INTEGER, NodeId, Nullable, OperationId, PendingCall, PendingSteer,
    ProviderErrorDiagnostics, ProviderFailureFacts, QueuedMessage, SessionPhase,
    Timestamp, TurnId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::{ClientFrameKind, TranscriptPatch, TranscriptVersion};

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
        #[garde(skip)]
        reason: String,
    },
    /// Every block, with the transcript's version.
    TranscriptReset {
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
    Phase {
        #[garde(skip)]
        phase: SessionPhase,
    },
    /// The estimate of the root's next request with the window its
    /// thresholds use: on open, after each response and compaction pass,
    /// and when an action ends.
    ContextUsage {
        #[garde(dive)]
        usage: ContextUsage,
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
    /// (`runtime.md` § Calls being written).
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
    /// Answers `steer`.
    SteerResult {
        #[garde(skip)]
        steer_id: BlockId,
        #[garde(dive)]
        outcome: SteerOutcome,
    },
    /// Answers `abort`, in request order.
    AbortResult {
        #[garde(dive)]
        result: AbortResult,
    },
    /// A command's live view (`runtime.md` § Live output): the same for
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
    },
    /// A subagent started or closed.
    Subagent {
        #[garde(skip)]
        event: SubagentEvent,
        #[garde(dive)]
        job: SubagentJob,
    },
    SubagentTranscriptReset {
        #[garde(skip)]
        subagent_id: NodeId,
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
    /// quiet connection from a dead one (`runtime.md` § Order and delivery).
    Heartbeat,
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
        #[garde(skip)]
        reason: String,
    },
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
        #[garde(skip)]
        reason: String,
    },
}

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
    PendingYieldWakeup,
}

serde_plain::derive_display_from_serialize!(AbortTarget);
serde_plain::derive_fromstr_from_deserialize!(AbortTarget);

/// Whether a subagent started or closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubagentEvent {
    Started,
    Closed,
}

serde_plain::derive_display_from_serialize!(SubagentEvent);
serde_plain::derive_fromstr_from_deserialize!(SubagentEvent);

/// One child agent as the parent's connection sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct SubagentJob {
    #[garde(skip)]
    pub subagent_id: NodeId,
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

/// Where a command is, with the pages' view of it: running, exited with its
/// code, or stopped.
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
}

impl ShellStatus {
    /// The command and its view, whatever its status.
    pub fn command(&self) -> &CommandView {
        match self {
            Self::Running { command }
            | Self::Exited { command, .. }
            | Self::Aborted { command } => command,
        }
    }
}

/// A command as the pages see it, whatever its status (`runtime.md`
/// § Live output).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct CommandView {
    #[garde(skip)]
    pub command_id: CommandId,
    /// The `shell_exec` call that started it, in the subagent's transcript
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
