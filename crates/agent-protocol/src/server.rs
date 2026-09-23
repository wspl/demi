//! What the backend sends (`runtime.md` § Server frames, `subagents.md`
//! § Protocol). A page accepts fields it does not know, so these types are
//! tolerant.

use std::collections::BTreeMap;

use demi_core::{
    BinaryStdout, Block, BlockId, CommandId, MAX_SAFE_INTEGER, NodeId, Nullable, OperationId,
    OutputView, PendingSteer, ProviderErrorDiagnostics, ProviderFailureFacts, QueuedMessage,
    SessionPhase, ShellId, StreamView, Timestamp, TurnId,
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
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
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
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
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
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Failures")]
        #[garde(skip)]
        failures: Option<Failures>,
    },
    Phase {
        #[garde(skip)]
        phase: SessionPhase,
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
    /// Answers `steer` and `steer_queued_message`.
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
    /// A running command's status and output, live.
    ShellOutput {
        #[garde(dive)]
        status: ShellStatus,
    },
    /// Acknowledges `shell_write`, after the `shell_output` it caused.
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
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "ProviderErrorDiagnostics")]
        #[garde(dive)]
        diagnostics: Option<ProviderErrorDiagnostics>,
    },
    /// A failed turn, a refused frame such as one with the code
    /// `invalid_frame`, or a failed save.
    Error {
        #[garde(skip)]
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(skip)]
        code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
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
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
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
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Failures")]
        #[garde(skip)]
        failures: Option<Failures>,
    },
    /// The connection is detached.
    Closed,
}

/// How an edit ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "status", rename_all = "snake_case", rename_all_fields = "camelCase")]
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
#[serde(tag = "status", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum SteerOutcome {
    /// The steer is pending until the next continuation boundary.
    Accepted,
    Rejected {
        #[garde(skip)]
        reason: String,
    },
}

/// What an `abort` stopped, and whether another `abort` would stop more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
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
    /// The node that spawned it: the browser keys nested views by it.
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
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
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

/// A command's status, live: running, exited or stopped, with its output
/// since the last look. Binary stdout is described by its size, never sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "status", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ShellStatus {
    Running {
        #[serde(flatten)]
        #[garde(dive)]
        command: CommandView,
        /// The running command's guidance for the model, when it declares
        /// one.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(skip)]
        running_hint: Option<String>,
    },
    Exited {
        #[serde(flatten)]
        #[garde(dive)]
        command: CommandView,
        #[garde(skip)]
        exit_code: i32,
        /// Present when the final stdout was not text.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "BinaryStdout")]
        #[garde(dive)]
        binary_stdout: Option<BinaryStdout>,
    },
    Aborted {
        #[serde(flatten)]
        #[garde(dive)]
        command: CommandView,
    },
}

impl ShellStatus {
    /// The command and its output, whatever its status.
    pub fn command(&self) -> &CommandView {
        match self {
            Self::Running { command, .. }
            | Self::Exited { command, .. }
            | Self::Aborted { command } => command,
        }
    }
}

/// The part of a command's status every status has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct CommandView {
    #[garde(skip)]
    pub shell_id: ShellId,
    #[garde(skip)]
    pub command_id: CommandId,
    /// The directory on the Host that holds the command's output files;
    /// absent when none is kept.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub output_dir: Option<String>,
    #[garde(dive)]
    pub stdout: StreamView,
    #[garde(dive)]
    pub stderr: StreamView,
    #[garde(dive)]
    pub output: OutputView,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub running_ms: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub idle_ms: u64,
}
