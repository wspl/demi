//! One agent's session: a cloneable handle over the session's state.
//!
//! One worker task changes the state by running one action at a time, while
//! commands from the connection change it between the worker's awaits. The
//! worker, the persister, the tasks that watch the commands that report to the
//! session and every command change the state only through
//! [`SessionShared::update`].
//!
//! The closure it takes is synchronous and ends before any await. Each
//! change's events reach the listeners after the state is released.
//!
//! ```text
//!   connection
//!      |  actions, steers, abort          ^  SessionEvent, once a change
//!      v                                  |  is complete
//!   AgentSession (worker, persister) -----'
//!      |
//!      +--> SessionRuntime (the node: prompts, context, tools, admission)
//!      +--> ProviderRuntime (one request at a time)
//!      '--> SessionStore (checkpoint saves, blobs)
//! ```
//!
//! The session knows no product and no plugin. Its node's [`SessionRuntime`]
//! answers what it needs, and a tool never reaches into the session.
//!
//! A session runs a tool only through the tool-call contract it defines
//! (`ToolInvocation`, `ToolOutcome`, `ToolEffect`, `ToolFailure`), which the
//! tools implement.
//!
//! # Status and queue
//!
//! The phase, the visible queue and whether the session has settled are derived
//! from one status and the waiting actions; no second flag records them.
//!
//! - The session is idle only once the save that ends an action has committed
//!   and no other action starts. A client that sees `idle` can edit at once,
//!   and what it then reads from the backend includes the finished turn.
//! - A send is never refused for being busy: it waits in the queue, and its
//!   message id makes a repeated send harmless.
//! - `retry`, `resume` and `compact` need an idle session.
//! - While an edit is prepared, the session refuses sends, steers, agent
//!   messages and command reports, and a model switch waits for it, so the edit
//!   never discards one.
//!
//! # Cancellation
//!
//! A turn races provider streams, tools, backoff sleeps and admission waits
//! against its cancellation token and drops them. It never races a store save.
//!
//! Work that must outlive its waiter is owned by a task of the session, never
//! by the waiting future.

mod ahead;
mod breakdown;
mod bus;
mod cancel;
mod compaction;
mod core;
mod editing;
mod input;
mod media;
mod persist;
mod reports;
mod retry;
mod runtime;
mod turn;
mod window;
mod worker;

/// What a session's tests compare its requests with (feature `testing`).
#[cfg(feature = "testing")]
pub mod testing {
    pub use crate::compaction::COMPACTION_SUMMARY_INSTRUCTION;
}

use tracing::Instrument as _;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    future::Future,
    pin::Pin,
    rc::{Rc, Weak},
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use demi_agent_store::{
    Checkpoint, CheckpointUpdate, Kind, RunningCommand, SessionStore, StoreError, Watch, media::HeldMedia,
};
use demi_agent_transcript::{
    Earlier, ForkEnd, IdSource, TranscriptLog, last_answer, last_failure,
};
use demi_conversation_socket_protocol::{AbortResult, AgentCode, Refusal, RequestOutput, TranscriptPatch, Version};
use demi_provider_common::{Ahead, ProviderFailure, ProviderRuntime};
use demi_shared_gates::SerialGate;
use demi_shared_types::{
    MessageComment,
    AgentMessage, AutomationMark, Block, BlockId, Clock, CommandId, ContextUsage, ErrorBlock, ModelSelection, NodeId, PendingCall,
    PendingSteer, PictureCopy, ProviderErrorDiagnostics, QueuedMessage, SessionPhase, ShellToolView, ShellViewStatus,
    TextBlock, Timestamp, ToolResultContentBlock, ToolView, TurnId, UserContentBlock,
};
use schemars::JsonSchema;
use serde::Serialize;
use tokio::sync::{Notify, oneshot, watch};
use tokio_util::task::AbortOnDropHandle;

pub use compaction::{CompactionConfig, compaction_refusal, token_threshold};
pub use editing::ForkError;
pub use editing::{
    EditCheck, EditContent, EditError, EditSubmission, ForkSeed, accepted, edit_digest, fork_seed, regenerate_digest,
};
pub use retry::RetryPolicy;
pub use window::{Reset, checkpoint_reset, latest_page};
pub use runtime::{
    CallCommand, CommandLook, InputArrival, NewContext, PromptLayer, RunningWork, SeenContext, SessionRuntime, StartedCommand, SystemPrompt,
    StepOutcomes, ToolEffect, ToolFailure, ToolInvocation, ToolOutcome, WindowEnd,
};

use self::{
    bus::EventBus,
    cancel::TurnCancel,
    core::{AbortStep, ActionKind, CoreParts, ProviderSlot, SessionCore},
    input::InputQueue,
    reports::Watched,
};

/// How long a command that is not in the background prints nothing before it
/// wakes its node, once (`crates/agent-session/src/reports.rs`).
pub const SILENCE_LIMIT: Duration = Duration::from_secs(600);

/// How a session saves, retries and compacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// How long after the first unsaved change a save starts.
    pub persist_interval: Duration,
    /// How long a command that is not in the background prints nothing before
    /// it wakes its node, once (`crates/agent-session/src/reports.rs`).
    pub silence: Duration,
    pub retry: RetryPolicy,
    pub compaction: CompactionConfig,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            persist_interval: Duration::from_secs(1),
            silence: SILENCE_LIMIT,
            retry: RetryPolicy::default(),
            compaction: CompactionConfig::default(),
        }
    }
}

/// What a session is built with besides its state: its node's runtime, its
/// checkpoint store, and where identities and times come from.
pub struct SessionDeps {
    pub runtime: Rc<dyn SessionRuntime>,
    pub store: Rc<dyn SessionStore>,
    pub ids: Rc<dyn IdSource>,
    pub clock: Arc<dyn Clock>,
    pub config: SessionConfig,
}

/// A new session's state.
pub struct SessionInit {
    pub id: NodeId,
    pub model: ModelSelection,
    pub runtime: Box<dyn ProviderRuntime>,
}

/// What a restored session hands back for its node to decide on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Continuation {
    /// The checkpoint says a turn was running: the process died in it, or
    /// dispose stopped it.
    pub interrupted: bool,
    /// The messages queued in the checkpoint, in order.
    pub queued: Vec<QueuedMessage>,
}

/// A change of the model selection, with the runtime it needs when its
/// model belongs to another provider.
pub struct ModelSwitch {
    pub model: Box<ModelSelection>,
    /// A new runtime when the model belongs to another provider than the
    /// selection before it.
    pub runtime: Option<Box<dyn ProviderRuntime>>,
}

impl std::fmt::Debug for ModelSwitch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModelSwitch")
            .field("model", &self.model)
            .field("runtime", &self.runtime.is_some())
            .finish()
    }
}

impl ModelSwitch {
    /// Closes the runtime of a switch that no session took.
    pub async fn discard(self) {
        if let Some(mut runtime) = self.runtime {
            runtime.close().await;
        }
    }
}

/// The transcript at one moment, with its version: its blocks from the
/// position `start` on.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptSnapshot {
    pub start: usize,
    pub blocks: Vec<Block>,
    pub version: Version,
    /// The versions at each place that holds more than one.
    pub versions: std::collections::BTreeMap<usize, demi_conversation_socket_protocol::Versions>,
}

/// How an action ended when it did not fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionEnd {
    Completed,
    /// It was stopped; one that had written into the transcript recorded
    /// the stop.
    Aborted,
    /// It left the queue without running: dequeued, cleared, stopped or
    /// turned into a steer.
    Dropped,
    /// Dispose kept it queued in the final checkpoint.
    Detached,
    /// A message whose id the session already knew.
    Duplicate,
}

pub(crate) type ActionResult = Result<ActionEnd, Box<ErrorReport>>;

type ActionReply = oneshot::Sender<ActionResult>;

/// Resolves when an admitted action ends. Dropping it does nothing.
pub struct ActionHandle(oneshot::Receiver<ActionResult>);

impl ActionHandle {
    fn channel() -> (ActionReply, Self) {
        let (reply, receiver) = oneshot::channel();
        (reply, Self(receiver))
    }

    fn ended(end: ActionEnd) -> Self {
        let (reply, handle) = Self::channel();
        // The receiver is in hand, so the send cannot fail.
        let _ = reply.send(Ok(end));
        handle
    }
}

impl Future for ActionHandle {
    type Output = ActionResult;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<ActionResult> {
        // A reply dropped unanswered belongs to a session that went away.
        Pin::new(&mut self.0)
            .poll(context)
            .map(|answer| answer.unwrap_or(Ok(ActionEnd::Detached)))
    }
}

/// Why a session refused an action or a change.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionError {
    #[error("The agent session is closed")]
    Closed,
    #[error("A message edit is being prepared")]
    Editing,
}

impl AdmissionError {
    /// The refusal as the page words it.
    pub fn refusal(&self) -> Refusal {
        let code = match self {
            Self::Closed => AgentCode::SessionClosed,
            Self::Editing => AgentCode::EditInProgress,
        };
        Refusal::new(code, self.to_string())
    }
}

/// Why a steer was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SteerError {
    #[error("No turn is running to steer")]
    NotRunning,
    #[error("The running turn is being stopped")]
    Stopped,
    #[error("The running turn is finishing")]
    Finishing,
    #[error("A message edit is being prepared")]
    Editing,
}

impl SteerError {
    /// The refusal as the page words it.
    pub fn refusal(&self) -> Refusal {
        let code = match self {
            Self::NotRunning => AgentCode::NothingToSteer,
            Self::Stopped => AgentCode::TurnStopping,
            Self::Finishing => AgentCode::TurnFinishing,
            Self::Editing => AgentCode::EditInProgress,
        };
        Refusal::new(code, self.to_string())
    }
}

/// Why an agent message was refused (`crates/agent-session/src/input.rs`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentMessageError {
    #[error("Agent message recipient does not match this session")]
    Recipient,
    #[error("Agent message id already belongs to different content")]
    DifferentContent,
    #[error("Agent message id conflicts with another input")]
    Conflict,
    #[error("The agent message is invalid: {0}")]
    Invalid(String),
    #[error("The agent session is closed")]
    Closed,
    #[error("A message edit is being prepared")]
    Editing,
    /// The admission is not durable: its save failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Why a checkpoint could not be restored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RestoreError {
    #[error("The checkpoint's waiting input is invalid: {0}")]
    Input(String),
    #[error("The checkpoint's edit receipts repeat operation {0}")]
    Edits(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// A failure as clients see it: the frame an `error` event becomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorReport {
    pub message: String,
    pub code: Option<String>,
    pub diagnostics: Option<ProviderErrorDiagnostics>,
}

impl From<&ProviderFailure> for ErrorReport {
    fn from(failure: &ProviderFailure) -> Self {
        Self {
            message: failure.message.clone(),
            code: failure.code.as_ref().map(|code| code.as_str().to_owned()),
            diagnostics: failure.diagnostics.as_deref().cloned(),
        }
    }
}

impl From<StoreError> for ErrorReport {
    fn from(error: StoreError) -> Self {
        Self::agent(AgentCode::SaveFailed, error.to_string())
    }
}

impl ErrorReport {
    /// A failure of the agent's own, which `code` names to the page and
    /// `message` says in English.
    pub fn agent(code: AgentCode, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: Some(code.to_string()),
            diagnostics: None,
        }
    }
}

/// A change of a session, reported once it is complete.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    TranscriptChanged {
        patches: Vec<TranscriptPatch>,
        revision: u64,
    },
    PhaseChanged {
        phase: SessionPhase,
    },
    QueueChanged {
        queue: Vec<QueuedMessage>,
    },
    /// The estimate of the next request after a response, a compaction
    /// pass or an action.
    ContextUsageChanged {
        usage: ContextUsage,
    },
    /// The human steers waiting for a boundary changed.
    PendingSteersChanged {
        pending_steers: Vec<PendingSteer>,
    },
    /// The message the running turn started with, which it has not written
    /// yet, changed: none once its `user` block is written or the turn ends.
    StartingChanged {
        starting: Option<QueuedMessage>,
    },
    /// The calls the model is writing changed.
    PendingCallsChanged {
        pending_calls: Vec<PendingCall>,
    },
    /// The node began to wait for the model's answer, with how long it has
    /// waited, or its wait ended.
    WaitingChanged {
        waited_ms: Option<u64>,
    },
    /// The output of the turn's latest request changed: it started, it
    /// streamed more, it ended, or the turn ended, which leaves none.
    ///
    /// A hold of the turn's end never holds it (`bus`): the response's own
    /// count goes out when the response ends.
    OutputChanged {
        output: Option<RequestOutput>,
    },
    /// A transient provider failure is retried after `delay_ms`.
    RetryScheduled {
        attempt: u32,
        delay_ms: u64,
        code: Option<String>,
        diagnostics: Option<ProviderErrorDiagnostics>,
    },
    /// A failed turn or a failed save.
    Error {
        report: ErrorReport,
    },
    /// An action failed for good, after its checkpoint was saved.
    ActionFailed {
        report: ErrorReport,
    },
}

/// Why an action's body ended early.
pub(crate) enum TurnError {
    /// The action was stopped: by the user, or by dispose.
    Cancelled,
    /// The action failed: a provider's run, a save it waited for, or what it
    /// was asked to do, such as a retry without an input turn.
    Failed(Box<ErrorReport>),
}

impl TurnError {
    /// The action cannot do what it was asked, for `code`.
    pub(crate) fn refused(code: AgentCode, message: impl Into<String>) -> Self {
        Self::Failed(Box::new(ErrorReport::agent(code, message)))
    }
}

impl From<StoreError> for TurnError {
    fn from(error: StoreError) -> Self {
        Self::Failed(Box::new(error.into()))
    }
}

/// Whether the session will do anything more by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// An action runs or waits.
    Busy,
    Settled,
    /// Disposed: nothing will ever run again.
    Closed,
}

/// What the tree watches a session for: whether it acts, and what could
/// make it act later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub settle: Settle,
    /// An agent message or a command report waits for a boundary.
    pub input: bool,
    /// A command the session's calls left running still runs, and will
    /// report to it (`crates/agent-session/src/reports.rs`).
    pub commands: bool,
}

/// What a session is doing, as a supervisor observes it
/// (`crates/agent-server/src/server/tree/supervisor.rs`); not the phase that
/// `phase` frames carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Execution {
    Idle,
    ProviderStreaming,
    ToolExecuting,
    Compacting,
    /// The action ended and its checkpoint is being saved.
    Finalizing,
    /// Idle while commands it started, or its children, still run.
    Waiting,
}

impl Execution {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::ProviderStreaming => "provider_streaming",
            Self::ToolExecuting => "tool_executing",
            Self::Compacting => "compacting",
            Self::Finalizing => "finalizing",
            Self::Waiting => "waiting",
        }
    }
}

/// Ends a listener's subscription when dropped.
pub struct Subscription {
    session: Weak<SessionShared>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(session) = self.session.upgrade() {
            session.bus.unsubscribe(self.id);
        }
    }
}

pub(crate) struct SessionShared {
    core: RefCell<SessionCore>,
    bus: EventBus,
    /// The one order of saves, history rewrites and edit commits: a save that
    /// started finishes before the next one starts.
    persist_gate: SerialGate,
    status: watch::Sender<Status>,
    /// What ends a shell call's window early: one more agent message or
    /// command report, or the user's send now (`crates/agent-tools/src/lib.rs`).
    arrivals: watch::Sender<runtime::Arrivals>,
    /// Wakes the worker when an action starts.
    work: Rc<Notify>,
    /// Wakes the persister when a change makes a save due.
    persist_wake: Rc<Notify>,
    /// Tells a waiting fork that the runtime is back in its slot.
    runtime_returned: Notify,
    /// Whether a dispose saved, or is saving, the final checkpoint: only the
    /// first does.
    finalized: Cell<bool>,
    runtime: Rc<dyn SessionRuntime>,
    store: Rc<dyn SessionStore>,
    ids: Rc<dyn IdSource>,
    config: SessionConfig,
    worker: RefCell<Option<AbortOnDropHandle<()>>>,
    persister: RefCell<Option<AbortOnDropHandle<()>>>,
}

impl SessionShared {
    /// Changes the state in one synchronous step, then does what the change
    /// asked for: wakes the worker or the persister, publishes the
    /// session's status, and delivers the change's events.
    fn update<R>(&self, change: impl FnOnce(&mut SessionCore) -> R) -> R {
        let (result, effects, arrivals) = {
            let mut core = self.core.borrow_mut();
            let result = change(&mut core);
            (result, core.take_effects(), core.window_arrivals())
        };
        self.arrivals.send_if_modified(|published| {
            let changed = *published != arrivals;
            *published = arrivals;
            changed
        });
        if effects.wake_worker {
            self.work.notify_one();
        }
        if effects.save {
            self.persist_wake.notify_one();
        }
        self.status.send_if_modified(|status| {
            let changed = *status != effects.status;
            *status = effects.status;
            changed
        });
        self.bus.deliver(effects.events);
        result
    }

    fn read<R>(&self, look: impl FnOnce(&SessionCore) -> R) -> R {
        look(&self.core.borrow())
    }

    fn emit(&self, event: SessionEvent) {
        self.update(|core| core.outbox.push(event));
    }

    /// Holds the events of the changes from now on until `release_events`.
    ///
    /// A response that requested no tool may have ended the turn. That end
    /// reaches clients as one change: the response's last text with the phase
    /// that goes idle after the closing save.
    fn hold_events(&self) {
        self.bus.hold();
    }

    /// Delivers the events held, then each change's as it completes.
    fn release_events(&self) {
        self.bus.release();
    }

    /// Puts a run's runtime back in its slot.
    fn return_runtime(&self, runtime: Box<dyn ProviderRuntime>) {
        self.update(|core| core.provider = ProviderSlot::Held(runtime));
        self.runtime_returned.notify_waiters();
    }
}

/// A handle over one agent's session; clones share it.
#[derive(Clone)]
pub struct AgentSession {
    shared: Rc<SessionShared>,
}

/// A session that is not kept alive by whoever holds it, such as a question
/// still out about one of its blocks.
#[derive(Clone)]
pub struct WeakSession(std::rc::Weak<SessionShared>);

impl WeakSession {
    pub fn upgrade(&self) -> Option<AgentSession> {
        self.0.upgrade().map(|shared| AgentSession { shared })
    }
}

impl AgentSession {
    pub fn downgrade(&self) -> WeakSession {
        WeakSession(Rc::downgrade(&self.shared))
    }

    /// Keeps `files` with the complete text `id` as which of its code spans
    /// name files on the conversation's Host
    /// (`crates/agent-server/src/node/reply_files.rs`): once, and only while
    /// the window holds the text.
    pub fn keep_code_files(&self, id: &BlockId, files: Vec<demi_shared_types::CodeFile>) {
        self.shared.update(|core| core.keep_code_files(id, files));
    }

    /// The texts of the window with a picture whose copy is still being made,
    /// as a node that stopped in the middle of copying finds them when it
    /// starts. Only those texts are copied out of the window.
    pub fn unsettled_pictures(&self) -> Vec<TextBlock> {
        self.shared.read(|core| {
            core.transcript
                .blocks()
                .iter()
                .filter_map(|block| match block {
                    Block::Text(text)
                        if text.pictures.iter().flatten().any(|picture| picture.copy == PictureCopy::Copying) =>
                    {
                        Some(text.clone())
                    }
                    _ => None,
                })
                .collect()
        })
    }

    /// Settles the picture the text `id` names by `target`, whose copy was
    /// being made, as `copy`.
    pub fn settle_picture(&self, id: &BlockId, target: &str, copy: PictureCopy) {
        self.shared.update(|core| core.settle_picture(id, target, copy));
    }

    /// A new session with an empty transcript; its node's first checkpoint is
    /// already in the store.
    pub fn create(init: SessionInit, deps: SessionDeps) -> Self {
        let transcript = TranscriptLog::new(Vec::new(), deps.ids.clone(), deps.clock.clone());
        let parts = CoreParts {
            id: init.id,
            model: init.model,
            provider: init.runtime,
            transcript,
            media: HeldMedia::default(),
            tools: deps.runtime.tools(),
            inputs: InputQueue::default(),
            watched: Watched::default(),
            edits: Vec::new(),
            account: None,
            turn_speed: None,
            one_request: false,
            request_turn: None,
            ids: deps.ids.clone(),
            clock: deps.clock.clone(),
            runtime: deps.runtime.clone(),
        };
        Self::start(SessionCore::new(parts), deps)
    }

    /// A session from its checkpoint.
    ///
    /// A tool call still marked executing completes as interrupted and never
    /// runs again. `running` is the node's commands that the conversation
    /// records as running. When it holds one of the call's commands, the call
    /// names that command, which then reports to the node as one a call left
    /// running does.
    ///
    /// The restored session:
    ///
    /// - is idle;
    /// - watches again the commands its calls left running;
    /// - hands back its queue and whether a turn was interrupted.
    ///
    /// Waiting input arrives only once the restored hold is in place, before
    /// any watch starts:
    ///
    /// - A root's interrupted turn ends with its interruption record, which
    ///   holds what arrives as any failed root's turn does.
    /// - A child is held until its supervisor looks at it or its next action
    ///   starts (`crates/agent-server/src/server/tree/supervisor.rs`).
    ///
    /// The session holds the checkpoint's window. Of a checkpoint that holds
    /// its whole path, such as a Fork's seed, it holds the blocks from its last
    /// `compaction_boundary` on. It reads what it keeps of the blocks before
    /// from the store.
    pub async fn restore(
        checkpoint: Checkpoint,
        id: NodeId,
        runtime: Box<dyn ProviderRuntime>,
        deps: SessionDeps,
        running: &[RunningCommand],
    ) -> Result<(Self, Continuation), RestoreError> {
        let Checkpoint {
            state,
            start,
            transcript,
            versions,
        } = checkpoint;
        check_restored_input(&id, &transcript, &state.agent_inputs)?;
        for input in &state.agent_inputs {
            if start > 0 && deps.store.position(&input.message.id).await?.is_some() {
                return Err(RestoreError::Input(format!("agent message {} is not unique", input.message.id)));
            }
        }
        let earlier = match start {
            0 => Earlier::default(),
            start => Earlier::read(&*deps.store, start).await?,
        };
        let mut operations = HashSet::new();
        if let Some(receipt) = state
            .edits
            .iter()
            .find(|receipt| !operations.insert(receipt.operation_id.as_str()))
        {
            return Err(RestoreError::Edits(receipt.operation_id.to_string()));
        }
        let mut transcript = TranscriptLog::new(transcript, deps.ids.clone(), deps.clock.clone())
            .with_versions(versions)
            .starting_at(start, earlier);
        // Every row it holds is stored.
        transcript.release_before(transcript.replay_start());
        let mut watched = Watched::default();
        for watch in &state.watches {
            watched.add(watch.command_id.clone(), watch.watch);
        }
        for call in transcript.pending_tool_calls() {
            let started = running
                .iter()
                .find(|running| running.tool_use_id == call.tool_use_id);
            let command = started.map(|running| running.command.clone());
            let text = core::interrupted_text(&core::CallEnd::Crashed, command.as_ref());
            // The call keeps naming the command it started, which runs as
            // far as the conversation knows, so that its later report and
            // notice find their call.
            let view = command
                .clone()
                .map(|command| ToolView::Shell(ShellToolView::named(command, ShellViewStatus::Running)));
            if let Some(started) = started
                && watched.get(&started.command).is_none()
            {
                // It wakes the node as its call asked, which its record kept.
                watched.add(started.command.clone(), Watch::started(started.background));
            }
            transcript.complete_tool_call(
                &call.tool_use_id,
                vec![ToolResultContentBlock::Text { text }],
                true,
                view,
            );
        }
        let interrupted = state.phase != SessionPhase::Idle;
        let parts = CoreParts {
            id,
            model: state.model,
            provider: runtime,
            transcript,
            media: HeldMedia::default(),
            tools: deps.runtime.tools(),
            inputs: InputQueue::restored(state.agent_inputs, state.reports, state.next_turn_reports),
            watched,
            edits: state.edits,
            account: state.account,
            turn_speed: state.turn_speed,
            one_request: false,
            request_turn: None,
            ids: deps.ids.clone(),
            clock: deps.clock.clone(),
            runtime: deps.runtime.clone(),
        };
        let continuation = Continuation {
            interrupted,
            queued: state.queue,
        };
        let user_recovers = deps.runtime.user_recovers_turns();
        let session = Self::start(SessionCore::new(parts), deps);
        // The rows the store holds are current; the completed calls mark
        // theirs, and an interruption record its own, for the next save.
        session.shared.update(|core| {
            if !user_recovers {
                core.hold();
            } else if interrupted {
                core.record_interruption();
            }
            core.commit();
        });
        let commands = session.shared.read(|core| core.watched.commands());
        for command in &commands {
            reports::start_watch(&session.shared, command);
        }
        Ok((session, continuation))
    }

    fn start(core: SessionCore, deps: SessionDeps) -> Self {
        let status = core.status();
        let arrivals = core.window_arrivals();
        let shared = Rc::new(SessionShared {
            core: RefCell::new(core),
            bus: EventBus::default(),
            persist_gate: SerialGate::new(),
            status: watch::Sender::new(status),
            arrivals: watch::Sender::new(arrivals),
            work: Rc::new(Notify::new()),
            persist_wake: Rc::new(Notify::new()),
            runtime_returned: Notify::new(),
            finalized: Cell::new(false),
            runtime: deps.runtime,
            store: deps.store,
            ids: deps.ids,
            config: deps.config,
            worker: RefCell::new(None),
            persister: RefCell::new(None),
        });
        // The session's work logs where its start was asked, such as a
        // socket's frame or a request
        // (`crates/backend-http/src/request_id.rs`).
        let worker = tokio::task::spawn_local(
            worker::run(Rc::downgrade(&shared), shared.work.clone()).in_current_span(),
        );
        let persister = tokio::task::spawn_local(
            persist::run(
                Rc::downgrade(&shared),
                shared.persist_wake.clone(),
                deps.config.persist_interval,
            )
            .in_current_span(),
        );
        *shared.worker.borrow_mut() = Some(AbortOnDropHandle::new(worker));
        *shared.persister.borrow_mut() = Some(AbortOnDropHandle::new(persister));
        Self { shared }
    }

    /// A new node's first checkpoint: its state row, with no blocks.
    pub fn first_checkpoint(&self) -> CheckpointUpdate {
        self.shared.read(|core| CheckpointUpdate {
            state: core.checkpoint_state(),
            changed_blocks: Vec::new(),
            block_count: 0,
            moved_from: None,
            version: false,
        })
    }

    pub fn id(&self) -> NodeId {
        self.shared.read(|core| core.id.clone())
    }

    pub fn phase(&self) -> SessionPhase {
        self.shared.read(SessionCore::phase)
    }

    /// How long the node has waited for the model's answer, while it
    /// waits.
    pub fn waited_ms(&self) -> Option<u64> {
        self.shared.read(SessionCore::waited_ms)
    }

    /// The output of the turn's latest request, while a turn runs.
    pub fn request_output(&self) -> Option<RequestOutput> {
        self.shared.read(SessionCore::request_output)
    }

    /// Whether no action runs or waits.
    pub fn is_settled(&self) -> bool {
        self.shared.read(|core| core.settle()) != Settle::Busy
    }

    /// Waits until no action runs or waits.
    pub async fn settled(&self) {
        let mut status = self.shared.status.subscribe();
        // The sender lives in the session this handle keeps alive.
        let _ = status
            .wait_for(|status| status.settle != Settle::Busy)
            .await;
    }

    /// The model selection current now.
    pub fn model(&self) -> ModelSelection {
        self.shared.read(|core| core.model.clone())
    }

    /// The transcript's version, without copying its blocks.
    pub fn transcript_version(&self) -> Version {
        self.shared.read(|core| core.transcript.version())
    }

    /// The blocks the session holds at one moment, its window, with its
    /// version: the whole transcript while no compaction let go of the
    /// blocks before it.
    pub fn transcript(&self) -> TranscriptSnapshot {
        self.shared.read(|core| TranscriptSnapshot {
            start: core.transcript.start(),
            blocks: core.transcript.blocks().to_vec(),
            version: core.transcript.version(),
            versions: core.transcript.versions().clone(),
        })
    }

    /// The whole transcript at one moment, the blocks before the window
    /// read from the store: for a reading of the whole history, which holds
    /// it no longer than it needs it.
    pub async fn history(&self) -> Result<TranscriptSnapshot, StoreError> {
        let snapshot = |core: &SessionCore, earlier: demi_agent_store::Rows| {
            let mut blocks = earlier.blocks;
            blocks.extend_from_slice(core.transcript.blocks());
            let mut versions = earlier.versions;
            versions.extend(core.transcript.versions().iter().map(|(at, held)| (*at, *held)));
            TranscriptSnapshot {
                start: 0,
                blocks,
                version: core.transcript.version(),
                versions,
            }
        };
        if self.shared.read(|core| core.transcript.start()) == 0 {
            return Ok(self.shared.read(|core| snapshot(core, Default::default())));
        }
        let _turn = self.shared.persist_gate.acquire().await;
        let start = self.shared.read(|core| core.transcript.start());
        let earlier = self.shared.store.blocks(0..start).await?;
        Ok(self.shared.read(|core| snapshot(core, earlier)))
    }

    /// Reads the session's window: the blocks it holds, by their
    /// positions on the path, and what it keeps of those before.
    pub fn with_window<R>(&self, look: impl FnOnce(&TranscriptLog) -> R) -> R {
        self.shared.read(|core| look(&core.transcript))
    }

    /// The transcript a reset sends to a page that holds its blocks up to
    /// `from`, whose block there is `edge`: at most its latest page, in its
    /// light form, read from the window as it is now.
    pub fn reset(&self, from: Option<u32>, edge: Option<&BlockId>) -> Reset {
        window::reset(&self.shared, from, edge)
    }

    /// The automation's run whose message is the latest the user or an
    /// automation sent this agent, with when it came.
    ///
    /// Every turn it starts or that continues from it works for that run, until
    /// the user writes again, in a message or a steer. None when the user's own
    /// is the latest, or neither came.
    pub fn automation_input(&self) -> Option<(AutomationMark, Timestamp)> {
        self.shared.read(|core| core.transcript.user_input().flatten())
    }

    pub fn queued_messages(&self) -> Vec<QueuedMessage> {
        self.shared.read(SessionCore::queued_messages)
    }

    /// The human steers accepted and not yet written.
    pub fn pending_steers(&self) -> Vec<PendingSteer> {
        self.shared.read(SessionCore::pending_steers)
    }

    /// The message the running turn started with, until its `user` block is
    /// written.
    pub fn starting(&self) -> Option<QueuedMessage> {
        self.shared.read(SessionCore::starting)
    }

    /// The calls the model is writing.
    pub fn pending_calls(&self) -> Vec<PendingCall> {
        self.shared.read(SessionCore::pending_calls)
    }

    /// The last `text` block, the answer a child closes with: its id, which
    /// its completion names, and its text, the result.
    pub fn last_answer(&self) -> Option<(BlockId, String)> {
        self.shared.read(|core| {
            last_answer(core.transcript.blocks(), 0).map(|text| (text.id.clone(), text.text.clone()))
        })
    }

    /// The last `error` block, the record of the failure a failed child
    /// closes with, which its completion names.
    pub fn last_failure(&self) -> Option<BlockId> {
        self.shared
            .read(|core| last_failure(core.transcript.blocks()).map(|error| error.id.clone()))
    }

    /// The block that ended the latest ended turn, the one a conversation's
    /// summary names: what a turn that ends later is told apart from.
    pub fn ended_turn(&self) -> Option<BlockId> {
        self.shared
            .read(|core| demi_agent_transcript::ended_turn(core.transcript.blocks()).map(|block| block.id().clone()))
    }

    /// The latest ended turn as it ended: the block that ended it, how, and
    /// the Markdown of its last text that is not blank.
    pub fn ended_turn_result(&self) -> Option<EndedTurn> {
        self.shared.read(|core| {
            let (end, answer) = demi_agent_transcript::ended_turn_answer(core.transcript.blocks())?;
            let outcome = match end {
                Block::Response(_) => TurnOutcome::Answered,
                Block::Error(_) => TurnOutcome::Failed,
                _ => TurnOutcome::Stopped,
            };
            Some(EndedTurn {
                id: end.id().clone(),
                outcome,
                answer: answer.map(|text| text.text.clone()),
            })
        })
    }

    /// The record of the last turn when it failed: the `error` block that
    /// left it unfinished.
    pub fn failed_turn(&self) -> Option<ErrorBlock> {
        self.shared.read(|core| core.transcript.turn_failure().cloned())
    }

    /// The block of the call `tool_use_id`, which a message its command
    /// sends comes from; none when the transcript does not hold it.
    pub fn call_block(&self, tool_use_id: &str) -> Option<BlockId> {
        self.shared.read(|core| {
            core.transcript.blocks().iter().rev().find_map(|block| match block {
                Block::ToolCall(call) if call.tool_use_id == tool_use_id => Some(call.id.clone()),
                _ => None,
            })
        })
    }

    /// Holds the bytes the backend read of the media a frame's content
    /// references, for the send, steer or edit that follows it; what nothing
    /// references by the next request is let go.
    pub fn hold_media(&self, media: HeldMedia) {
        self.shared.update(|core| core.media.absorb(media));
    }

    /// Submits a message the user wrote.
    pub fn send(
        &self,
        content: Vec<UserContentBlock>,
        id: TurnId,
    ) -> Result<ActionHandle, AdmissionError> {
        self.send_message(QueuedMessage {
            id,
            content,
            comments: Vec::new(),
            automation: None,
        })
    }

    /// Submits a message, which may be an automation's run's.
    pub fn send_message(&self, message: QueuedMessage) -> Result<ActionHandle, AdmissionError> {
        self.shared.update(|core| core.admit_send(message))
    }

    /// Rewinds the last input turn and runs it again.
    pub fn retry(&self) -> Result<ActionHandle, AdmissionError> {
        self.shared.update(|core| core.admit(ActionKind::Retry))
    }

    /// Unwinds the unfinished turn to its resume point and continues it.
    ///
    /// `reason`, when given, is what the model reads before it goes on: Demi's
    /// own for a resume of its own, such as a subagent's after a backend
    /// restart.
    pub fn resume(&self, reason: Option<&str>) -> Result<ActionHandle, AdmissionError> {
        self.shared.update(|core| {
            core.admit(ActionKind::Resume {
                reason: reason.map(str::to_owned),
            })
        })
    }

    /// The estimate of the next request with the window in use. It reads
    /// the media of the blocks the estimate weighs when the session holds
    /// nothing for them, as after a restore.
    pub async fn context_usage(&self) -> Result<ContextUsage, ErrorReport> {
        // Outside an action nothing stops the read.
        let unstoppable = TurnCancel::new();
        compaction::context_usage(&self.shared, &unstoppable)
            .await
            .map_err(|error| match error {
                TurnError::Failed(report) => *report,
                TurnError::Cancelled => unreachable!("a token nothing cancels"),
            })
    }

    /// The usage as [`context_usage`](Self::context_usage) says it, without
    /// reading a blob.
    ///
    /// None while a block the estimate weighs references a medium the session
    /// holds nothing for, as a restored session can. The estimate is read when
    /// the window in use is known, with nothing awaited after it, so it is the
    /// session's estimate at the moment this returns.
    pub async fn held_context_usage(&self) -> Option<ContextUsage> {
        compaction::held_context_usage(&self.shared).await
    }

    /// Why the user's `compact` would be refused now, or none when it is taken.
    ///
    /// It is refused below half the window in use, unless the unfinished turn
    /// failed for its size. A usage that cannot be read refuses it with the
    /// read's failure.
    pub async fn compaction_refusal(&self) -> Option<Refusal> {
        let size_refused = self
            .shared
            .read(|core| compaction::size_refused(&core.transcript));
        match self.context_usage().await {
            Ok(usage) => compaction_refusal(&usage, size_refused),
            Err(report) => Some(Refusal::new(AgentCode::FrameFailed, report.message)),
        }
    }

    /// Runs one compaction pass.
    pub fn compact(&self) -> Result<ActionHandle, AdmissionError> {
        self.shared.update(|core| core.admit(ActionKind::Compact))
    }

    pub fn dequeue_message(&self, id: &TurnId) -> bool {
        self.shared.update(|core| core.dequeue(id))
    }

    /// Sends a queued message now: while a turn runs, the turn ends at
    /// once and the message runs next; otherwise it moves to the front of
    /// the queue. False when no queued message has the id.
    pub fn send_queued_message(&self, id: &TurnId) -> bool {
        self.shared.update(|core| core.send_queued_now(id))
    }

    pub fn clear_message_queue(&self) -> usize {
        self.shared.update(SessionCore::clear_queue)
    }

    /// Adds input to the running turn at its next boundary.
    pub fn steer(&self, content: Vec<UserContentBlock>, id: BlockId) -> Result<(), SteerError> {
        self.steer_with_comments(content, Vec::new(), id)
    }

    /// A steer that carries comments on the reply's words, which the model
    /// reads before its content.
    pub fn steer_with_comments(
        &self,
        content: Vec<UserContentBlock>,
        comments: Vec<MessageComment>,
        id: BlockId,
    ) -> Result<(), SteerError> {
        self.shared.update(|core| core.steer(id, content, comments))
    }

    /// Withdraws a pending steer; an id that is not pending changes nothing.
    pub fn cancel_pending_steer(&self, id: &BlockId) -> bool {
        self.shared.update(|core| core.cancel_steer(id))
    }

    /// Delivers a pending steer now: the running round is cut short
    /// without stopping anything, and the turn goes on with the steer. A
    /// steer that is no longer pending changes nothing.
    pub fn steer_now(&self, id: &BlockId) {
        self.shared.update(|core| core.steer_now(id));
    }

    /// Admits a message from another agent of the tree; it returns once the
    /// admission is saved (`crates/agent-session/src/input.rs`).
    pub async fn accept_agent_message(
        &self,
        message: AgentMessage,
    ) -> Result<(), AgentMessageError> {
        if let Err(report) = garde::Validate::validate(&message) {
            return Err(AgentMessageError::Invalid(report.to_string()));
        }
        // A message whose block is before the window is admitted once too:
        // its block is read from the store, while the save order keeps the
        // window where it is.
        if self.shared.read(|core| core.transcript.start()) == 0 {
            self.shared.update(|core| core.admit_agent_message(message, None))?;
        } else {
            let _turn = self.shared.persist_gate.acquire().await;
            let unheld = self.shared.read(|core| {
                core.transcript.start() > 0
                    && core.transcript.position(&message.id).is_none()
                    && !core.inputs.contains(message.id.as_str())
            });
            let stored = match unheld {
                true => match self.shared.store.position(&message.id).await? {
                    Some(at) => self.shared.store.blocks(at..at + 1).await?.blocks.pop(),
                    None => None,
                },
                false => None,
            };
            self.shared
                .update(|core| core.admit_agent_message(message, stored.as_ref()))?;
        }
        // A message admitted before is saved already, or its save is under
        // way ahead of this one; either way it is durable when this returns.
        persist::flush(&self.shared).await?;
        Ok(())
    }

    /// What the session knows of an edit request before its uploads are
    /// resolved, so that a repeated request writes no file.
    pub fn check_edit(
        &self,
        operation: &demi_shared_types::OperationId,
        digest: &str,
        version: &Version,
    ) -> Result<EditCheck, EditError> {
        self.shared
            .read(|core| core.check_edit(operation, digest, version))
    }

    /// Replaces a user message and everything after it; returns at durable
    /// acceptance, and the replacement's turn runs on. An operation
    /// accepted before returns its receipt again.
    pub async fn edit_and_send(
        &self,
        submission: EditSubmission,
    ) -> Result<demi_agent_store::EditReceipt, EditError> {
        match self.shared.update(|core| core.admit_edit(submission))? {
            EditCheck::Accepted(receipt) => Ok(receipt),
            EditCheck::InFlight(acceptance) => accepted(acceptance).await,
            EditCheck::Proceed => unreachable!("an admitted edit is in flight"),
        }
    }

    /// Makes the `to`th version at the block `target`'s place current,
    /// admitted as an edit is against the transcript `version` the page
    /// showed; returns once the switch is published, or why nothing changed.
    pub async fn switch_version(
        &self,
        target: BlockId,
        to: u32,
        version: &Version,
    ) -> Result<(), Refusal> {
        let handle = self
            .shared
            .update(|core| core.admit_switch(target, to, version))
            .map_err(|error| error.refusal())?;
        match handle.await {
            Ok(_) => Ok(()),
            Err(report) => Err(Refusal {
                code: report.code.clone().unwrap_or_else(|| AgentCode::FrameFailed.to_string()),
                params: Default::default(),
                reason: report.message.clone(),
            }),
        }
    }

    /// Whether the message `id` is the running turn, a queued message or the
    /// turn of a `user` block, which a message sent again under it does not
    /// run twice.
    pub fn knows_turn(&self, id: &TurnId) -> bool {
        self.shared.read(|core| core.knows_turn(id))
    }

    /// Whether the tool call `id` is on the transcript's current path.
    pub fn holds_call(&self, id: &BlockId) -> bool {
        self.shared.read(|core| core.transcript.holds_call(id))
    }

    /// The blocks that tell how `turn` ended.
    ///
    /// These are the `text`, `error`, `response` and `user` blocks after the
    /// message that opened `turn`, through the next message, which tells the
    /// turn ended, or to the transcript's end. A turn whose message is before
    /// the window, as when a compaction ran inside it or later turns went on
    /// and compacted, is read from the store up to the window, through the
    /// index of block types. None when the path holds no message of `turn`.
    pub async fn turn_blocks(&self, turn: &TurnId) -> Result<Option<Vec<Block>>, StoreError> {
        let kinds = [Kind::Text, Kind::Error, Kind::Response, Kind::User];
        let of_kinds = |block: &Block| kinds.iter().any(|kind| kind.holds(block));
        let opens = |block: &Block| matches!(block, Block::User(user) if &user.turn_id == turn);
        let (start, held, known) = self.shared.read(|core| {
            let transcript = &core.transcript;
            (transcript.start(), transcript.blocks().iter().position(opens), transcript.has_user_turn(turn))
        });
        let window_from = |from: usize| {
            self.shared.read(|core| {
                let blocks = core.transcript.blocks();
                let end = blocks[from..]
                    .iter()
                    .position(|block| matches!(block, Block::User(_)))
                    .map_or(blocks.len(), |next| from + next + 1);
                blocks[from..end].iter().filter(|block| of_kinds(block)).cloned().collect::<Vec<_>>()
            })
        };
        if let Some(at) = held {
            return Ok(Some(window_from(at + 1)));
        }
        if !known {
            return Ok(None);
        }
        let store = &self.shared.store;
        let Some(at) = store.turn_position(turn).await? else {
            return Ok(None);
        };
        let next = store.next_of(Kind::User, at + 1).await?;
        let end = next.map_or(start, |next| (next + 1).min(start));
        let read = store.blocks_of(kinds.to_vec(), at + 1..end).await?;
        let mut blocks: Vec<Block> = read.into_iter().map(|(_, block)| block).collect();
        // The turn goes on into the window when no message of the store
        // follows it there; the window may have let go of more meanwhile,
        // and starts where it does now.
        if next.is_none_or(|next| next >= start) {
            let now = self.shared.read(|core| core.transcript.start());
            if now == start {
                blocks.extend(window_from(0));
            }
        }
        Ok(Some(blocks))
    }

    /// The node's checkpoint store, where the blocks before the window
    /// are read.
    pub fn store(&self) -> Rc<dyn SessionStore> {
        self.shared.store.clone()
    }

    /// Whether the block `id` is on the transcript's current path: held,
    /// or before the window as the store finds it.
    pub async fn holds(&self, id: &BlockId) -> Result<bool, StoreError> {
        Ok(window::position(&self.shared, id).await?.is_some())
    }

    /// The latest call still executing: the one a command it runs, such as
    /// `demi agent`, acts for, so a child it starts belongs to that call's
    /// branch.
    pub fn executing_call(&self) -> Option<BlockId> {
        self.shared.read(|core| {
            core.transcript.blocks().iter().rev().find_map(|block| match block {
                Block::ToolCall(call) if call.status == demi_shared_types::ToolCallStatus::Executing => {
                    Some(call.id.clone())
                }
                _ => None,
            })
        })
    }

    /// The tool-use IDs of the calls still executing, the calls of the
    /// model's response that run at the same time.
    pub fn executing_calls(&self) -> Vec<String> {
        self.shared.read(|core| {
            core.transcript
                .blocks()
                .iter()
                .filter_map(|block| match block {
                    Block::ToolCall(call) if call.status == demi_shared_types::ToolCallStatus::Executing => {
                        Some(call.tool_use_id.clone())
                    }
                    _ => None,
                })
                .collect()
        })
    }

    /// A Fork's seed from this live session, through the completed text `end`
    /// names.
    ///
    /// The seed is captured in one step, with the model a switch the session
    /// accepted would use. The blocks before the window are named as the
    /// store's rows, which the destination's store copies or starts from, under the session's
    /// save order, so no rewrite before the window comes between the capture
    /// and the naming.
    pub async fn prepare_fork(&self, end: ForkEnd<'_>) -> Result<ForkSeed, ForkError> {
        let capture = |core: &SessionCore| {
            let state = demi_agent_store::CheckpointState {
                model: core.latest_selection().clone(),
                ..core.checkpoint_state()
            };
            (core.transcript.start(), core.transcript.blocks().to_vec(), state)
        };
        let _turn = match self.shared.read(|core| core.transcript.start()) {
            0 => None,
            _ => Some(self.shared.persist_gate.acquire().await),
        };
        let (start, window, state) = self.shared.read(capture);
        fork_seed(&*self.shared.store, start, &window, state, end).await
    }

    /// Opens a continuation for input that waits, when nothing runs: the
    /// node's policy calls it once a restored session may act.
    pub fn wake(&self) {
        self.shared.update(SessionCore::wake);
    }

    /// Keeps waiting input from opening a continuation until the next action
    /// starts, as a restored interrupted session does.
    ///
    /// A closing child admits what its own children deliver, keeps it in its
    /// final checkpoint, and runs nothing more.
    pub fn hold(&self) {
        self.shared.update(SessionCore::hold);
    }

    /// Lets waiting input open a continuation again after
    /// [`hold`](Self::hold), and opens one when nothing runs.
    pub fn release(&self) {
        self.shared.update(SessionCore::release);
    }

    /// How `command`, which one of the session's calls left running, wakes the
    /// node; none when the session does not watch it.
    pub fn watch_of(&self, command: &CommandId) -> Option<Watch> {
        self.shared.read(|core| core.watched.get(command))
    }

    /// Stops one thing: the running action, which has recorded the stop
    /// when this returns; else the first waiting action. Whether another
    /// `abort` would stop more is read when the stop is recorded.
    pub async fn abort(&self) -> AbortResult {
        match self.shared.update(SessionCore::abort_step) {
            AbortStep::Running { target, cancel } => AbortResult {
                target: Some(target),
                can_abort_again: cancel.recorded().await,
            },
            AbortStep::Removed(target) => AbortResult {
                target: Some(target),
                can_abort_again: self.shared.read(SessionCore::can_abort_again),
            },
            AbortStep::Nothing => AbortResult {
                target: None,
                can_abort_again: false,
            },
        }
    }

    /// Stops the running action and returns once it recorded the stop.
    ///
    /// What waits to run stays, and runs once admission lets it. `hold` names
    /// the hold, as the stopped calls' results say it
    /// (`crates/agent-session/src/core.rs`), such as "the Cloud is being
    /// reset".
    pub async fn stop_running(&self, hold: &str) {
        let hold: Rc<str> = hold.into();
        if let Some(cancel) = self.shared.update(|core| core.stop_running(hold)) {
            cancel.recorded().await;
        }
    }

    /// Records a model switch, which lands at the next provider request.
    ///
    /// A switch that arrives while an edit is being prepared waits for the
    /// edit. A runtime the switch replaces is closed once it serves no run. A
    /// session that is closing gives the switch back.
    pub fn update_model(&self, switch: ModelSwitch) -> Result<(), ModelSwitch> {
        self.shared.update(|core| core.record_switch(switch))
    }

    /// Readies ahead what the session's next request needs, now that it is
    /// known but for its input (`crates/provider-claude-code/src/prepared.rs`).
    ///
    /// It is called when a page opened the conversation, or the user chose
    /// another model, effort or account for it. From then on each action's end
    /// readies it again. It is done once the session is idle; a subagent's
    /// session is never asked.
    pub fn prepare_ahead(&self, ahead: Ahead) {
        self.shared.update(|core| core.ask_ahead(ahead));
    }

    /// The selection the next request lands with, or the one a switch that
    /// waits for an edit names.
    pub fn latest_model(&self) -> ModelSelection {
        self.shared.read(|core| core.latest_selection().clone())
    }

    /// Whether `model` needs a runtime of its own: it belongs to another
    /// provider than the selection the next request would use.
    pub fn needs_runtime_for(&self, model: &ModelSelection) -> bool {
        self.shared
            .read(|core| core.latest_selection().provider_id != model.provider_id)
    }

    /// A runtime fork of the session's current runtime, when that runtime
    /// serves the provider entry `provider_id`.
    ///
    /// The fork has the same provider and configuration and none of its
    /// execution state. It is none when the runtime serves another entry. A run
    /// in progress holds the runtime, so the fork waits until it is back.
    pub async fn fork_runtime_for(
        &self,
        provider_id: &str,
    ) -> Result<Option<Box<dyn ProviderRuntime>>, AdmissionError> {
        loop {
            let returned = self.shared.runtime_returned.notified();
            tokio::pin!(returned);
            returned.as_mut().enable();
            let fork = self.shared.read(|core| {
                if core.disposing {
                    return Some(Err(AdmissionError::Closed));
                }
                // The runtime serves the selection the last request landed.
                if core.model.provider_id != provider_id {
                    return Some(Ok(None));
                }
                core.provider.held().map(|runtime| Ok(Some(runtime.fresh())))
            });
            if let Some(fork) = fork {
                return fork;
            }
            returned.await;
        }
    }

    /// Saves now, after the saves ahead in the session's order.
    pub async fn flush(&self) -> Result<(), StoreError> {
        persist::flush(&self.shared).await
    }

    /// Stops the session for good, at once and without waiting: it refuses
    /// new actions, its running one is cancelled as by a shutdown, and a fork
    /// waiting for its runtime learns it is closing. A close stops what it
    /// owns this way before it waits for anything
    /// (`docs/decisions/0024-a-close-stops-before-it-waits.md`); `dispose`
    /// then saves the final checkpoint.
    pub fn stop(&self) {
        self.shared.update(SessionCore::begin_dispose);
        self.shared.runtime_returned.notify_waiters();
    }

    /// Disposes the session: stops it ([`AgentSession::stop`]), keeps the
    /// queue, the waiting input and the commands that report to it, saves the
    /// final checkpoint and closes the provider runtimes.
    ///
    /// The error is a final checkpoint that could not be saved.
    pub async fn dispose(&self) -> Result<(), StoreError> {
        self.stop();
        let mut status = self.shared.status.subscribe();
        // The sender lives in the session this handle keeps alive.
        let _ = status
            .wait_for(|status| status.settle == Settle::Closed)
            .await;
        if self.shared.finalized.replace(true) {
            return Ok(());
        }
        let saved = {
            let _turn = self.shared.persist_gate.acquire().await;
            // With the order held, the persister is between saves.
            self.shared.persister.borrow_mut().take();
            persist::write_if_dirty(&self.shared).await
        };
        // The commands' watches end with the session; the final checkpoint
        // keeps the commands.
        let runtimes = self.shared.update(|core| {
            core.watched.stop_watching();
            core.take_runtimes()
        });
        for mut runtime in runtimes {
            runtime.close().await;
        }
        self.shared.runtime.dispose().await;
        saved
    }

    /// Calls `listener` with every event from now on, step after step
    /// (`bus`), until the subscription is dropped.
    pub fn subscribe(&self, mut listener: impl FnMut(&SessionEvent) + 'static) -> Subscription {
        self.subscribe_steps(move |step| step.iter().for_each(&mut listener))
    }

    /// Calls `listener` with the events of each step, one change of the
    /// session, which a client applies together (`bus`).
    pub fn subscribe_steps(&self, listener: impl FnMut(&[SessionEvent]) + 'static) -> Subscription {
        let id = self.shared.bus.subscribe(Box::new(listener));
        Subscription {
            session: Rc::downgrade(&self.shared),
            id,
        }
    }

    /// Resolves once every change made before the call reached the listeners;
    /// at once unless the end of a turn holds them (`bus`).
    pub async fn delivered(&self) {
        self.shared.bus.delivered().await;
    }

    pub fn status_watch(&self) -> watch::Receiver<Status> {
        self.shared.status.subscribe()
    }

    pub fn status(&self) -> Status {
        *self.shared.status.borrow()
    }

    /// What the session is doing now, as a supervisor observes it.
    pub fn execution(&self) -> Execution {
        self.shared.read(SessionCore::execution)
    }

    /// Whether the block at `index` is assistant text.
    pub fn is_text_at(&self, index: usize) -> bool {
        self.shared
            .read(|core| matches!(core.transcript.get(index), Some(Block::Text(_))))
    }
}

/// The session's own checks of a checkpoint's waiting input: each agent
/// message is addressed to this node, and its id is unique and not yet in
/// the transcript.
fn check_restored_input(
    node: &NodeId,
    blocks: &[Block],
    agent_inputs: &[demi_agent_store::PendingAgentInput],
) -> Result<(), RestoreError> {
    let mut ids = HashSet::new();
    for input in agent_inputs {
        let message = &input.message;
        if &message.recipient_id != node {
            return Err(RestoreError::Input(format!(
                "agent message {} is addressed to {}",
                message.id, message.recipient_id
            )));
        }
        if !ids.insert(message.id.as_str()) || blocks.iter().any(|block| block.id() == &message.id)
        {
            return Err(RestoreError::Input(format!(
                "agent message {} is not unique",
                message.id
            )));
        }
    }
    Ok(())
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    /// With an answer.
    Answered,
    /// With an error.
    Failed,
    /// The user stopped it.
    Stopped,
}

/// A turn that ended, as [`Session::ended_turn_result`] reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedTurn {
    /// The block that ended it.
    pub id: BlockId,
    pub outcome: TurnOutcome,
    /// The Markdown of its last text that is not blank.
    pub answer: Option<String>,
}
