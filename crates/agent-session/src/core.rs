//! A session's state and every decision about it.
//!
//! The state is one status, the actions waiting to run, the input waiting for a
//! boundary, the commands that report to it, the transcript, the model
//! selection with its provider runtime, and what the next save writes.
//!
//! Every method here is synchronous; the worker and the turn loop await between
//! calls, never inside one.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem,
    num::NonZeroU32,
    rc::Rc,
    sync::Arc,
};

use demi_agent_store::{
    CheckpointState, CheckpointUpdate, CommandWatch, EditReceipt, PendingAgentInput, Watch,
    media::{self, HeldMedia, ModelView},
};
use demi_agent_transcript::{
    DirtyRows, INTERRUPTED_CODE, INTERRUPTED_TURN_MESSAGE, IdSource, REPLAY_CHARS, RequestView,
    TranscriptLog, kept, labelled_output,
    estimate::{OutputEstimate, anchor_tokens, blocks_tokens, context_anchor},
    replay,
};
use demi_conversation_socket_protocol::{AbortTarget, RequestOutput, TurnSpeed};
use demi_provider_common::{
    Ahead, EntriesOf, InferenceRequest, PromptCache, ProviderEvent, ProviderFailure, RequestPurpose,
    ProviderRuntime, RequestBlock, RequestLimits, ToolDefinition,
};
use demi_shared_types::{
    ScheduleMark, MessageComment,
    AgentAccount, AgentMessage, BlobRef, Block, BlockId, Clock, CommandId, CommandReport, FailureSource, ReportEvent,
    ModelSelection, NodeId, PendingCall, PendingSteer, ProviderErrorDiagnostics, QueuedMessage, SessionPhase,
    ShellToolView, ShellViewStatus, Timestamp, ToolResultContentBlock, ToolView, TurnId, UserContentBlock,
    WakeupPlacement,
};
use tokio_util::sync::CancellationToken;

use super::{
    ActionEnd, ActionHandle, ActionReply, AdmissionError, AgentMessageError, ErrorReport,
    Execution, IdleCompactionDue, ModelSwitch, SessionEvent, Settle, Status, SteerError,
    cancel::{CancelReason, TurnCancel},
    compaction::{IDLE_COMPACTION_CONTEXT, IDLE_COMPACTION_SHARE, IDLE_COMPACTION_WINDOW},
    editing::{EditCheck, EditError, EditInFlight, EditSubmission},
    input::{Input, InputQueue, Opens, Take},
    persist::PersistMarks,
    reports::Watched,
    runtime::{CallCommand, CommandLook, SessionRuntime},
};

pub(crate) struct SessionCore {
    /// The calls of the running request that the model is writing, live
    /// only.
    pub(super) writing: Vec<WritingCall>,
    /// The blocks the running request's entries name (`EntriesOf`), live
    /// only.
    request_blocks: RequestBlocks,
    pub(super) id: NodeId,
    /// What the session's lines log in, with its conversation, node and user
    /// (`SessionRuntime::log_span`).
    pub(super) span: tracing::Span,
    /// The selection current now; every block records the one current when
    /// it was written.
    pub(super) model: ModelSelection,
    /// The runtime that serves requests, or what it takes while a run holds it.
    pub(super) provider: ProviderSlot,
    /// A recorded model switch that has not landed yet.
    pub(super) switch: Option<ModelSwitch>,
    /// A switch that arrived while an edit was being prepared: it waits for
    /// the edit, and is recorded once the edit is accepted or rejected.
    pub(super) waiting_switch: Option<ModelSwitch>,
    /// Runtimes a later switch replaced before they served a request, closed
    /// when the next switch lands or at dispose.
    pub(super) retired: Vec<Box<dyn ProviderRuntime>>,
    pub(super) transcript: TranscriptLog,
    /// What the session holds for the media its replayed blocks and its
    /// waiting input reference.
    pub(super) media: HeldMedia,
    /// The tools its requests declare, which replay reads.
    pub(super) tools: Arc<[ToolDefinition]>,
    /// The actions waiting, in the order they run; the queue is its sends.
    pub(super) pending: VecDeque<PendingAction>,
    pub(super) inputs: InputQueue,
    /// The commands the session's calls left running, which report to it.
    pub(super) watched: Watched,
    /// The command each running call started, by the call's id, which the
    /// call's result names when something other than its tool ends it.
    pub(super) calls: HashMap<String, CallCommand>,
    pub(super) edits: Vec<EditReceipt>,
    /// The account the agent took under its entry's rotation, which every
    /// request names; none before it took one and after a compaction
    /// (`crates/backend-providers/src/llm/rotation.rs`).
    pub(super) account: Option<AgentAccount>,
    /// The edit being prepared or run, from its admission until its action
    /// ends.
    pub(super) editing: Option<EditInFlight>,
    pub(super) activity: Activity,
    /// Waiting input opens no continuation and stays waiting until the node's
    /// next action.
    ///
    /// The session is a closing child, or a restored one its supervisor has not
    /// looked at. Not saved: a restore holds a child again, and a root's failed
    /// turn holds by itself ([`failed_turn_holds`](Self::failed_turn_holds)).
    pub(super) held: bool,
    /// The session sends one request per turn and runs no tool, as a
    /// session copy does.
    pub(super) one_request: bool,
    /// The turn a session copy's request belongs to ([`CoreParts`]).
    request_turn: Option<TurnId>,
    /// Dispose started: every admission is refused.
    pub(super) disposing: bool,
    /// What the next request's runtime is to ready ahead, once the session
    /// is idle (`crates/provider-claude-code/src/prepared.rs`).
    ahead: Option<Ahead>,
    /// Whether the session readies ahead: a page opened its conversation,
    /// or the user chose its model; a subagent's session never does.
    prepares_ahead: bool,
    pub(super) persist: PersistMarks,
    /// Events of the current change, delivered once it is complete.
    pub(super) outbox: Vec<SessionEvent>,
    /// When the node began to wait for the model's answer, while it waits: from
    /// when its turn needs the next answer until that answer shows.
    ///
    /// How long it has waited is the session's fact, not the page's: a clock a
    /// page started would say 0s at each reload, as if the request began again.
    ///
    /// The first output of the model ends the wait, reasoning whose text the
    /// vendor withholds included, so a model that thinks never reads as one
    /// that has not answered. A compaction pauses it.
    waiting_since: Option<Timestamp>,
    /// The output of the turn's latest request, from its start until the
    /// turn ends; live only, like the wait.
    output: Option<StreamedOutput>,
    /// What the turn's requests before the latest streamed; live only.
    turn_output: TurnOutput,
    /// The speed of the last turn that streamed for [`TURN_SPEED_AFTER_MS`]
    /// or more, which the checkpoint keeps.
    turn_speed: Option<TurnSpeed>,
    ids: Rc<dyn IdSource>,
    clock: Arc<dyn Clock>,
    /// The node's runtime, which knows which commands' ends a look showed it.
    runtime: Rc<dyn SessionRuntime>,
    requests: Requests,
    published: Published,
    /// What made the execution change, which the next publication says: the
    /// last cause set since it.
    state_cause: &'static str,
}

/// Logs a call that completes as interrupted, in `span`, and what ended it. A
/// crash is a restart that found calls with no result; any other end is the
/// session's own.
pub(crate) fn log_interrupted(
    span: &tracing::Span,
    node: &NodeId,
    turn: Option<&TurnId>,
    call: &demi_agent_transcript::PendingCall,
    end: &CallEnd<'_>,
    command: Option<&CommandId>,
) {
    let end_name = match end {
        CallEnd::Stopped => "stopped",
        CallEnd::Held(_) => "held",
        CallEnd::Shutdown => "shutdown",
        CallEnd::Crashed => "crashed",
        CallEnd::Failed(_) => "failed",
    };
    let turn = turn.map(tracing::field::display);
    let command = command.map(tracing::field::display);
    if matches!(end, CallEnd::Crashed | CallEnd::Failed(_)) {
        tracing::warn!(
            parent: span,
            event = "tool_call.interrupted",
            node = %node,
            turn,
            call = call.tool_use_id,
            tool = call.tool_name,
            end = end_name,
            command,
            "a tool call ended without its result"
        );
    } else {
        tracing::info!(
            parent: span,
            event = "tool_call.interrupted",
            node = %node,
            turn,
            call = call.tool_use_id,
            tool = call.tool_name,
            end = end_name,
            command,
            "a tool call ended without its result"
        );
    }
}

/// What made the node wait for the model's answer.
#[derive(Debug, Clone, Copy)]
pub(super) enum WaitStart {
    /// The action started.
    TurnStart,
    /// The calls of a step returned, and the next request follows them.
    ToolRound,
    /// Input arrived while the turn ran, and the next request carries it.
    InputArrived,
    /// A failed request is asked again.
    Retry,
    /// The model finished a text and goes on within its run.
    TextEnded,
    /// A compaction pass ended inside the turn.
    CompactionEnd,
}

impl WaitStart {
    fn as_str(self) -> &'static str {
        match self {
            Self::TurnStart => "turn_start",
            Self::ToolRound => "tool_round",
            Self::InputArrived => "input_arrived",
            Self::Retry => "retry",
            Self::TextEnded => "text_ended",
            Self::CompactionEnd => "compaction_end",
        }
    }
}

/// What ended the node's wait for the model's answer.
#[derive(Debug, Clone, Copy)]
pub(super) enum WaitEnd {
    /// The model began to answer, reasoning or text.
    FirstOutput,
    /// The model named a call it is still writing.
    CallTitle,
    /// A compaction pass began, which the page shows in its place.
    CompactionPass,
    /// The action ended.
    ActionEnd,
    /// The user sent a message now, which restarts the wait.
    SteerNow,
    /// Dispose closed the session.
    Shutdown,
}

impl WaitEnd {
    fn as_str(self) -> &'static str {
        match self {
            Self::FirstOutput => "first_output",
            Self::CallTitle => "call_title",
            Self::CompactionPass => "compaction_pass",
            Self::ActionEnd => "action_end",
            Self::SteerNow => "steer_now",
            Self::Shutdown => "shutdown",
        }
    }
}

/// What the session is doing.
pub(super) enum Activity {
    Idle,
    Running(ActionRun),
    /// The action ended and its closing save is in progress. The save
    /// records the session idle, while clients see it running until that
    /// save has committed; `idle_compaction` is the action's.
    Finishing {
        idle_compaction: bool,
    },
    /// Dispose is complete but for the final save; `interrupted` when it
    /// stopped a running turn, whose final checkpoint says `running`.
    Closed {
        interrupted: bool,
    },
}

pub(super) struct ActionRun {
    /// The turn the action's blocks belong to; a retry takes over the turn it
    /// reruns.
    pub(super) turn: TurnId,
    pub(super) cancel: Rc<TurnCancel>,
    pub(super) stage: TurnStage,
    /// The action is a compaction the session runs by itself while idle,
    /// which clients see as `compacting_idle` from its start to its end.
    idle_compaction: bool,
    /// The transcript's revision when the action started: an action that
    /// wrote nothing since has not begun.
    started_at: u64,
    /// The message a send started with, until its `user` block is written:
    /// clients show it as the turn's starting message, so the state holds
    /// every message the session accepted, and its held bytes stay.
    unwritten: Option<QueuedMessage>,
    /// What the user sent now, which the turn takes at its next boundary.
    send_now: Option<SendNow>,
    /// Taken by the worker when it starts the action.
    start: Option<StartedAction>,
    /// When the action started, for its end's line.
    began: Timestamp,
    /// How long the message that started the action waited in the queue.
    queued_ms: u64,
    /// When the user stopped the action, until it records the stop.
    stop_at: Option<Timestamp>,
    /// What the log calls the action.
    kind_name: &'static str,
    /// What the action's turn did, which its end's line counts.
    tally: TurnTally,
}

/// What a running action's turn did, which its end line counts.
#[derive(Default)]
struct TurnTally {
    requests: u32,
    tool_rounds: u32,
    compactions: u32,
    /// A message the user sent now ended the turn at a boundary.
    yielded: bool,
}

/// What the user sent now. A queued message outranks a steer: both are
/// written at the boundary, and the turn ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum SendNow {
    /// A pending steer: the turn goes on with the next request.
    Steer,
    /// A queued message, now first in the queue: the turn ends at the
    /// boundary, and the message runs next.
    Message,
}

/// Where a running action is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnStage {
    /// Before a request: the first one, or the next one after tools ran.
    Preparing,
    Streaming,
    Tools,
    Compacting,
}

/// An action the worker runs.
pub(super) struct StartedAction {
    pub(super) kind: ActionKind,
    pub(super) turn: TurnId,
    pub(super) cancel: Rc<TurnCancel>,
    pub(super) reply: Option<ActionReply>,
}

pub(super) struct PendingAction {
    pub(super) kind: ActionKind,
    /// A send's message id; a fresh id for every other action.
    turn: TurnId,
    /// Answered when the action ends, or when it leaves the queue.
    reply: Option<ActionReply>,
    /// When it was admitted, for the line that says how long it waited.
    admitted_at: Timestamp,
}

impl PendingAction {
    /// Answers the action's caller, if it still has one.
    fn end(&mut self, end: ActionEnd) {
        if let Some(reply) = self.reply.take() {
            // A caller that dropped its handle wants no answer.
            let _ = reply.send(Ok(end));
        }
    }

    fn is_send(&self) -> bool {
        matches!(self.kind, ActionKind::Send { .. })
    }
}

/// What ended a running call other than its tool.
pub(crate) enum CallEnd<'a> {
    /// The user's Stop.
    Stopped,
    /// A hold of the conversation, which names it.
    Held(&'a str),
    /// The backend's shutdown.
    Shutdown,
    /// The backend stopped before the call's result was recorded, which a
    /// restore finds.
    Crashed,
    /// The action failed before its calls ran, for this reason.
    Failed(&'a str),
}

/// The result of a call `end` ended, which watched `command`, when it started
/// one.
///
/// It says what ended the call and what became of its command, since a model
/// told only `Tool call aborted: shell_exec` spent calls on `ps` and
/// `journalctl` to find out. A shutdown or a crash leaves the command running,
/// and it reports as any command whose call returned does.
pub(crate) fn interrupted_text(end: &CallEnd<'_>, command: Option<&CommandId>) -> String {
    let reason = match end {
        CallEnd::Stopped => "Tool call aborted: the user stopped the turn".to_owned(),
        CallEnd::Held(hold) => format!("Tool call aborted: {hold}"),
        CallEnd::Shutdown => "Tool call interrupted: the backend shut down".to_owned(),
        CallEnd::Crashed => {
            "Tool call interrupted: the backend stopped before its result was recorded".to_owned()
        }
        CallEnd::Failed(failure) => {
            return format!("Tool call aborted: {}.", failure.trim_end_matches('.'));
        }
    };
    let Some(command) = command else {
        return format!("{reason}.");
    };
    match end {
        CallEnd::Stopped | CallEnd::Held(_) => format!("{reason}; command {command} was stopped."),
        CallEnd::Shutdown => format!(
            "{reason} while this call watched command {command}, which keeps running and wakes you when it ends."
        ),
        CallEnd::Crashed => format!("{reason}; command {command} keeps running if its Host kept it."),
        CallEnd::Failed(_) => unreachable!("answered above"),
    }
}

pub(super) enum ActionKind {
    /// A message: appends a `user` block and runs a turn.
    Send {
        content: Vec<UserContentBlock>,
        /// The comments sent with it, which its `user` block keeps.
        comments: Vec<MessageComment>,
        /// The schedule's run that sent it, which its `user` block keeps.
        schedule: Option<ScheduleMark>,
    },
    /// Input that arrived while nothing ran, command reports or agent
    /// messages: it opens a turn of its own, never shown in the queue.
    Continue,
    Retry,
    /// Resumes the unfinished turn; `reason` says what made Demi resume it,
    /// none when the user did.
    Resume {
        reason: Option<String>,
    },
    /// A compaction pass: the user's, or one while the session is `idle`,
    /// before its prompt cache expires.
    Compact {
        idle: bool,
    },
    /// An admitted edit, whose submission waits in the session's edit state.
    Edit,
    /// A switch to the `to`th version at the block `target`'s place.
    Switch {
        target: BlockId,
        to: u32,
    },
}

impl ActionKind {
    /// The name the log gives the action.
    pub(super) fn name(&self) -> &'static str {
        match self {
            Self::Send { .. } => "send",
            Self::Continue => "continue",
            Self::Retry => "retry",
            Self::Resume { .. } => "resume",
            Self::Compact { .. } => "compact",
            Self::Edit => "edit",
            Self::Switch { .. } => "switch",
        }
    }
}

/// What one `abort` found to stop.
pub(super) enum AbortStep {
    /// The running action, which records the stop; `abort` waits for that.
    Running {
        target: AbortTarget,
        cancel: Rc<TurnCancel>,
    },
    /// A waiting action, which had not run yet.
    Removed(AbortTarget),
    Nothing,
}

/// What the wrapper does once a change is complete.
#[derive(Default)]
struct Requests {
    wake_worker: bool,
    save: bool,
}

/// A call the model is writing: what the page is told, and the input written so
/// far, from which its description is read.
///
/// It is live only: nothing of it is stored, replayed to the model or saved in
/// the checkpoint, since until it is whole it is not a call. It leaves the list
/// in the same step that adds its `tool_call` block, so a page never shows the
/// call twice or not at all.
pub(super) struct WritingCall {
    call: PendingCall,
    input: String,
}

/// The derived values listeners and the save were last told about.
struct Published {
    queue: Vec<TurnId>,
    phase: SessionPhase,
    pending_steers: Vec<PendingSteer>,
    starting: Option<QueuedMessage>,
    pending_calls: Vec<PendingCall>,
    agent_inputs: Vec<BlockId>,
    reports: Vec<CommandReport>,
    next_turn_reports: Vec<CommandReport>,
    watches: Vec<CommandWatch>,
    edits: usize,
    waiting_since: Option<Timestamp>,
    output: Option<StreamedOutput>,
    execution: Execution,
    /// When it changed to it; none until it first changes.
    execution_at: Option<Timestamp>,
}

/// The output of one request: estimated as it streams, then the response's
/// own count once it ends.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamedOutput {
    request_id: String,
    estimate: OutputEstimate,
    /// The response's own output tokens, once it reported them.
    reported: Option<u64>,
    /// When the model's first output of the request came.
    first_at: Option<Timestamp>,
    /// When the response ended.
    ended_at: Option<Timestamp>,
    /// When the request was sent, from which its prompt cache's lifetime
    /// counts.
    sent_at: Timestamp,
    /// How long the vendor keeps the request's prompt cache, when its
    /// provider said.
    cache_lifetime: Option<std::time::Duration>,
    /// The tokens the request carried, input and cache alike, once its
    /// response reported them.
    carried: Option<u64>,
}

impl StreamedOutput {
    /// The output tokens: the response's own count once it reported one,
    /// else the estimate.
    fn tokens(&self) -> u64 {
        self.reported.unwrap_or_else(|| self.estimate.tokens())
    }

    /// How long it streamed from its first token until its end, or until
    /// `now` while it streams; none before its first token.
    fn streamed_ms(&self, now: Timestamp) -> Option<u64> {
        let until = self.ended_at.unwrap_or(now);
        self.first_at
            .map(|first| u64::try_from(until.as_millisecond() - first.as_millisecond()).unwrap_or(0))
    }
}

/// How long a turn streams, at least, for its speed to be kept: over less,
/// a few tokens read as thousands a second, so the speed before it stays.
const TURN_SPEED_AFTER_MS: u64 = 1_000;

/// The output tokens and streaming time of a turn's requests so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TurnOutput {
    tokens: u64,
    streamed_ms: u64,
}

/// What a complete change leaves to do after the core is released.
pub(super) struct Effects {
    pub(super) events: Vec<SessionEvent>,
    pub(super) wake_worker: bool,
    pub(super) save: bool,
    pub(super) status: Status,
}

/// Where a session's provider runtime is.
///
/// A run takes it out of its slot for the length of a request, and the session
/// goes on weighing its next request meanwhile, as when a page opens the
/// conversation while a turn streams and is told the usage. The slot therefore
/// keeps what the runtime takes in one request of the current model, which does
/// not change while the runtime is out.
pub(super) enum ProviderSlot {
    Held(Box<dyn ProviderRuntime>),
    Out(RequestLimits),
}

impl ProviderSlot {
    /// The runtime, while it is in its slot.
    pub(super) fn held(&self) -> Option<&dyn ProviderRuntime> {
        match self {
            Self::Held(runtime) => Some(runtime.as_ref()),
            Self::Out(_) => None,
        }
    }

    /// The runtime, while it is in its slot, to change.
    pub(super) fn held_mut(&mut self) -> Option<&mut (dyn ProviderRuntime + 'static)> {
        match self {
            Self::Held(runtime) => Some(runtime.as_mut()),
            Self::Out(_) => None,
        }
    }
}

/// The blocks of the running request that its run's entries name: the
/// block of each item it carries, and each block its events wrote, in
/// order.
#[derive(Default)]
struct RequestBlocks {
    items: Vec<BlockId>,
    written: Vec<BlockId>,
}

/// What a new or restored session starts with.
pub(super) struct CoreParts {
    pub(super) id: NodeId,
    pub(super) model: ModelSelection,
    pub(super) provider: Box<dyn ProviderRuntime>,
    pub(super) transcript: TranscriptLog,
    /// The bytes held for the transcript's media: none for a new or restored
    /// session, the window's for a session copy.
    pub(super) media: HeldMedia,
    pub(super) tools: Arc<[ToolDefinition]>,
    pub(super) inputs: InputQueue,
    pub(super) watched: Watched,
    pub(super) edits: Vec<EditReceipt>,
    pub(super) account: Option<AgentAccount>,
    /// The speed of its last turn that streamed long enough, as its
    /// checkpoint keeps it.
    pub(super) turn_speed: Option<TurnSpeed>,
    /// Whether the session sends one request per turn and runs no tool, as
    /// a session copy does.
    pub(super) one_request: bool,
    /// The turn a session copy's request belongs to: the one its session's
    /// pass runs in; none for a session, whose requests belong to its own.
    pub(super) request_turn: Option<TurnId>,
    pub(super) ids: Rc<dyn IdSource>,
    pub(super) clock: Arc<dyn Clock>,
    /// The node's runtime, which knows which commands' ends a look showed it.
    pub(super) runtime: Rc<dyn SessionRuntime>,
}

impl SessionCore {
    pub(super) fn new(parts: CoreParts) -> Self {
        let mut core = Self {
            writing: Vec::new(),
            request_blocks: RequestBlocks::default(),
            id: parts.id,
            span: parts.runtime.log_span(),
            model: parts.model,
            provider: ProviderSlot::Held(parts.provider),
            switch: None,
            waiting_switch: None,
            retired: Vec::new(),
            transcript: parts.transcript,
            media: parts.media,
            tools: parts.tools,
            pending: VecDeque::new(),
            inputs: parts.inputs,
            runtime: parts.runtime,
            watched: parts.watched,
            calls: HashMap::new(),
            edits: parts.edits,
            account: parts.account,
            editing: None,
            activity: Activity::Idle,
            held: false,
            one_request: parts.one_request,
            request_turn: parts.request_turn,
            disposing: false,
            ahead: None,
            prepares_ahead: false,
            persist: PersistMarks::default(),
            outbox: Vec::new(),
            waiting_since: None,
            output: None,
            turn_output: TurnOutput::default(),
            turn_speed: parts.turn_speed,
            ids: parts.ids,
            clock: parts.clock,
            requests: Requests::default(),
            published: Published {
                queue: Vec::new(),
                phase: SessionPhase::Idle,
                pending_steers: Vec::new(),
                starting: None,
                pending_calls: Vec::new(),
                agent_inputs: Vec::new(),
                reports: Vec::new(),
                next_turn_reports: Vec::new(),
                watches: Vec::new(),
                edits: 0,
                waiting_since: None,
                output: None,
                execution: Execution::Idle,
                execution_at: None,
            },
            state_cause: "",
        };
        // What the checkpoint holds counts as published: only later changes
        // make a save due.
        core.published.agent_inputs = core.agent_input_ids();
        core.published.reports = core.inputs.reports(Opens::Turn);
        core.published.next_turn_reports = core.inputs.reports(Opens::Nothing);
        core.published.watches = core.watched.watches();
        core.published.edits = core.edits.len();
        core
    }

    pub(super) fn clock(&self) -> Arc<dyn Clock> {
        self.clock.clone()
    }

    fn new_turn(&self) -> TurnId {
        TurnId::try_from(self.ids.next_id()).expect("an id source never gives an empty identity")
    }

    // Admission and the start of actions.

    fn refuse_admission(&self) -> Result<(), AdmissionError> {
        if self.disposing {
            return Err(AdmissionError::Closed);
        }
        if self.preparing_edit() {
            return Err(AdmissionError::Editing);
        }
        Ok(())
    }

    /// Whether an edit is admitted and not yet accepted: while it is, the
    /// session refuses every other change of its history.
    pub(super) fn preparing_edit(&self) -> bool {
        self.editing.as_ref().is_some_and(|edit| !edit.accepted)
    }

    // Editing.

    /// What the session knows of an edit before it runs: an accepted
    /// operation's receipt, or the acceptance of one being prepared, when the
    /// digest matches.
    ///
    /// Otherwise the edit needs a settled session with nothing waiting and a
    /// current snapshot.
    pub(super) fn check_edit(
        &self,
        operation: &demi_shared_types::OperationId,
        digest: &str,
        version: &demi_conversation_socket_protocol::Version,
    ) -> Result<EditCheck, EditError> {
        if let Some(receipt) = self
            .edits
            .iter()
            .find(|receipt| &receipt.operation_id == operation)
        {
            if receipt.digest != digest {
                return Err(EditError::Conflict);
            }
            return Ok(EditCheck::Accepted(receipt.clone()));
        }
        if let Some(edit) = self
            .editing
            .as_ref()
            .filter(|edit| &edit.operation_id == operation)
        {
            if edit.digest != digest {
                return Err(EditError::Conflict);
            }
            return Ok(EditCheck::InFlight(edit.acceptance()));
        }
        self.check_settled(version)?;
        Ok(EditCheck::Proceed)
    }

    /// Whether the session admits a rewrite of its path made against the
    /// transcript `version`: settled, with nothing waiting, and nothing
    /// changed since.
    fn check_settled(&self, version: &demi_conversation_socket_protocol::Version) -> Result<(), EditError> {
        if self.disposing {
            return Err(EditError::Closed);
        }
        let waiting = self.inputs.has_agent_input()
            || self.inputs.has_report()
            || !self.inputs.pending_steers().is_empty();
        if self.settle() != Settle::Settled || self.editing.is_some() || waiting {
            return Err(EditError::Busy);
        }
        if version != &self.transcript.version() {
            return Err(EditError::Stale);
        }
        Ok(())
    }

    /// Admits a switch to the `to`th version at the block `target`'s place,
    /// as an edit is admitted; the answer ends with the switch.
    pub(super) fn admit_switch(
        &mut self,
        target: BlockId,
        to: u32,
        version: &demi_conversation_socket_protocol::Version,
    ) -> Result<ActionHandle, EditError> {
        // A target before the window is found by the switch, in the store.
        self.check_settled(version)?;
        let turn = self.new_turn();
        Ok(self.push_action(ActionKind::Switch { target, to }, turn))
    }

    /// Adopts the path a switch made current from where it changed on, on fresh
    /// runtimes; returns the runtimes it discarded, for closing.
    ///
    /// `before` holds the path's blocks before `switched.from` from where the
    /// new window starts, with what is kept of those before them, when the
    /// switch changed the path from a place before the window.
    pub(super) fn adopt_switch(
        &mut self,
        switched: demi_agent_store::Switched,
        before: Option<(usize, demi_agent_store::Rows, demi_agent_transcript::Earlier)>,
    ) -> Vec<Box<dyn ProviderRuntime>> {
        let from = switched.from;
        let batch = match before {
            None => {
                let mut blocks = self.transcript.blocks()[..from - self.transcript.start()].to_vec();
                blocks.extend(switched.blocks);
                self.transcript.switch(from, blocks, switched.versions)
            }
            Some((start, rows, earlier)) => {
                let mut blocks = rows.blocks;
                blocks.extend(switched.blocks);
                let mut versions = rows.versions;
                versions.extend(switched.versions);
                self.transcript.switch_window(from, start, blocks, versions, earlier)
            }
        };
        self.persist.rows = Default::default();
        self.outbox.push(SessionEvent::TranscriptChanged {
            patches: batch.patches,
            version: self.transcript.version(),
        });
        let mut discarded = Vec::new();
        if let Some(fresh) = self.provider.held().map(|runtime| runtime.fresh()) {
            discarded.extend(self.place_runtime(fresh));
        }
        if let Some(switch) = &mut self.switch
            && let Some(runtime) = switch.runtime.as_mut()
        {
            discarded.push(mem::replace(runtime, runtime.fresh()));
        }
        discarded
    }

    /// Admits an edit whose check proceeds; the answer is its acceptance, or
    /// what the check found.
    pub(super) fn admit_edit(
        &mut self,
        submission: EditSubmission,
    ) -> Result<EditCheck, EditError> {
        let check = self.check_edit(
            &submission.operation_id,
            &submission.digest,
            &submission.version,
        )?;
        let EditCheck::Proceed = check else {
            return Ok(check);
        };
        let edit = EditInFlight::new(submission);
        let acceptance = edit.acceptance();
        self.editing = Some(edit);
        let turn = self.new_turn();
        // The edit's own action answers through its acceptance.
        drop(self.push_action(ActionKind::Edit, turn));
        Ok(EditCheck::InFlight(acceptance))
    }

    /// Adopts an edit its save made durable: the replacement history, the
    /// receipt, the model and the runtime fork the replacement runs on.
    /// Returns the runtimes it discarded, for closing.
    pub(super) fn adopt_edit(
        &mut self,
        blocks: Vec<Block>,
        runtime: Box<dyn ProviderRuntime>,
        model: ModelSelection,
        receipt: EditReceipt,
    ) -> Vec<Box<dyn ProviderRuntime>> {
        self.adopt_rewrite(blocks, true);
        self.edits.push(receipt.clone());
        // The replacement was prepared with the recorded switch; the one
        // that waited for the edit is recorded now, and lands after the
        // replacement's first request.
        let mut discarded: Vec<_> = self.place_runtime(runtime).into_iter().collect();
        discarded.extend(self.switch.take().and_then(|switch| switch.runtime));
        discarded.append(&mut self.retired);
        self.switch = self.waiting_switch.take();
        self.model = model;
        if let Some(edit) = &mut self.editing {
            edit.accepted = true;
            edit.resolve(Ok(receipt));
        }
        discarded
    }

    /// Rejects the edit being prepared: nothing of the history changed, and
    /// a switch that waited for the edit is recorded.
    pub(super) fn reject_edit(&mut self, error: EditError) {
        if let Some(edit) = self.editing.take() {
            edit.resolve(Err(error));
        }
        if let Some(switch) = self.waiting_switch.take() {
            replace_switch(&mut self.switch, switch, &mut self.retired);
        }
    }

    /// Admits a message: it starts at once when nothing runs and waits in the
    /// queue otherwise. A message whose id the session already knows is
    /// acknowledged without a second turn.
    pub(super) fn admit_send(
        &mut self,
        message: QueuedMessage,
    ) -> Result<ActionHandle, AdmissionError> {
        self.refuse_admission()?;
        let QueuedMessage { id, content, comments, schedule } = message;
        if self.knows_turn(&id) {
            return Ok(ActionHandle::ended(ActionEnd::Duplicate));
        }
        let path = if matches!(self.activity, Activity::Idle) && self.pending.is_empty() {
            "starts_turn"
        } else {
            "after_turn"
        };
        session_log!(
            self,
            info,
            event = "input.admitted",
            node = %self.id,
            turn = %id,
            input = %id,
            kind = "user",
            path,
            queued_ahead = self.pending.len(),
            "a message was admitted"
        );
        Ok(self.push_action(ActionKind::Send { content, comments, schedule }, id))
    }

    /// Admits a retry, a resume or a compaction pass.
    pub(super) fn admit(&mut self, kind: ActionKind) -> Result<ActionHandle, AdmissionError> {
        self.refuse_admission()?;
        let turn = self.new_turn();
        Ok(self.push_action(kind, turn))
    }

    /// Admits a compaction while the session is idle: only while nothing
    /// runs, waits or would open a turn. Returns whether it was admitted.
    pub(super) fn admit_idle_compaction(&mut self) -> bool {
        let idle = matches!(self.activity, Activity::Idle) && self.pending.is_empty() && !self.wants_continuation();
        if !idle || self.refuse_admission().is_err() {
            return false;
        }
        let turn = self.new_turn();
        drop(self.push_action(ActionKind::Compact { idle: true }, turn));
        true
    }

    fn push_action(&mut self, kind: ActionKind, turn: TurnId) -> ActionHandle {
        let (reply, handle) = ActionHandle::channel();
        self.pending.push_back(PendingAction {
            kind,
            turn,
            reply: Some(reply),
            admitted_at: self.clock.now(),
        });
        if matches!(self.activity, Activity::Idle) {
            self.start_next();
        }
        handle
    }

    /// Whether `id` is the running turn, a queued message or the turn of a
    /// `user` block.
    pub(super) fn knows_turn(&self, id: &TurnId) -> bool {
        let running = matches!(&self.activity, Activity::Running(run) if &run.turn == id);
        running
            || self
                .pending
                .iter()
                .any(|action| action.is_send() && &action.turn == id)
            || self.transcript.has_user_turn(id)
    }

    /// Starts the next turn with the first human steer that waits, which no
    /// model's turn took, unless a queued message is next; else the first
    /// waiting action, or a continuation for input that waits; else settles:
    /// idle, or closed once dispose began, with the steers kept in the queue.
    /// The steers that wait join the turn that starts at its first boundary.
    ///
    /// A root whose last turn failed writes the input that waits into that turn
    /// instead.
    pub(super) fn start_next(&mut self) {
        if self.disposing {
            self.queue_steers(0);
            self.activity = Activity::Closed { interrupted: false };
            return;
        }
        // A message that waits next opens the turn, and the steers join it.
        let message_next = self.pending.front().is_some_and(PendingAction::is_send);
        if let Some(steer) = (!message_next).then(|| self.inputs.take_first_steer()).flatten() {
            let pending_ms = self.inputs.take_admitted(steer.id.as_str()).map(|at| self.ms_since(at));
            let turn = TurnId::try_from(steer.id.into_string()).expect("a block id is not empty");
            session_log!(
                self,
                info,
                event = "input.opens_turn",
                node = %self.id,
                turn = %turn,
                input = %turn,
                kind = "steer",
                pending_ms,
                "a steer no turn took opens the next turn"
            );
            let kind = ActionKind::Send { content: steer.content, comments: steer.comments, schedule: None };
            self.start_action(kind, turn, None, pending_ms.unwrap_or(0));
            return;
        }
        let (kind, turn, reply, queued_ms) = match self.pending.pop_front() {
            Some(action) => {
                let queued_ms = self.ms_since(action.admitted_at);
                (action.kind, action.turn, action.reply, queued_ms)
            }
            None if self.wants_continuation() => {
                let cause = if self.inputs.has_report() { "report" } else { "agent_message" };
                self.log_wakeup(cause);
                (ActionKind::Continue, self.new_turn(), None, 0)
            }
            None => {
                self.activity = Activity::Idle;
                self.write_into_failed_turn();
                return;
            }
        };
        self.start_action(kind, turn, reply, queued_ms);
    }

    /// Runs `kind` as the session's action, writing `turn`.
    fn start_action(&mut self, kind: ActionKind, turn: TurnId, reply: Option<ActionReply>, queued_ms: u64) {
        let cancel = TurnCancel::new();
        self.held = false;
        let unwritten = match &kind {
            ActionKind::Send { content, comments, schedule } => Some(QueuedMessage {
                id: turn.clone(),
                content: content.clone(),
                comments: comments.clone(),
                schedule: schedule.clone(),
            }),
            _ => None,
        };
        // A turn waits for the model's answer from its start, through the
        // hooks and the Host reads that build its request; a compaction
        // alone asks for none.
        let waits = !matches!(kind, ActionKind::Compact { .. });
        let kind_name = kind.name();
        self.activity = Activity::Running(ActionRun {
            turn: turn.clone(),
            cancel: cancel.clone(),
            stage: TurnStage::Preparing,
            idle_compaction: matches!(kind, ActionKind::Compact { idle: true }),
            started_at: self.transcript.version().revision,
            unwritten,
            send_now: None,
            began: self.clock.now(),
            queued_ms,
            stop_at: None,
            kind_name,
            tally: TurnTally::default(),
            start: Some(StartedAction {
                kind,
                turn,
                cancel,
                reply,
            }),
        });
        if waits {
            self.start_waiting(WaitStart::TurnStart);
        }
        self.state_cause = "action_started";
        self.requests.wake_worker = true;
    }

    /// Whether waiting input would open a continuation of its own: command
    /// reports or agent messages, unless the session is held or its last
    /// turn failed.
    fn wants_continuation(&self) -> bool {
        !self.held
            && !self.failed_turn_holds()
            && (self.inputs.has_report() || self.inputs.has_agent_input())
    }

    /// Whether the last turn's failure holds the node's waiting input until the
    /// user acts, with Resume or a message.
    ///
    /// A root's turn that ended with an `error` block does, while a child's
    /// parent decides for it. The user's action writes after the record, which
    /// ends the hold. Otherwise each wakeup would start a turn that fails the
    /// same way, a whole history sent for nothing.
    ///
    /// A Stop holds nothing, since the user stopped the work, not the agent's
    /// ability to answer.
    fn failed_turn_holds(&self) -> bool {
        self.runtime.user_recovers_turns() && self.transcript.turn_failure().is_some()
    }

    /// Writes the agent messages and command reports that wait into the failed
    /// turn that holds them, where they show at once and the user's next action
    /// answers them.
    ///
    /// Nothing is written while the session is held for another reason, which
    /// keeps them waiting.
    fn write_into_failed_turn(&mut self) {
        if self.held || !self.failed_turn_holds() {
            return;
        }
        let Some(turn) = self.transcript.last_turn().cloned() else {
            return;
        };
        self.write_inputs_into(turn, Take::Everything, "failed_turn");
    }

    /// Keeps waiting input from opening a continuation until the node's
    /// next action.
    pub(super) fn hold(&mut self) {
        self.held = true;
    }

    /// Lets waiting input open a continuation again, and opens one when
    /// nothing runs or waits.
    pub(super) fn release(&mut self) {
        self.held = false;
        self.wake();
    }

    /// Opens a continuation when waiting input wants one and nothing runs or
    /// waits, or writes it into the failed turn that holds it.
    pub(super) fn wake(&mut self) {
        if matches!(self.activity, Activity::Idle) && self.pending.is_empty() {
            self.start_next();
        }
    }

    /// Opens a continuation for the user's decision on a permission request
    /// when nothing runs or waits, also after the user stopped the last turn or
    /// while the session is held.
    ///
    /// The decision is the user's own next action.
    fn wake_for_user(&mut self) {
        if matches!(self.activity, Activity::Idle) && self.pending.is_empty() && !self.disposing {
            let turn = self.new_turn();
            self.log_wakeup("user_decision");
            self.start_action(ActionKind::Continue, turn, None, 0);
        }
    }

    pub(super) fn take_started(&mut self) -> Option<StartedAction> {
        match &mut self.activity {
            Activity::Running(run) => run.start.take(),
            _ => None,
        }
    }

    pub(super) fn set_stage(&mut self, stage: TurnStage) {
        if let Activity::Running(run) = &mut self.activity {
            run.stage = stage;
            self.state_cause = match stage {
                TurnStage::Preparing => "step_ended",
                TurnStage::Streaming => "request_sent",
                TurnStage::Tools => "calls_requested",
                TurnStage::Compacting => "compaction_pass",
            };
        }
    }

    pub(super) fn stage(&self) -> Option<TurnStage> {
        match &self.activity {
            Activity::Running(run) => Some(run.stage),
            _ => None,
        }
    }

    /// The turn the running action writes.
    pub(super) fn turn(&self) -> TurnId {
        self.running_turn()
            .unwrap_or_else(|| unreachable!("only a running action writes a turn"))
    }

    /// Counts a provider request of the running turn, and says its number.
    pub(super) fn count_request(&mut self) -> u32 {
        match &mut self.activity {
            Activity::Running(run) => {
                run.tally.requests += 1;
                run.tally.requests
            }
            _ => 0,
        }
    }

    /// Milliseconds from `at` to now.
    fn ms_since(&self, at: Timestamp) -> u64 {
        u64::try_from(self.clock.now().as_millisecond() - at.as_millisecond()).unwrap_or(0)
    }

    /// An idle session opens a turn of its own for waiting input or a decision.
    fn log_wakeup(&self, cause: &'static str) {
        session_log!(
            self,
            info,
            event = "wakeup.opened_turn",
            node = %self.id,
            cause,
            reports = self.inputs.reports(Opens::Turn).len(),
            held = self.held,
            "waiting input opened a turn"
        );
    }

    /// How many provider requests the running turn has made.
    pub(super) fn requests_so_far(&self) -> u32 {
        match &self.activity {
            Activity::Running(run) => run.tally.requests,
            _ => 0,
        }
    }

    /// Counts a step of tool calls the running turn ran.
    pub(super) fn count_tool_round(&mut self) {
        if let Activity::Running(run) = &mut self.activity {
            run.tally.tool_rounds += 1;
        }
    }

    /// Counts a compaction pass that compacted inside the running action.
    pub(super) fn count_compaction(&mut self) {
        if let Activity::Running(run) = &mut self.activity {
            run.tally.compactions += 1;
        }
    }

    /// The running turn ends at a boundary for a message the user sent now.
    pub(super) fn yield_turn(&mut self) {
        if let Activity::Running(run) = &mut self.activity {
            run.tally.yielded = true;
        }
    }

    /// Says the running turn goes on with another provider request, and why.
    pub(super) fn log_continued(&self, why: &'static str) {
        let Activity::Running(run) = &self.activity else {
            return;
        };
        session_log!(
            self,
            info,
            event = "turn.continued",
            node = %self.id,
            turn = %run.turn,
            request_no = run.tally.requests + 1,
            why,
            "the turn asks the model again"
        );
    }

    /// The running action ends, as `outcome` says: one line with what its
    /// turn did.
    pub(super) fn log_turn_ended(&self, outcome: &'static str, code: Option<&str>) {
        let Activity::Running(run) = &self.activity else {
            return;
        };
        let ms = self.ms_since(run.began);
        session_log!(
            self,
            info,
            event = "turn.ended",
            node = %self.id,
            turn = %run.turn,
            action = run.kind_name,
            outcome = match outcome {
                "answered" if run.tally.yielded => "yielded",
                // An action that asks the model nothing, such as a compaction
                // or an edit's save, ends as completed, not as answered.
                "answered" if matches!(run.kind_name, "compact" | "edit" | "switch") => "completed",
                outcome => outcome,
            },
            code,
            requests = run.tally.requests,
            tool_rounds = run.tally.tool_rounds,
            compactions = run.tally.compactions,
            ms,
            "an action ended"
        );
    }

    /// The turn of the running action; none while no action runs.
    pub(super) fn running_turn(&self) -> Option<TurnId> {
        match &self.activity {
            Activity::Running(run) => Some(run.turn.clone()),
            _ => None,
        }
    }

    /// The running action takes over `turn`, as a retry does the turn it
    /// reruns.
    pub(super) fn take_over_turn(&mut self, turn: TurnId) {
        if let Activity::Running(run) = &mut self.activity {
            run.turn = turn;
        }
    }

    /// The action ended; its checkpoint is saved next. The human steers
    /// still pending open the next turn (`start_next`).
    pub(super) fn end_action(&mut self) {
        let idle_compaction = matches!(&self.activity, Activity::Running(run) if run.idle_compaction);
        self.activity = Activity::Finishing { idle_compaction };
        self.state_cause = "action_ended";
        // The closing save records the session idle, also after an action
        // whose last save recorded it running and that changed nothing
        // since, as a compaction pass.
        if self.persist.recorded != SessionPhase::Idle {
            self.mark_state_changed();
        }
        self.stop_waiting(WaitEnd::ActionEnd);
        self.end_turn_output();
        self.editing = None;
        self.release_media();
    }

    /// Whether the running action wrote into the transcript: one that wrote
    /// nothing has begun no turn, so nothing of it is left unfinished.
    fn action_began(&self) -> bool {
        matches!(
            &self.activity,
            Activity::Running(run) if run.started_at != self.transcript.version().revision
        )
    }

    /// Dispose stopped the running action, and the worker ends with it.
    ///
    /// - An action that wrote into the transcript is recorded as interrupted,
    ///   so the final checkpoint says it was running.
    /// - A message whose turn wrote nothing yet goes back to the front of the
    ///   queue instead.
    /// - Another action that wrote nothing ends with the session.
    /// - The human steers that wait go into the queue as messages, ahead of
    ///   what waited there, so the checkpoint keeps them.
    ///
    /// Returns whether a turn was interrupted.
    pub(super) fn shut_down(&mut self, kind: ActionKind, turn: &TurnId) -> bool {
        let began = self.action_began();
        self.reject_edit(EditError::Closed);
        let mut ahead = 0;
        if began {
            self.record_stop(CancelReason::Shutdown);
            self.log_turn_ended("interrupted", None);
        } else if let ActionKind::Send { content, comments, schedule } = kind {
            self.pending.push_front(PendingAction {
                kind: ActionKind::Send { content, comments, schedule },
                turn: turn.clone(),
                reply: None,
                admitted_at: self.clock.now(),
            });
            ahead = 1;
        }
        self.queue_steers(ahead);
        self.stop_waiting(WaitEnd::Shutdown);
        self.state_cause = "closed";
        self.activity = Activity::Closed { interrupted: began };
        began
    }

    /// Moves the human steers that wait into the queue as messages, after its
    /// first `ahead` entries: a session that closes keeps them in its
    /// checkpoint.
    fn queue_steers(&mut self, mut ahead: usize) {
        while let Some(steer) = self.inputs.take_first_steer() {
            self.pending.insert(ahead, PendingAction {
                kind: ActionKind::Send { content: steer.content, comments: steer.comments, schedule: None },
                turn: TurnId::try_from(steer.id.into_string()).expect("a block id is not empty"),
                reply: None,
                admitted_at: self.clock.now(),
            });
            ahead += 1;
        }
    }

    // Stop and dispose.

    /// Finds the one thing an `abort` stops: the running action, else the
    /// first waiting action. A session being disposed stops nothing more.
    pub(super) fn abort_step(&mut self) -> AbortStep {
        if self.disposing {
            return AbortStep::Nothing;
        }
        let now = self.clock.now();
        if let Activity::Running(run) = &mut self.activity
            && !run.cancel.is_cancelled()
        {
            let (target, stage) = match run.stage {
                TurnStage::Preparing => (AbortTarget::ActiveTurn, "preparing"),
                TurnStage::Streaming => (AbortTarget::ActiveProviderStream, "streaming"),
                TurnStage::Tools => (AbortTarget::ActiveTool, "tools"),
                TurnStage::Compacting => (AbortTarget::ActiveCompaction, "compacting"),
            };
            run.cancel.cancel(CancelReason::Stop);
            run.stop_at = Some(now);
            let cancel = run.cancel.clone();
            let turn = run.turn.clone();
            session_log!(
                self,
                info,
                event = "stop.requested",
                node = %self.id,
                turn = %turn,
                stage,
                target = %target,
                commands_running = self.watched.commands().len(),
                pending_calls = self.transcript.pending_tool_calls().len(),
                "the user stopped the running action"
            );
            return AbortStep::Running { target, cancel };
        }
        if let Some(mut action) = self.pending.pop_front() {
            action.end(ActionEnd::Dropped);
            let target = match action.kind {
                ActionKind::Send { .. } => AbortTarget::QueuedMessage,
                _ => AbortTarget::QueuedAction,
            };
            session_log!(
                self,
                info,
                event = "stop.requested",
                node = %self.id,
                stage = "pending",
                target = %target,
                commands_running = self.watched.commands().len(),
                "the user stopped a waiting action"
            );
            return AbortStep::Removed(target);
        }
        session_log!(
            self,
            info,
            event = "stop.requested",
            node = %self.id,
            stage = "idle",
            commands_running = self.watched.commands().len(),
            "the user stopped a session with nothing to stop"
        );
        AbortStep::Nothing
    }

    /// Stops the running action, as the user's Stop would, and nothing
    /// that waits: a waiting action stays. The answer is the stopped
    /// action's token, which says when it recorded the stop.
    pub(super) fn stop_running(&mut self, hold: Rc<str>) -> Option<Rc<TurnCancel>> {
        if self.disposing {
            return None;
        }
        let cancel = match &self.activity {
            Activity::Running(run) if !run.cancel.is_cancelled() => run.cancel.clone(),
            _ => return None,
        };
        cancel.cancel(CancelReason::Hold(hold));
        Some(cancel)
    }

    /// Whether another `abort` would stop something.
    pub(super) fn can_abort_again(&self) -> bool {
        let running =
            matches!(&self.activity, Activity::Running(run) if !run.cancel.is_cancelled());
        !self.disposing && (running || !self.pending.is_empty())
    }

    /// A stopped action records the stop.
    ///
    /// - A message its turn had not written yet is written for the user's
    ///   Stop, which then stops it as it would a moment later, and waits at
    ///   the front of the queue for a hold.
    /// - For the user's Stop, it writes the agent messages and the command
    ///   reports waiting for its next boundary; the human steers join the
    ///   next turn. A shutdown writes none, which the checkpoint keeps
    ///   (`shut_down`).
    /// - Then it completes running calls as aborted and appends the stopped
    ///   marker, or the interruption record when the session is shutting down.
    /// - A Stop of an action that wrote nothing, with what the stop wrote
    ///   counted, appends no marker: it began no turn.
    ///
    /// The Stop holds nothing afterwards.
    pub(super) fn record_stop(&mut self, reason: CancelReason) {
        let unwritten = match &mut self.activity {
            Activity::Running(run) => run.unwritten.take().map(|message| (run.turn.clone(), message)),
            _ => None,
        };
        if let Some((turn, message)) = unwritten {
            let QueuedMessage { content, comments, schedule, .. } = message;
            match reason {
                CancelReason::Stop => self.push_user(turn, content, comments, None, schedule),
                CancelReason::Hold(_) => self.pending.push_front(PendingAction {
                    kind: ActionKind::Send { content, comments, schedule },
                    turn,
                    reply: None,
                    admitted_at: self.clock.now(),
                }),
                CancelReason::Shutdown => {}
            }
        }
        if !matches!(reason, CancelReason::Shutdown) {
            self.write_inputs(Take::AllButSteers, "stop");
        }
        let end = match &reason {
            CancelReason::Stop => CallEnd::Stopped,
            CancelReason::Hold(hold) => CallEnd::Held(hold),
            CancelReason::Shutdown => CallEnd::Shutdown,
        };
        let calls_aborted = self.abort_executing_calls(&end);
        let marker_written = match reason {
            CancelReason::Stop | CancelReason::Hold(_) if self.action_began() => {
                self.transcript.push_abort(&self.model);
                true
            }
            CancelReason::Stop | CancelReason::Hold(_) => false,
            CancelReason::Shutdown => {
                self.append_interruption();
                true
            }
        };
        self.commit();
        let (turn, stopped_ms) = match &self.activity {
            Activity::Running(run) => (Some(run.turn.clone()), run.stop_at.map(|at| self.ms_since(at))),
            _ => (None, None),
        };
        session_log!(
            self,
            info,
            event = "stop.settled",
            node = %self.id,
            turn = turn.as_ref().map(tracing::field::display),
            reason = match &reason {
                CancelReason::Stop => "user",
                CancelReason::Hold(_) => "hold",
                CancelReason::Shutdown => "shutdown",
            },
            ms = stopped_ms,
            calls_aborted,
            commands_left_running = self.watched.commands().len(),
            marker_written,
            "a stopped action recorded its stop"
        );
    }

    /// A failed action writes the agent messages and the command reports
    /// that wait, and completes the calls it did not run, then reports the
    /// failure. The human steers open the next turn: the user's own act ends
    /// the failure's hold, as a message sent then does.
    pub(super) fn fail(&mut self, report: &ErrorReport) {
        self.write_inputs(Take::AllButSteers, "failure");
        self.abort_executing_calls(&CallEnd::Failed(&report.message));
        self.commit();
        self.outbox.push(SessionEvent::Error {
            report: report.clone(),
        });
    }

    /// Records that the turn the process died in is unfinished, unless the
    /// transcript already says so.
    pub(super) fn record_interruption(&mut self) {
        self.append_interruption();
        self.commit();
    }

    fn append_interruption(&mut self) {
        if self.transcript.ends_with_interruption() {
            return;
        }
        self.transcript.push_error(
            &self.model,
            INTERRUPTED_TURN_MESSAGE.to_owned(),
            Some(INTERRUPTED_CODE.to_owned()),
            None,
            false,
            None,
        );
    }

    /// Completes each running call as an error that says what ended it and what
    /// became of its command.
    ///
    /// The call keeps in its view the command it started, so that the command's
    /// later report and notice find their call. A command the backend's
    /// shutdown leaves running reports to the node from its next start, as one
    /// a call left running does.
    fn abort_executing_calls(&mut self, end: &CallEnd<'_>) -> usize {
        let calls = self.transcript.pending_tool_calls();
        let aborted = calls.len();
        for call in calls {
            let started = self
                .calls
                .remove(&call.tool_use_id)
                .and_then(|handle| handle.command());
            self.log_call_interrupted(&call, end, started.as_ref().map(|started| &started.command));
            let mut text = interrupted_text(end, started.as_ref().map(|started| &started.command));
            let view = started.as_ref().map(|started| {
                // The call shows how far its command got, as a result does.
                let others = text.chars().count() + "\noutput:\n".len();
                let budget = REPLAY_CHARS.saturating_sub(others);
                let look = self.runtime.look(&started.command, budget);
                let mut view = match look {
                    Some(CommandLook { output, view }) => {
                        text.push('\n');
                        text.push_str(&labelled_output(&output));
                        view
                    }
                    None => ShellToolView::named(started.command.clone(), ShellViewStatus::Running),
                };
                // A Stop or a hold stops the command with the call, whose
                // end may not have arrived yet.
                if matches!(end, CallEnd::Stopped | CallEnd::Held(_))
                    && view.status == ShellViewStatus::Running
                {
                    view.status = ShellViewStatus::Aborted;
                }
                ToolView::Shell(view)
            });
            if let (CallEnd::Shutdown, Some(started)) = (end, started) {
                self.watched.add(started.command, Watch::started(started.service));
            }
            self.transcript.complete_tool_call(
                &call.tool_use_id,
                vec![ToolResultContentBlock::Text { text }],
                true,
                view,
            );
        }
        aborted
    }

    /// Says that a call's result is an interruption, and what ended it.
    fn log_call_interrupted(&self, call: &demi_agent_transcript::PendingCall, end: &CallEnd<'_>, command: Option<&CommandId>) {
        if !self.one_request {
            log_interrupted(&self.span, &self.id, self.running_turn().as_ref(), call, end, command);
        }
    }

    /// Starts dispose: admissions end, the running action is stopped as a
    /// shutdown, and queued messages lose their callers but stay for the
    /// final checkpoint. Returns false when dispose already started.
    pub(super) fn begin_dispose(&mut self) {
        if self.disposing {
            return;
        }
        self.disposing = true;
        for action in &mut self.pending {
            action.end(ActionEnd::Detached);
        }
        // Only queued messages are the checkpoint's; other waiting actions
        // end with the session.
        self.pending.retain(PendingAction::is_send);
        if let Activity::Running(run) = &self.activity {
            run.cancel.cancel(CancelReason::Shutdown);
        }
        // A finishing action saves, then starts nothing.
        if matches!(self.activity, Activity::Idle) {
            self.activity = Activity::Closed { interrupted: false };
        }
    }

    /// Every provider runtime the session holds, for closing.
    pub(super) fn take_runtimes(&mut self) -> Vec<Box<dyn ProviderRuntime>> {
        let mut runtimes: Vec<_> = self.take_runtime().into_iter().collect();
        runtimes.extend(self.switch.take().and_then(|switch| switch.runtime));
        runtimes.extend(self.waiting_switch.take().and_then(|switch| switch.runtime));
        runtimes.append(&mut self.retired);
        runtimes
    }

    // The queue.

    pub(super) fn dequeue(&mut self, id: &TurnId) -> bool {
        if self.disposing {
            return false;
        }
        let Some(mut action) = self.take_queued(id) else {
            return false;
        };
        self.log_recalled(id.as_str(), "queued");
        action.end(ActionEnd::Dropped);
        true
    }

    fn log_recalled(&self, input: &str, from: &'static str) {
        session_log!(
            self,
            info,
            event = "input.recalled",
            node = %self.id,
            input,
            from,
            "a waiting message was taken back"
        );
    }

    /// Moves a queued message ahead of every other, so it runs next.
    pub(super) fn send_next(&mut self, id: &TurnId) -> bool {
        if self.disposing {
            return false;
        }
        let Some(action) = self.take_queued(id) else {
            return false;
        };
        let front = self
            .pending
            .iter()
            .position(PendingAction::is_send)
            .unwrap_or(self.pending.len());
        self.pending.insert(front, action);
        true
    }

    /// Sends a queued message now: it moves to the front of the queue and,
    /// while a turn runs, cuts the turn's round short and ends the turn at
    /// the boundary. False when no queued message has the id.
    pub(super) fn send_queued_now(&mut self, id: &TurnId) -> bool {
        if !self.send_next(id) {
            return false;
        }
        self.cut_round(SendNow::Message);
        session_log!(
            self,
            info,
            event = "input.admitted",
            node = %self.id,
            turn = self.running_turn().map(tracing::field::display),
            input = %id,
            kind = "queued",
            path = "send_now",
            "a queued message was sent now"
        );
        true
    }

    pub(super) fn clear_queue(&mut self) -> usize {
        if self.disposing {
            return 0;
        }
        let mut cleared = Vec::new();
        self.pending.retain_mut(|action| {
            if !action.is_send() {
                return true;
            }
            action.end(ActionEnd::Dropped);
            cleared.push(action.turn.clone());
            false
        });
        for turn in &cleared {
            self.log_recalled(turn.as_str(), "queued");
        }
        cleared.len()
    }

    fn take_queued(&mut self, id: &TurnId) -> Option<PendingAction> {
        let index = self
            .pending
            .iter()
            .position(|action| action.is_send() && &action.turn == id)?;
        self.pending.remove(index)
    }

    pub(super) fn queued_messages(&self) -> Vec<QueuedMessage> {
        self.pending
            .iter()
            .filter_map(|action| match &action.kind {
                ActionKind::Send { content, comments, schedule } => Some(QueuedMessage {
                    id: action.turn.clone(),
                    content: content.clone(),
                    comments: comments.clone(),
                    schedule: schedule.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    // Steers and agent messages.

    /// Accepts a human steer, whatever runs, while the session is open.
    ///
    /// - With nothing running, it is the message that starts the turn.
    /// - Otherwise it waits for the running model's turn to take it at its
    ///   next boundary; when the action ends with no turn that did, the
    ///   scheduler opens the next turn with it (`start_next`).
    ///
    /// A steer whose id the session holds, pending, written or as the turn
    /// it opened, was taken already, as a page that lost the answer sends it
    /// again: it is accepted once.
    pub(super) fn steer(
        &mut self,
        id: BlockId,
        content: Vec<UserContentBlock>,
        comments: Vec<MessageComment>,
    ) -> Result<(), SteerError> {
        let own_turn = TurnId::try_from(id.as_str().to_owned()).expect("a block id is not empty");
        if self.inputs.pending_steers().iter().any(|steer| steer.id == id)
            || self.transcript.find(&id).is_some()
            || self.knows_turn(&own_turn)
        {
            return Ok(());
        }
        if self.disposing || matches!(self.activity, Activity::Closed { .. }) {
            return Err(SteerError::Closed);
        }
        if matches!(self.activity, Activity::Idle) {
            session_log!(
                self,
                info,
                event = "input.admitted",
                node = %self.id,
                turn = %own_turn,
                input = %id,
                kind = "steer",
                path = "starts_turn",
                "a steer was admitted while nothing ran, and starts the turn"
            );
            drop(self.push_action(ActionKind::Send { content, comments, schedule: None }, own_turn));
            return Ok(());
        }
        // A steer no running turn will take, as one sent after a Stop, names
        // the turn it is to open.
        let turn = match &self.activity {
            Activity::Running(run) if !run.cancel.is_cancelled() => run.turn.clone(),
            _ => own_turn,
        };
        session_log!(
            self,
            info,
            event = "input.admitted",
            node = %self.id,
            turn = %turn,
            input = %id,
            kind = "steer",
            path = "pending_for_boundary",
            pending = self.inputs.pending_steers().len() + 1,
            "a steer was admitted for the turn's next boundary"
        );
        self.inputs.note_admitted(id.as_str(), self.clock.now());
        self.inputs.add(Input::Steer(PendingSteer {
            id,
            turn_id: turn,
            model: self.model.clone(),
            content,
            comments,
        }));
        Ok(())
    }

    pub(super) fn cancel_steer(&mut self, id: &BlockId) -> bool {
        let recalled = self.inputs.cancel_steer(id);
        if recalled {
            self.log_recalled(id.as_str(), "pending");
        }
        recalled
    }

    /// Delivers the pending steer `id` now: the turn's round is cut
    /// short, and the turn goes on with the steer. A steer that is no
    /// longer pending changes nothing.
    ///
    /// A wait for the model that runs starts again from now: the request it
    /// waited for is stopped, and the next one carries the steer. A wait that
    /// does not run, as while a call runs, starts before that next request.
    pub(super) fn steer_now(&mut self, id: &BlockId) -> bool {
        if !self.inputs.has_steer(id) {
            return false;
        }
        self.cut_round(SendNow::Steer);
        session_log!(
            self,
            info,
            event = "input.admitted",
            node = %self.id,
            turn = self.running_turn().map(tracing::field::display),
            input = %id,
            kind = "steer",
            path = "send_now",
            "a pending steer was sent now"
        );
        if self.send_now_pending() && self.waiting_since.is_some() {
            self.stop_waiting(WaitEnd::SteerNow);
            self.start_waiting(WaitStart::InputArrived);
        }
        true
    }

    /// Asks the running turn to cut its round short for what the user sent
    /// now. A stopped action, and one that prepares an edit, runs no turn to
    /// cut; nor does a finishing or an idle session.
    fn cut_round(&mut self, now: SendNow) {
        if self.preparing_edit() {
            return;
        }
        if let Activity::Running(run) = &mut self.activity
            && !run.cancel.is_cancelled()
        {
            run.send_now = run.send_now.max(Some(now));
        }
    }

    /// Whether the user sent a message now that the running turn has not
    /// taken yet.
    pub(super) fn send_now_pending(&self) -> bool {
        matches!(&self.activity, Activity::Running(run) if run.send_now.is_some())
    }

    /// Takes what the user sent now, at a boundary.
    pub(super) fn take_send_now(&mut self) -> Option<SendNow> {
        match &mut self.activity {
            Activity::Running(run) => run.send_now.take(),
            _ => None,
        }
    }

    /// Admits an agent message for the next boundary.
    ///
    /// A message the session already has, pending or written, is admitted once:
    /// the same content counts as admitted, other content under its id is
    /// refused. `stored` is the block of the message's id before the window, as
    /// the store holds it, when there is one.
    pub(super) fn admit_agent_message(
        &mut self,
        message: AgentMessage,
        stored: Option<&Block>,
    ) -> Result<(), AgentMessageError> {
        if message.recipient_id != self.id {
            return Err(AgentMessageError::Recipient);
        }
        if self.disposing {
            return Err(AgentMessageError::Closed);
        }
        if self.preparing_edit() {
            return Err(AgentMessageError::Editing);
        }
        let written = self.transcript.find(&message.id).or(stored);
        let existing = self.inputs.agent_message(&message.id).or_else(|| {
            written.and_then(|block| match block {
                Block::AgentMessage(block) => Some(&block.message),
                _ => None,
            })
        });
        if let Some(existing) = existing {
            if existing != &message {
                return Err(AgentMessageError::DifferentContent);
            }
            self.wake();
            return Ok(());
        }
        if self.inputs.contains(message.id.as_str()) || written.is_some() {
            return Err(AgentMessageError::Conflict);
        }
        let turn_id = match &self.activity {
            Activity::Running(run) => run.turn.clone(),
            _ => TurnId::try_from(message.id.as_str()).expect("a message id is never empty"),
        };
        let from_product = message.event.is_from_product();
        session_log!(
            self,
            info,
            event = "input.admitted",
            node = %self.id,
            turn = %turn_id,
            input = %message.id,
            kind = "agent_message",
            from_node = message.sender.as_ref().map(|sender| sender.id.to_string()),
            path = if matches!(self.activity, Activity::Running(_)) { "pending_for_boundary" } else { "starts_turn" },
            held = self.held,
            "an agent message was admitted"
        );
        self.inputs.note_admitted(message.id.as_str(), self.clock.now());
        self.inputs.add(Input::Agent(PendingAgentInput {
            turn_id,
            model: self.model.clone(),
            message,
        }));
        if from_product {
            self.wake_for_user();
        } else {
            self.wake();
        }
        Ok(())
    }

    /// Writes the waiting input `take` names into the running turn, in arrival
    /// order.
    ///
    /// The command reports among it are one `wakeup` block, one paragraph each,
    /// where the first of them arrived. Returns whether an agent message was
    /// written, which is saved at once.
    pub(super) fn write_inputs(&mut self, take: Take, boundary: &'static str) -> bool {
        let turn = self.turn();
        self.write_inputs_into(turn, take, boundary)
    }

    /// [`write_inputs`](Self::write_inputs) into `turn`.
    fn write_inputs_into(&mut self, turn: TurnId, take: Take, boundary: &'static str) -> bool {
        self.drop_seen_ends();
        let inputs = self.inputs.take(take);
        if inputs.is_empty() {
            return false;
        }
        let mut reports: Vec<CommandReport> = inputs
            .iter()
            .filter_map(|input| match input {
                Input::Report(report, _) => Some(report.clone()),
                _ => None,
            })
            .collect();
        let mut agent_message = false;
        for input in inputs {
            match input {
                Input::Steer(steer) => {
                    self.log_written(&turn, steer.id.as_str(), "steer", boundary);
                    self.transcript
                        .push_steer(steer.id, turn.clone(), &steer.model, steer.content, steer.comments);
                }
                Input::Report(..) if reports.is_empty() => {}
                Input::Report(..) => {
                    let reports = mem::take(&mut reports);
                    self.push_reports(turn.clone(), WakeupPlacement::Steer, reports, boundary);
                }
                Input::Agent(input) => {
                    agent_message = true;
                    self.log_written(&turn, input.message.id.as_str(), "agent_message", boundary);
                    // A message no agent sent is the user's or Demi's,
                    // which ends a run wherever it stands.
                    let sender_below = input
                        .message
                        .sender
                        .as_ref()
                        .is_some_and(|sender| self.runtime.is_below(&sender.id));
                    self.transcript
                        .push_agent_message(turn.clone(), &input.model, input.message, sender_below);
                }
            }
        }
        self.commit();
        agent_message
    }

    /// A steer or an agent message is written into `turn` at `boundary`.
    fn log_written(&mut self, turn: &TurnId, input: &str, kind: &'static str, boundary: &'static str) {
        let pending_ms = self.inputs.take_admitted(input).map(|at| self.ms_since(at));
        session_log!(
            self,
            info,
            event = "input.written",
            node = %self.id,
            turn = %turn,
            input,
            kind,
            boundary,
            pending_ms,
            "waiting input was written into its turn"
        );
    }

    /// Writes what a continuation opens with: the command reports as one
    /// `wakeup` block that opens the turn, or else the waiting agent messages.
    ///
    /// False when nothing waits any more, and the continuation ends without a
    /// turn.
    pub(super) fn open_continuation(&mut self) -> Option<bool> {
        self.drop_seen_ends();
        let reports = self.inputs.take_reports();
        if !reports.is_empty() {
            let turn = self.turn();
            self.push_reports(turn, WakeupPlacement::NewTurn, reports, "continuation");
            self.commit();
            return Some(false);
        }
        if self.inputs.has_agent_input() {
            return Some(self.write_inputs(Take::Everything, "continuation"));
        }
        None
    }

    /// A `wakeup` block of command reports, in the order they arrived, each
    /// readied as it is written: only now does it move the node's place in
    /// its command's output.
    fn push_reports(
        &mut self,
        turn: TurnId,
        placement: WakeupPlacement,
        mut reports: Vec<CommandReport>,
        boundary: &'static str,
    ) {
        session_log!(
            self,
            info,
            event = "command_report.written",
            node = %self.id,
            turn = %turn,
            placement = %placement,
            boundary,
            reports = reports.len(),
            commands = reports.iter().map(|report| report.command_id.as_str()).collect::<std::collections::BTreeSet<_>>().len(),
            ends = reports.iter().filter(|report| is_end(report)).count(),
            "command reports were written into a turn"
        );
        for report in &mut reports {
            self.runtime.write_report(report);
        }
        let id = BlockId::try_from(self.ids.next_id())
            .expect("an id source never gives an empty identity");
        self.transcript
            .push_wakeup(id, turn, &self.model, placement, reports);
    }

    // Command reports.

    /// Watches `command`, which a call left running, so that its end wakes
    /// the node, and its silence once unless it is a `service`. A
    /// Stop of the action leaves it running.
    pub(super) fn watch_command(&mut self, command: CommandId, service: bool) {
        self.watched.add(command, Watch::started(service));
    }

    /// Admits a command's report for the next boundary.
    ///
    /// It joins the running turn, or opens a continuation unless it `opens`
    /// nothing, when it waits for the node's next turn
    /// (`crates/agent-session/src/reports.rs`). A session that is closing keeps
    /// no more input.
    pub(super) fn admit_report(&mut self, report: CommandReport, opens: Opens) {
        let dropped = if self.disposing {
            Some("closing")
        } else if end_seen(self.runtime.as_ref(), &report) {
            Some("seen_in_look")
        } else {
            None
        };
        if let Some(reason) = dropped {
            self.log_report_dropped(&report, reason);
            return;
        }
        let running = matches!(self.activity, Activity::Running(_));
        let waits = self.held || self.failed_turn_holds();
        session_log!(
            self,
            info,
            event = "command_report.admitted",
            node = %self.id,
            turn = self.running_turn().map(tracing::field::display),
            command = %report.command_id,
            kind = report_kind(&report),
            placement = match (running, opens, waits) {
                (true, ..) => "into_open_turn",
                (false, Opens::Turn, false) => "opens_turn",
                _ => "waits",
            },
            turn_open = running,
            held = self.held,
            "a command report was admitted"
        );
        self.inputs.add(Input::Report(report, opens));
        self.wake();
    }

    fn log_report_dropped(&self, report: &CommandReport, reason: &'static str) {
        session_log!(
            self,
            info,
            event = "command_report.dropped",
            node = %self.id,
            turn = self.running_turn().map(tracing::field::display),
            command = %report.command_id,
            kind = report_kind(report),
            reason,
            "a command report was dropped"
        );
    }

    /// How many inputs arrived, which a shell call counts
    /// (`crates/agent-session/src/runtime.rs`).
    pub(super) fn arrivals(&self) -> u64 {
        self.inputs.arrivals()
    }

    /// Whether input arrived since `before` that is still to be written:
    /// an end report whose end a look showed meanwhile is withdrawn first.
    pub(super) fn arrived_since(&mut self, before: u64) -> bool {
        self.drop_seen_ends();
        self.inputs.arrivals() > before
    }

    /// Drops the waiting end reports whose ends a look showed the node
    /// since they were made.
    fn drop_seen_ends(&mut self) {
        let runtime = self.runtime.clone();
        for report in self.inputs.retain_reports(|report| !end_seen(runtime.as_ref(), report)) {
            self.log_report_dropped(&report, "seen_in_look");
        }
    }

    // The model selection.

    /// Records `switch` in place of the pending one; while an edit is
    /// being prepared, in place of the one that waits for it. A session
    /// that is closing gives it back.
    pub(super) fn record_switch(&mut self, switch: ModelSwitch) -> Result<(), ModelSwitch> {
        if self.disposing {
            return Err(switch);
        }
        let slot = if self.preparing_edit() {
            &mut self.waiting_switch
        } else {
            &mut self.switch
        };
        replace_switch(slot, switch, &mut self.retired);
        Ok(())
    }

    /// The model the recorded switch lands with at the next provider
    /// request, and what its vendor takes in one request, from the runtime
    /// it lands on.
    pub(super) fn switch_target(&self) -> Option<(ModelSelection, RequestLimits)> {
        let switch = self.switch.as_ref()?;
        let runtime = switch
            .runtime
            .as_deref()
            .or(self.provider.held())
            .expect("the provider runtime is in its slot between runs");
        let limits = runtime.request_limits(&switch.model.model);
        Some((ModelSelection::clone(&switch.model), limits))
    }

    /// Makes the recorded switch current, and returns the runtimes it
    /// replaced for closing. Runs between two requests, when the runtime is
    /// in its slot.
    pub(super) fn install_switch(&mut self) -> Vec<Box<dyn ProviderRuntime>> {
        let Some(switch) = self.switch.take() else {
            return Vec::new();
        };
        let mut replaced = mem::take(&mut self.retired);
        let new_runtime = switch.runtime.is_some();
        if let Some(runtime) = switch.runtime {
            replaced.extend(self.place_runtime(runtime));
        }
        if self.model != *switch.model {
            session_log!(
                self,
                info,
                event = "model_switch.applied",
                node = %self.id,
                turn = self.running_turn().map(tracing::field::display),
                from_provider = %self.model.provider_id,
                to_provider = %switch.model.provider_id,
                from_model = self.model.model.id,
                to_model = switch.model.model.id,
                new_runtime,
                replayed_blocks = self.replayed().len(),
                "a model switch landed"
            );
            self.model = *switch.model;
            self.persist.dirty = true;
            self.requests.save = true;
        }
        replaced
    }

    /// The selection the next request uses once the recorded switch lands.
    /// A switch that waits for an edit lands later.
    pub(super) fn landing_selection(&self) -> &ModelSelection {
        self.switch
            .as_ref()
            .map_or(&self.model, |switch| &switch.model)
    }

    /// The selection the last switch named: the one that waits for an edit,
    /// else the one the next request lands.
    pub(super) fn latest_selection(&self) -> &ModelSelection {
        self.waiting_switch
            .as_ref()
            .map_or_else(|| self.landing_selection(), |switch| &switch.model)
    }

    // The transcript during a turn.

    /// Appends the message's `user` block.
    pub(super) fn push_user(
        &mut self,
        turn: TurnId,
        content: Vec<UserContentBlock>,
        comments: Vec<MessageComment>,
        preamble: Option<String>,
        schedule: Option<ScheduleMark>,
    ) {
        session_log!(
            self,
            info,
            event = "input.written",
            node = %self.id,
            turn = %turn,
            input = %turn,
            kind = "user",
            boundary = "turn_start",
            pending_ms = match &self.activity {
                Activity::Running(run) => run.queued_ms,
                _ => 0,
            },
            "a message was written into its turn"
        );
        self.transcript
            .push_user(turn, &self.model, content, comments, preamble, schedule);
        if let Activity::Running(run) = &mut self.activity {
            run.unwritten = None;
        }
        self.commit();
    }

    pub(super) fn push_context(&mut self, news: crate::runtime::NewContext) {
        let turn = self.turn();
        self.transcript.push_context(
            turn,
            &self.model,
            news.source,
            news.text,
            news.instructions,
            news.cwd,
        );
        self.commit();
    }

    /// Appends a `resume` block: the turn continues after a cut.
    pub(super) fn push_resume(&mut self, reason: Option<String>) {
        let turn = self.turn();
        self.transcript.push_resume(turn, &self.model, reason);
        self.commit();
    }

    pub(super) fn mark_abort_resumed(&mut self) {
        self.transcript.mark_latest_abort_resumed();
        self.commit();
    }

    /// Applies one provider event.
    pub(super) fn apply_event(&mut self, event: ProviderEvent) {
        if let ProviderEvent::Entries { of, entries } = event {
            self.keep_entries(of, entries);
            return;
        }
        // The wait for the model ends with its first output:
        //
        // - reasoning as soon as its block starts, with text or without,
        //   redacted reasoning too;
        // - text;
        // - a whole call, or a call being written once its title is.
        //
        // The response's usage and a call without its title show nothing, and
        // the row goes on saying Requesting.
        let shows = event.is_output()
            && !matches!(
                event,
                ProviderEvent::ToolCallStart { .. } | ProviderEvent::ToolCallInput { .. }
            );
        self.count_output(&event);
        let before = self.transcript.len();
        self.apply_event_to_transcript(event);
        if shows {
            self.stop_waiting(WaitEnd::FirstOutput);
        } else if self.writing.iter().any(|writing| writing.call.description.is_some()) {
            self.stop_waiting(WaitEnd::CallTitle);
        }
        // The blocks a run's events write, which its entries name by
        // position (`EntriesOf::Output`).
        let written = self.transcript.from(before)
            .iter()
            .filter(|block| {
                matches!(
                    block,
                    Block::Thinking(_) | Block::RedactedThinking(_) | Block::Text(_) | Block::ToolCall(_)
                )
            })
            .map(|block| block.id().clone());
        self.request_blocks.written.extend(written);
        self.commit();
    }

    /// Keeps a run's entries on the block they belong to: one that gives an
    /// item of the request, or one the run wrote. Entries that name no such
    /// block belong to nothing the session keeps.
    pub(super) fn keep_entries(&mut self, of: EntriesOf, entries: Vec<demi_shared_types::Entry>) {
        let blocks = &self.request_blocks;
        let block = match of {
            EntriesOf::Item(index) => blocks.items.get(index),
            EntriesOf::Output(index) => blocks.written.get(index),
        };
        let Some(block) = block.cloned() else {
            return;
        };
        self.transcript.add_entries(&block, entries);
        self.commit();
    }

    /// A new request starts: its output counts from zero, whatever the
    /// request before it in the turn, a failed attempt included, wrote,
    /// which counts for the turn's speed.
    pub(super) fn start_output(&mut self, request_id: String) {
        self.add_to_turn_output();
        self.output = Some(StreamedOutput {
            request_id,
            estimate: OutputEstimate::default(),
            reported: None,
            first_at: None,
            ended_at: None,
            sent_at: self.clock.now(),
            cache_lifetime: None,
            carried: None,
        });
    }

    /// When the latest request's conversation compacts while idle, by the
    /// conditions the request decides: a window in use of `window` and a
    /// context large enough, and a cache lifetime its provider said. None
    /// when any fails, or before the request was answered.
    pub(super) fn idle_compaction_due(&self, window: Option<u64>) -> Option<IdleCompactionDue> {
        let output = self.output.as_ref()?;
        let lifetime = output.cache_lifetime?;
        if window? < IDLE_COMPACTION_WINDOW || output.carried? < IDLE_COMPACTION_CONTEXT {
            return None;
        }
        let before_expiry = lifetime * IDLE_COMPACTION_SHARE.0 / IDLE_COMPACTION_SHARE.1;
        let at = output.sent_at.as_millisecond() + i64::try_from(before_expiry.as_millis()).ok()?;
        Some(IdleCompactionDue {
            at: Timestamp::from_millisecond(at).ok()?,
            lifetime,
        })
    }

    /// The running request ended without a response, failed, stopped or
    /// cut short: its output stays as it streamed.
    pub(super) fn end_output(&mut self) {
        let now = self.clock.now();
        if let Some(output) = self.output.as_mut().filter(|output| output.ended_at.is_none()) {
            output.ended_at = Some(now);
        }
    }

    /// Adds what `event` streamed to the running request's output.
    ///
    /// Each response ends its request: output after it in the same run, as
    /// when Claude Code answers several model calls in one run, is the next
    /// request's, counted from zero.
    fn count_output(&mut self, event: &ProviderEvent) {
        let now = self.clock.now();
        if event.is_output() && self.output.as_ref().is_some_and(|output| output.ended_at.is_some()) {
            let request_id = self.ids.next_id();
            self.start_output(request_id);
        }
        let writing = &self.writing;
        let Some(output) = self.output.as_mut().filter(|output| output.ended_at.is_none()) else {
            return;
        };
        let before = output.estimate;
        match event {
            ProviderEvent::ThinkingDelta(text) | ProviderEvent::TextDelta(text) => output.estimate.add_text(text),
            ProviderEvent::ToolCallStart { tool_use_id, .. } => {
                if !writing.iter().any(|writing| &writing.call.tool_use_id == tool_use_id) {
                    output.estimate.add_call();
                }
            }
            ProviderEvent::ToolCallInput { partial_json, .. } => output.estimate.add_text(partial_json),
            // The arguments count when they come whole, unless they
            // streamed in pieces; a call that never opened counts its
            // framing too.
            ProviderEvent::ToolCall(call) => {
                match writing.iter().find(|writing| writing.call.tool_use_id == call.tool_use_id) {
                    Some(opened) if !opened.input.is_empty() => {}
                    Some(_) => output.estimate.add_text(&call.input.to_string()),
                    None => {
                        output.estimate.add_call();
                        output.estimate.add_text(&call.input.to_string());
                    }
                }
            }
            ProviderEvent::Response(usage) => {
                output.reported = usage.map(|usage| usage.output_tokens);
                output.carried =
                    usage.map(|usage| usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens);
                output.ended_at = Some(now);
            }
            ProviderEvent::CacheLifetime(lifetime) => output.cache_lifetime = Some(*lifetime),
            _ => {}
        }
        // The speed's clock starts at the first output that adds to the
        // count, not at a reasoning block that opens empty.
        if output.first_at.is_none() && output.estimate != before {
            output.first_at = Some(now);
        }
    }

    /// Adds the latest request's output to the turn's, when it streamed.
    fn add_to_turn_output(&mut self) {
        let now = self.clock.now();
        let Some(output) = &self.output else {
            return;
        };
        if let Some(streamed_ms) = output.streamed_ms(now) {
            self.turn_output.tokens += output.tokens();
            self.turn_output.streamed_ms += streamed_ms;
        }
    }

    /// The turn ends: its output goes, and its speed is kept when it
    /// streamed long enough to tell one.
    fn end_turn_output(&mut self) {
        self.add_to_turn_output();
        self.output = None;
        let turn = mem::take(&mut self.turn_output);
        if turn.streamed_ms < TURN_SPEED_AFTER_MS {
            return;
        }
        let speed = Some(TurnSpeed {
            tokens: turn.tokens,
            streamed_ms: turn.streamed_ms,
        });
        if speed != self.turn_speed {
            self.turn_speed = speed;
            self.mark_state_changed();
        }
    }

    /// The output of the turn's latest request as the pages see it now.
    pub(super) fn request_output(&self) -> Option<RequestOutput> {
        let output = self.output.as_ref()?;
        Some(RequestOutput {
            request_id: output.request_id.clone(),
            tokens: output.tokens(),
            streamed_ms: output.streamed_ms(self.clock.now()),
            ended: output.ended_at.is_some(),
        })
    }

    fn apply_event_to_transcript(&mut self, event: ProviderEvent) {
        match event {
            ProviderEvent::ThinkingStart => {
                self.transcript.open_thinking(&self.model, String::new());
            }
            ProviderEvent::ThinkingDelta(text) => {
                self.transcript.append_thinking(&self.model, &text)
            }
            ProviderEvent::ThinkingSignature(signature) => {
                self.transcript.sign_thinking(signature)
            }
            ProviderEvent::RedactedThinking(data) => {
                self.transcript.push_redacted_thinking(&self.model, data);
            }
            ProviderEvent::TextDelta(text) => {
                self.transcript.append_text(&self.model, &text)
            }
            ProviderEvent::ToolCallStart {
                tool_use_id,
                tool_name,
            } => {
                if !self.writing.iter().any(|writing| writing.call.tool_use_id == tool_use_id) {
                    self.writing.push(WritingCall {
                        call: PendingCall {
                            tool_use_id,
                            tool_name,
                            description: None,
                        },
                        input: String::new(),
                    });
                }
            }
            ProviderEvent::ToolCallInput {
                tool_use_id,
                partial_json,
            } => {
                if let Some(writing) = self
                    .writing
                    .iter_mut()
                    .find(|writing| writing.call.tool_use_id == tool_use_id)
                {
                    writing.input.push_str(&partial_json);
                    if writing.call.description.is_none() {
                        writing.call.description = written_description(&writing.input);
                    }
                }
            }
            // The call leaves the calls being written in the step
            // that adds its block.
            ProviderEvent::ToolCall(call) => {
                self.writing
                    .retain(|writing| writing.call.tool_use_id != call.tool_use_id);
                self.transcript.push_tool_call(&self.model, call)
            }
            ProviderEvent::Response(usage) => {
                self.transcript.push_response(&self.model, usage)
            }
            ProviderEvent::Error(failure) => self.push_failure(&(&failure).into()),
            ProviderEvent::Account(account) => self.take_account(account),
            ProviderEvent::AccountSwitch(switch) => {
                self.transcript.push_account_switch(&self.model, switch)
            }
            ProviderEvent::Entries { .. } => {
                unreachable!("apply_event keeps entries before it writes blocks")
            }
            // A call's accounting is the usage ledger's, which the metered
            // runtime writes; the transcript keeps the run's response. The
            // cache's lifetime is the request's output's.
            ProviderEvent::Call(_) | ProviderEvent::CacheLifetime(_) => {}
        }
    }

    /// Keeps the account the agent uses from now on with the session's
    /// state; the clients learn it with the next context usage.
    pub(super) fn take_account(&mut self, account: Option<AgentAccount>) {
        if self.account != account {
            self.account = account;
            self.mark_state_changed();
        }
    }

    /// The request ended, failed or was cancelled: no call is being written
    /// any more.
    pub(super) fn end_writing(&mut self) {
        self.writing.clear();
    }

    /// The node's turn needs the model's next answer: it waits from now,
    /// unless a wait runs already, whose clock runs on, as across a
    /// retry.
    pub(super) fn start_waiting(&mut self, cause: WaitStart) {
        if self.waiting_since.is_none() {
            self.waiting_since = Some(self.clock.now());
            session_log!(
                self,
                info,
                event = "model_wait.started",
                node = %self.id,
                turn = self.running_turn().map(tracing::field::display),
                cause = cause.as_str(),
                "the node began to wait for the model"
            );
        }
    }

    /// The model's output shows, a compaction pass begins, or the action
    /// ended.
    pub(super) fn stop_waiting(&mut self, by: WaitEnd) {
        if let Some(ms) = self.waited_ms() {
            session_log!(
                self,
                info,
                event = "model_wait.ended",
                node = %self.id,
                turn = self.running_turn().map(tracing::field::display),
                ms,
                by = by.as_str(),
                "the node stopped waiting for the model"
            );
        }
        self.waiting_since = None;
    }

    /// How long the node has waited for the model, while it waits.
    pub(super) fn waited_ms(&self) -> Option<u64> {
        let now = self.clock.now().as_millisecond();
        self.waiting_since
            .map(|since| u64::try_from(now - since.as_millisecond()).unwrap_or(0))
    }

    /// The calls the model is writing, oldest first.
    pub(super) fn pending_calls(&self) -> Vec<PendingCall> {
        self.writing.iter().map(|writing| writing.call.clone()).collect()
    }

    /// Records a failed provider request, the turn's own or a compaction's
    /// summary request, as an `error` block.
    pub(super) fn record_failure(&mut self, failure: &ErrorReport) {
        self.push_failure(failure);
        self.commit();
    }

    /// A failure of an action that has not begun ended no turn, as a
    /// `compact` action whose summary request failed: its record says
    /// so.
    fn push_failure(&mut self, failure: &ErrorReport) {
        let outside_turn = !self.action_began();
        self.transcript.push_error(
            &self.model,
            failure.message.clone(),
            failure.code.clone(),
            failure.diagnostics.clone(),
            outside_turn,
            self.account
                .as_ref()
                .map(|account| account.credential_id.clone()),
        );
    }

    /// Marks the answer text at the end complete, and answers whether it
    /// did.
    pub(super) fn complete_tail_text(&mut self) -> bool {
        let completed = self.transcript.complete_tail_text();
        self.commit();
        completed
    }

    /// Completes the call `tool_use_id` with its result as the transcript
    /// holds it: its media by reference.
    pub(super) fn complete_tool_call(
        &mut self,
        tool_use_id: &str,
        output: Vec<ToolResultContentBlock>,
        is_error: bool,
        view: Option<ToolView>,
    ) {
        self.calls.remove(tool_use_id);
        self.transcript
            .complete_tool_call(tool_use_id, output, is_error, view);
        self.commit();
    }

    /// Keeps `files` with the complete text `id` (`AgentSession::keep_code_files`).
    pub(super) fn keep_code_files(&mut self, id: &BlockId, files: Vec<demi_shared_types::CodeFile>) {
        if self.transcript.keep_code_files(id, files) {
            self.commit();
        }
    }

    /// Settles the picture of the text `id` named by `target`
    /// (`AgentSession::settle_picture`).
    pub(super) fn settle_picture(&mut self, id: &BlockId, target: &str, copy: demi_shared_types::PictureCopy) {
        if self.transcript.settle_picture(id, target, copy) {
            self.commit();
        }
    }

    /// Gives the call `id` the view `view`, as its lists change while the
    /// calls before it in its step return.
    pub(super) fn replace_view(&mut self, id: &BlockId, view: ToolView) {
        self.transcript.replace_view(id, view);
        self.commit();
    }

    /// The next request, whose items replay `view`, the model's view of the
    /// replayed blocks ([`Self::model_view`]).
    pub(super) fn inference_request(
        &mut self,
        view: &ModelView,
        system_prompt: String,
        request_id: String,
        cancel: CancellationToken,
    ) -> InferenceRequest {
        let mut replayed = replay(&self.request_view(view));
        // A session copy's instruction joins the input that ends its history.
        if self.one_request {
            super::compaction::join_instruction(&mut replayed);
        }
        let mut items = Vec::with_capacity(replayed.items.len());
        let mut blocks = Vec::with_capacity(replayed.blocks.len());
        for block in replayed.blocks {
            items.extend(block.items.clone().map(|_| block.id.clone()));
            blocks.push(RequestBlock {
                items: block.items,
                entries: block.entries,
            });
        }
        self.request_blocks = RequestBlocks {
            items,
            written: Vec::new(),
        };
        let turn = self.request_turn.clone().unwrap_or_else(|| self.turn());
        InferenceRequest {
            session_id: self.id.to_string(),
            affinity: self.runtime.affinity(),
            turn_id: turn.to_string(),
            request_id,
            model_id: self.model.model.id.clone(),
            output_limit: self.model.model.output_limit.and_then(NonZeroU32::new),
            output_cap: None,
            system_prompt,
            items: replayed.items.into(),
            blocks: blocks.into(),
            tools: self.tools.clone(),
            thinking: self.model.thinking.clone(),
            service_tier_id: self.model.service_tier_id.clone(),
            prompt_cache: PromptCache::Session {
                answered_items: replayed.answered,
            },
            cancel,
            account: self.account.clone(),
            // A session copy sends a compaction pass's summary request.
            purpose: if self.one_request {
                RequestPurpose::Compaction
            } else {
                RequestPurpose::Turn
            },
            agent: self.runtime.agent(),
        }
    }

    // Readying ahead (`crates/provider-claude-code/src/prepared.rs`).

    /// Asks for the next request's runtime to ready ahead, once the session is
    /// idle.
    ///
    /// A page that opened the conversation or a choice of the user asks first,
    /// and makes every later action's end ask too; what they ask goes before an
    /// action's end.
    pub(super) fn ask_ahead(&mut self, ahead: Ahead) {
        if self.disposing {
            return;
        }
        match ahead {
            Ahead::Ended if !self.prepares_ahead => return,
            Ahead::Ended => {
                self.ahead.get_or_insert(ahead);
            }
            Ahead::Opened | Ahead::Chosen => {
                self.prepares_ahead = true;
                self.ahead = Some(ahead);
            }
        }
        self.requests.wake_worker = true;
    }

    /// What was asked to ready ahead, taken once the session is idle with
    /// its runtime in its slot and nothing waits to run.
    pub(super) fn take_ahead(&mut self) -> Option<Ahead> {
        if self.idle_with_runtime() { self.ahead.take() } else { None }
    }

    /// Whether the session is idle, with nothing waiting to run and its
    /// runtime in its slot.
    pub(super) fn idle_with_runtime(&self) -> bool {
        matches!(self.activity, Activity::Idle)
            && self.pending.is_empty()
            && self.provider.held().is_some()
    }

    /// The request the next one is but for its input: the history as the
    /// selection the next request lands with replays it, under
    /// `system_prompt`. No turn has opened it yet.
    pub(super) fn ahead_request(
        &self,
        view: &ModelView,
        system_prompt: String,
        request_id: String,
    ) -> InferenceRequest {
        let (model, limits) = self
            .switch_target()
            .unwrap_or_else(|| (self.model.clone(), self.request_limits()));
        let replayed = replay(&RequestView::new(view, &model.model, limits, &self.tools));
        let blocks = replayed
            .blocks
            .into_iter()
            .map(|block| RequestBlock {
                items: block.items,
                entries: block.entries,
            })
            .collect::<Vec<_>>();
        InferenceRequest {
            session_id: self.id.to_string(),
            affinity: self.runtime.affinity(),
            turn_id: String::new(),
            request_id,
            model_id: model.model.id.clone(),
            output_limit: model.model.output_limit.and_then(NonZeroU32::new),
            output_cap: None,
            system_prompt,
            items: replayed.items.into(),
            blocks: blocks.into(),
            tools: self.tools.clone(),
            thinking: model.thinking.clone(),
            service_tier_id: model.service_tier_id.clone(),
            prompt_cache: PromptCache::Session {
                answered_items: replayed.answered,
            },
            cancel: CancellationToken::new(),
            account: self.account.clone(),
            purpose: RequestPurpose::Turn,
            agent: self.runtime.agent(),
        }
    }

    /// Whether the runtime the next request lands on readies anything ahead.
    pub(super) fn landing_prepares_ahead(&self) -> bool {
        self.switch
            .as_ref()
            .and_then(|switch| switch.runtime.as_deref())
            .or(self.provider.held())
            .is_some_and(|runtime| runtime.prepares_ahead())
    }

    /// Takes out the runtime the next request lands on, to ready it: the
    /// recorded switch's own when it has one, else the one in the slot.
    pub(super) fn take_landing_runtime(&mut self) -> Option<Box<dyn ProviderRuntime>> {
        if let Some(runtime) = self.switch.as_mut().and_then(|switch| switch.runtime.take()) {
            return Some(runtime);
        }
        self.take_runtime()
    }

    /// Puts back a runtime [`take_landing_runtime`](Self::take_landing_runtime)
    /// took, to land with the switch it came from when that switch is still the
    /// recorded one.
    ///
    /// One whose switch was replaced meanwhile serves nothing any more and is
    /// returned for closing, as is every runtime once dispose began.
    pub(super) fn return_landing_runtime(
        &mut self,
        runtime: Box<dyn ProviderRuntime>,
        from_switch: bool,
    ) -> Option<Box<dyn ProviderRuntime>> {
        if self.disposing {
            return Some(runtime);
        }
        if !from_switch {
            return self.place_runtime(runtime);
        }
        match self.switch.as_mut() {
            Some(switch) if switch.runtime.is_none() => {
                switch.runtime = Some(runtime);
                None
            }
            _ => Some(runtime),
        }
    }

    /// What the current model's vendor takes in one request
    /// (`crates/provider-common/src/contract.rs`): from the runtime in its slot, or as
    /// it was read when a run took the runtime.
    pub(super) fn request_limits(&self) -> RequestLimits {
        match &self.provider {
            ProviderSlot::Held(runtime) => runtime.request_limits(&self.model.model),
            ProviderSlot::Out(limits) => *limits,
        }
    }

    /// Takes the runtime out of its slot, for a run's request or for
    /// closing, and leaves what it takes in one request of the current
    /// model; none when it is out already.
    pub(super) fn take_runtime(&mut self) -> Option<Box<dyn ProviderRuntime>> {
        let limits = self.request_limits();
        match mem::replace(&mut self.provider, ProviderSlot::Out(limits)) {
            ProviderSlot::Held(runtime) => Some(runtime),
            ProviderSlot::Out(_) => None,
        }
    }

    /// Puts `runtime` in the slot, and returns the runtime it replaced.
    fn place_runtime(&mut self, runtime: Box<dyn ProviderRuntime>) -> Option<Box<dyn ProviderRuntime>> {
        match mem::replace(&mut self.provider, ProviderSlot::Held(runtime)) {
            ProviderSlot::Held(replaced) => Some(replaced),
            ProviderSlot::Out(_) => None,
        }
    }

    /// A request of the current model over `view`, the model's view of the
    /// replayed blocks: what replay sends it and what the estimates weigh.
    pub(super) fn request_view<'a>(&'a self, view: &'a ModelView) -> RequestView<'a> {
        RequestView::new(view, &self.model.model, self.request_limits(), &self.tools)
    }

    // Media.

    /// The blocks replay sends: from the last compaction boundary on.
    pub(super) fn replayed(&self) -> &[Block] {
        self.transcript.from(self.transcript.replay_start())
    }

    /// The blobs the replayed blocks and the waiting input reference: the
    /// queued messages, the pending steers and the message of a send whose
    /// `user` block is not written yet, such as while a switch lands first.
    fn referenced_media(&self) -> HashSet<BlobRef> {
        let mut referenced: HashSet<BlobRef> = self
            .replayed()
            .iter()
            .flat_map(media::references)
            .cloned()
            .collect();
        for action in &self.pending {
            if let ActionKind::Send { content, comments, .. } = &action.kind {
                referenced.extend(media::content_references(content).cloned());
                referenced.extend(media::comment_references(comments).cloned());
            }
        }
        for steer in self.inputs.pending_steers() {
            referenced.extend(media::content_references(&steer.content).cloned());
            referenced.extend(media::comment_references(&steer.comments).cloned());
        }
        if let Activity::Running(ActionRun { unwritten: Some(message), .. }) = &self.activity {
            referenced.extend(media::content_references(&message.content).cloned());
            referenced.extend(media::comment_references(&message.comments).cloned());
        }
        referenced
    }

    /// Lets go of the media nothing references any more.
    pub(super) fn release_media(&mut self) {
        let referenced = self.referenced_media();
        self.media.retain(&referenced);
    }

    /// The replayed blocks as the model receives them, with what the session
    /// holds for their media; or, while it holds nothing for some of them,
    /// their blobs.
    pub(super) fn model_view(&self) -> Result<ModelView, Vec<BlobRef>> {
        ModelView::of(self.transcript.replay_start(), self.replayed(), &self.media)
    }

    /// The estimate of the next request of the current model, or the blobs to
    /// read first.
    ///
    /// Only the blocks after the anchor weigh, so only their media need to be
    /// held. A session restored with media, which holds none, tells it without
    /// a read while those blocks reference none.
    pub(super) fn context_estimate(&self, system_prompt: &str) -> Result<u64, Vec<BlobRef>> {
        let start = self.transcript.replay_start();
        let replayed = self.replayed();
        let anchor = context_anchor(replayed, &self.model.model);
        let weighed = ModelView::of(start + anchor.from, &replayed[anchor.from..], &self.media)?;
        Ok(anchor_tokens(&anchor, system_prompt, &self.tools)
            + blocks_tokens(&weighed.blocks, &self.request_view(&weighed)))
    }

    /// Drains the transcript's patches into one event and marks their rows
    /// for the next save.
    pub(super) fn commit(&mut self) {
        let Some(batch) = self.transcript.take_patches() else {
            return;
        };
        self.persist.rows.merge(batch.rows);
        self.persist.dirty = true;
        self.requests.save = true;
        self.outbox.push(SessionEvent::TranscriptChanged {
            patches: batch.patches,
            version: self.transcript.version(),
        });
    }

    // History rewrites.

    /// The save of a rewritten history.
    ///
    /// It writes:
    ///
    /// - as new rows, which `version` makes a version the user sees, the path
    ///   from the first block that differs, or from a block added before it and
    ///   not saved yet;
    /// - the rows changed in place before it;
    /// - the state row.
    ///
    /// `blocks` is the path from the window's start on.
    pub(super) fn rewrite_update(&self, blocks: &[Block], version: bool) -> CheckpointUpdate {
        let start = self.transcript.start();
        let kept = start + kept(self.transcript.blocks(), blocks);
        let from = self.persist.rows.floor().map_or(kept, |floor| floor.min(kept));
        let length = start + blocks.len();
        let changed_blocks = self
            .persist
            .rows
            .indices(from)
            .into_iter()
            .chain(from..length)
            .map(|index| (index, blocks[index - start].clone()))
            .collect();
        CheckpointUpdate {
            state: self.checkpoint_state(),
            changed_blocks,
            block_count: length,
            moved_from: Some(from),
            version,
        }
    }

    /// Adopts a rewritten history its save made durable: the transcript is
    /// replaced and published as one `replace` patch, and the rows the save
    /// wrote are current.
    pub(super) fn adopt_rewrite(&mut self, blocks: Vec<Block>, version: bool) {
        let batch = self.transcript.replace_all(blocks, version);
        self.persist.rows = Default::default();
        self.outbox.push(SessionEvent::TranscriptChanged {
            patches: batch.patches,
            version: self.transcript.version(),
        });
    }

    // Saving.

    /// The next save, when one is due: the changed rows and the state row,
    /// with the rows it took, for putting back when it fails.
    pub(super) fn prepare_checkpoint(&mut self) -> Option<(CheckpointUpdate, DirtyRows)> {
        self.commit();
        if !self.persist.dirty {
            return None;
        }
        self.persist.dirty = false;
        let rows = mem::take(&mut self.persist.rows);
        let transcript = &self.transcript;
        let changed_blocks = rows
            .indices(transcript.len())
            .into_iter()
            .map(|index| {
                let block = transcript.get(index).expect("the window holds every row a save writes");
                (index, block.clone())
            })
            .collect();
        let state = self.checkpoint_state();
        self.persist.recorded = state.phase;
        let update = CheckpointUpdate {
            state,
            changed_blocks,
            block_count: transcript.len(),
            moved_from: rows.floor(),
            version: false,
        };
        Some((update, rows))
    }

    /// Lets go of the blocks before the last `compaction_boundary` once every
    /// row up to it is saved: after a save commits, and when nothing was left
    /// to save.
    ///
    /// Blocks a rewrite or an edit read from before the window go the same way.
    pub(super) fn release_settled(&mut self) {
        self.commit();
        let boundary = self.transcript.replay_start();
        if self.persist.rows.lowest().is_some_and(|lowest| lowest < boundary) {
            return;
        }
        self.transcript.release_before(boundary);
    }

    /// Puts back the rows a failed save took.
    pub(super) fn restore_marks(&mut self, rows: DirtyRows) {
        self.persist.dirty = true;
        self.persist.rows.merge(rows);
    }

    pub(super) fn checkpoint_state(&self) -> CheckpointState {
        CheckpointState {
            phase: self.recorded_phase(),
            queue: self.queued_messages(),
            agent_inputs: self.inputs.agent_inputs(),
            reports: self.inputs.reports(Opens::Turn),
            next_turn_reports: self.inputs.reports(Opens::Nothing),
            watches: self.watched.watches(),
            model: self.model.clone(),
            edits: self.edits.clone(),
            account: self.account.clone(),
            turn_speed: self.turn_speed,
        }
    }

    fn agent_input_ids(&self) -> Vec<BlockId> {
        self.inputs
            .agent_inputs()
            .into_iter()
            .map(|input| input.message.id)
            .collect()
    }

    // What clients see, derived from the one status.

    /// The phase clients see.
    ///
    /// An action is running until its closing save has committed, and the next
    /// waiting action starts without the session going idle between them, so a
    /// client that sees `idle` can edit at once.
    pub(super) fn phase(&self) -> SessionPhase {
        match &self.activity {
            Activity::Running(ActionRun { idle_compaction: true, .. })
            | Activity::Finishing { idle_compaction: true } => SessionPhase::CompactingIdle,
            Activity::Running(run) if run.stage == TurnStage::Compacting => {
                SessionPhase::Compacting
            }
            Activity::Running(_) | Activity::Finishing { .. } => SessionPhase::Running,
            Activity::Closed { interrupted: true } => SessionPhase::Running,
            Activity::Idle | Activity::Closed { interrupted: false } => SessionPhase::Idle,
        }
    }

    /// The phase a checkpoint records. The closing save of an action records
    /// the session idle, so a checkpoint that says an action was running is
    /// one the process died in. A compaction while idle is recorded idle
    /// throughout: its pass writes its boundary and marker in one save, so a
    /// process that dies in it leaves no turn unfinished.
    fn recorded_phase(&self) -> SessionPhase {
        match (&self.activity, self.phase()) {
            (Activity::Finishing { .. }, _) | (_, SessionPhase::CompactingIdle) => SessionPhase::Idle,
            (_, phase) => phase,
        }
    }

    pub(super) fn execution(&self) -> Execution {
        match &self.activity {
            Activity::Running(run) => match run.stage {
                TurnStage::Preparing | TurnStage::Streaming => Execution::ProviderStreaming,
                TurnStage::Tools => Execution::ToolExecuting,
                TurnStage::Compacting => Execution::Compacting,
            },
            Activity::Finishing { .. } => Execution::Finalizing,
            Activity::Idle | Activity::Closed { .. } if !self.watched.is_empty() => {
                Execution::Waiting
            }
            Activity::Idle | Activity::Closed { .. } => Execution::Idle,
        }
    }

    pub(super) fn settle(&self) -> Settle {
        match &self.activity {
            Activity::Closed { .. } => Settle::Closed,
            Activity::Idle if self.pending.is_empty() => Settle::Settled,
            _ => Settle::Busy,
        }
    }

    pub(super) fn status(&self) -> Status {
        Status {
            settle: self.settle(),
            input: self.inputs.has_agent_input() || self.inputs.has_report(),
            commands: !self.watched.is_empty(),
            awaited: self.watched.awaits(),
        }
    }

    pub(super) fn pending_steers(&self) -> Vec<PendingSteer> {
        self.inputs.pending_steers()
    }

    /// The message the running turn started with, until its `user` block is
    /// written.
    pub(super) fn starting(&self) -> Option<QueuedMessage> {
        match &self.activity {
            Activity::Running(run) => run.unwritten.clone(),
            _ => None,
        }
    }

    /// Ends a change: the events it made, then the queue, the pending steers
    /// and the phase when they changed.
    ///
    /// A change of the queue, the waiting agent messages and command reports,
    /// the commands that report or the edit receipts is also due for a save.
    pub(super) fn take_effects(&mut self) -> Effects {
        let mut events = mem::take(&mut self.outbox);
        let queue: Vec<TurnId> = self
            .pending
            .iter()
            .filter(|action| action.is_send())
            .map(|action| action.turn.clone())
            .collect();
        if queue != self.published.queue {
            self.published.queue = queue;
            self.mark_state_changed();
            events.push(SessionEvent::QueueChanged {
                queue: self.queued_messages(),
            });
        }
        let agent_inputs = self.agent_input_ids();
        if agent_inputs != self.published.agent_inputs {
            self.published.agent_inputs = agent_inputs;
            self.mark_state_changed();
        }
        let reports = self.inputs.reports(Opens::Turn);
        if reports != self.published.reports {
            self.published.reports = reports;
            self.mark_state_changed();
        }
        let next_turn_reports = self.inputs.reports(Opens::Nothing);
        if next_turn_reports != self.published.next_turn_reports {
            self.published.next_turn_reports = next_turn_reports;
            self.mark_state_changed();
        }
        let watches = self.watched.watches();
        if watches != self.published.watches {
            self.published.watches = watches;
            self.mark_state_changed();
        }
        if self.edits.len() != self.published.edits {
            self.published.edits = self.edits.len();
            self.mark_state_changed();
        }

        let pending_steers = self.pending_steers();
        if pending_steers != self.published.pending_steers {
            self.published.pending_steers = pending_steers.clone();
            events.push(SessionEvent::PendingSteersChanged { pending_steers });
        }
        let starting = self.starting();
        if starting != self.published.starting {
            self.published.starting = starting.clone();
            events.push(SessionEvent::StartingChanged { starting });
        }
        let pending_calls = self.pending_calls();
        if pending_calls != self.published.pending_calls {
            self.published.pending_calls = pending_calls.clone();
            events.push(SessionEvent::PendingCallsChanged { pending_calls });
        }
        let execution = self.execution();
        if execution != self.published.execution {
            let ms_in_from = self.published.execution_at.map(|at| self.ms_since(at));
            session_log!(
                self,
                info,
                event = "node.state_changed",
                node = %self.id,
                turn = self.running_turn().map(tracing::field::display),
                from = self.published.execution.as_str(),
                to = execution.as_str(),
                cause = if self.state_cause.is_empty() { "state_changed" } else { self.state_cause },
                ms_in_from,
                "the node changed what it does"
            );
            self.published.execution = execution;
            self.published.execution_at = Some(self.clock.now());
            self.state_cause = "";
        }
        let phase = self.phase();
        if phase != self.published.phase {
            self.published.phase = phase;
            events.push(SessionEvent::PhaseChanged { phase });
        }
        if self.waiting_since != self.published.waiting_since {
            self.published.waiting_since = self.waiting_since;
            events.push(SessionEvent::WaitingChanged {
                waited_ms: self.waited_ms(),
            });
        }
        if self.output != self.published.output {
            self.published.output = self.output.clone();
            events.push(SessionEvent::OutputChanged {
                output: self.request_output(),
            });
        }
        let requests = mem::take(&mut self.requests);
        Effects {
            events,
            wake_worker: requests.wake_worker,
            save: requests.save,
            status: self.status(),
        }
    }

    fn mark_state_changed(&mut self) {
        self.persist.dirty = true;
        self.requests.save = true;
    }
}

/// The `description` of a call's input written so far, once its string is
/// whole: an unfinished document is read as far as it goes, and a string
/// still being written counts as absent.
fn written_description(input: &str) -> Option<String> {
    let value = jiter::JsonValue::parse_with_config(input.as_bytes(), false, jiter::PartialMode::On).ok()?;
    let jiter::JsonValue::Object(fields) = value else {
        return None;
    };
    fields.iter().find_map(|(key, value)| match (key.as_ref(), value) {
        ("description", jiter::JsonValue::Str(text)) if !text.trim().is_empty() => {
            Some(text.trim().to_owned())
        }
        _ => None,
    })
}

/// A provider failure's diagnostics as the session records them: with the
/// attempt's client request id, and the source `unknown` when the provider
/// named none.
pub(super) fn with_request_id(mut failure: ProviderFailure, request_id: &str) -> ProviderFailure {
    let diagnostics = failure
        .diagnostics
        .get_or_insert(Box::new(ProviderErrorDiagnostics {
            source: FailureSource::Unknown,
            client_request_id: None,
            provider_request_id: None,
            provider_response_id: None,
            provider_code: None,
            http_status: None,
            upstream: None,
        }));
    diagnostics.client_request_id = Some(request_id.to_owned());
    failure
}

/// Puts `switch` in `slot` in place of the switch there.
///
/// A switch within the replaced switch's provider brings no runtime and lands
/// on the replaced one's. One that brings its own retires the replaced one's,
/// which closes when a switch lands.
fn replace_switch(
    slot: &mut Option<ModelSwitch>,
    mut switch: ModelSwitch,
    retired: &mut Vec<Box<dyn ProviderRuntime>>,
) {
    if let Some(replaced) = slot.take() {
        if switch.runtime.is_none() {
            switch.runtime = replaced.runtime;
        } else {
            retired.extend(replaced.runtime);
        }
    }
    *slot = Some(switch);
}

/// Whether `report` tells an end a look showed the node since the report was
/// made.
///
/// A look is a `demi shell status` or a `shell` result. Such an end tells
/// nothing more
/// (`crates/agent-session/src/reports.rs`).
///
/// A report never marks its own end seen, so a mark is always a look's.
/// Admission and delivery both ask here, since a look that runs while the
/// report is made shows the end after it.
///
/// An end whose job viewed media is never stale: the report carries them, which
/// no look attaches, without the output the look showed.
fn end_seen(runtime: &dyn SessionRuntime, report: &CommandReport) -> bool {
    is_end(report) && report.media.is_empty() && runtime.end_seen(&report.command_id)
}

/// What a report tells, as the log names it.
fn report_kind(report: &CommandReport) -> &'static str {
    match report.event {
        ReportEvent::Running { .. } => "silence",
        ReportEvent::Ended { .. } => "end",
        ReportEvent::Stopped { .. } => "stopped",
        ReportEvent::Lost { .. } => "lost",
    }
}

/// Whether `report` tells its command's end, not its progress.
fn is_end(report: &CommandReport) -> bool {
    !matches!(report.event, ReportEvent::Running { .. })
}
