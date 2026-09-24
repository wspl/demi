//! One agent's session (`runtime.md` § Sessions and turns): a cloneable
//! handle over the session's state, which one worker task changes by running
//! one action at a time while commands from the connection change it between
//! the worker's awaits. The worker, the persister, the wakeup driver and every
//! command change the state only through [`SessionShared::update`], whose
//! synchronous closure ends before any await, and each change's events reach
//! the listeners after the state is released.

mod bus;
mod cancel;
mod compaction;
mod core;
mod editing;
mod input;
mod persist;
mod retry;
mod runtime;
mod storage;
mod turn;
mod wakeups;
mod worker;

#[cfg(test)]
mod tests;

use std::{
    cell::RefCell,
    collections::HashSet,
    future::Future,
    pin::Pin,
    rc::{Rc, Weak},
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use demi_agent_protocol::{AbortResult, ModelSwitchApply, TranscriptPatch, TranscriptVersion};
use demi_core::{
    AgentMessage, Block, BlockId, Clock, ModelSelection, NodeId, PendingSteer,
    ProviderErrorDiagnostics, QueuedMessage, SessionPhase, ToolResultContentBlock, TurnId,
    UserContentBlock,
};
use demi_gates::SerialGate;
use demi_provider::{ProviderFailure, ProviderRuntime};
use schemars::JsonSchema;
use serde::Serialize;
use tokio::sync::{Notify, oneshot, watch};
use tokio_util::task::AbortOnDropHandle;

pub use compaction::CompactionConfig;
pub use editing::ForkError;
pub(crate) use editing::{
    EditCheck, EditContent, EditError, EditSubmission, accepted, edit_digest, fork_seed,
};
pub use retry::RetryPolicy;
pub(crate) use runtime::{SessionRuntime, ToolEffect, ToolFailure, ToolInvocation, ToolOutcome};

use self::{
    bus::EventBus,
    core::{AbortStep, ActionKind, CoreParts, SessionCore},
    input::{InputQueue, Wakeups},
};
use crate::{
    IdSource,
    store::{
        Checkpoint, CheckpointUpdate, CommandStateError, CommandStateHistory, SessionStore,
        StoreError,
    },
    transcript::{TranscriptLog, last_assistant_text},
};

/// How a session saves, retries and compacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// How long after the first unsaved change a save starts.
    pub persist_interval: Duration,
    pub retry: RetryPolicy,
    pub compaction: CompactionConfig,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            persist_interval: Duration::from_secs(1),
            retry: RetryPolicy::default(),
            compaction: CompactionConfig::default(),
        }
    }
}

/// What a session is built with besides its state: its node's runtime, its
/// checkpoint store, and where identities and times come from.
pub(crate) struct SessionDeps {
    pub(crate) runtime: Rc<dyn SessionRuntime>,
    pub(crate) store: Rc<dyn SessionStore>,
    pub(crate) ids: Rc<dyn IdSource>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) config: SessionConfig,
}

/// A new session's state.
pub(crate) struct SessionInit {
    pub(crate) id: NodeId,
    pub(crate) cwd: String,
    pub(crate) model: ModelSelection,
    pub(crate) runtime: Box<dyn ProviderRuntime>,
}

/// What a restored session hands back for its node to decide on
/// (`runtime.md` § Dispose and restore).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Continuation {
    /// The checkpoint says a turn was running: the process died in it, or
    /// dispose stopped it.
    pub(crate) interrupted: bool,
    /// The messages queued in the checkpoint, in order.
    pub(crate) queued: Vec<QueuedMessage>,
}

/// A change of the model selection (`runtime.md` § Model switch).
pub(crate) struct ModelSwitch {
    pub(crate) model: ModelSelection,
    /// A new runtime when the model belongs to another provider than the
    /// selection before it.
    pub(crate) runtime: Option<Box<dyn ProviderRuntime>>,
    pub(crate) apply: ModelSwitchApply,
}

/// The transcript at one moment, with its version.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptSnapshot {
    pub blocks: Vec<Block>,
    pub version: TranscriptVersion,
}

/// How an action ended when it did not fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionEnd {
    Completed,
    /// It was stopped, and recorded the stop.
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
pub(crate) struct ActionHandle(oneshot::Receiver<ActionResult>);

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
pub(crate) enum AdmissionError {
    #[error("The agent session is closed")]
    Closed,
    #[error("A message edit is being prepared")]
    Editing,
}

/// Why a steer was refused (`runtime.md` § Steers).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SteerError {
    #[error("No turn is running to steer")]
    NotRunning,
    #[error("The running turn is being stopped")]
    Stopped,
    #[error("The running turn is finishing")]
    Finishing,
    #[error("A message edit is being prepared")]
    Editing,
}

/// Why an agent message was refused (`subagents.md` § Communication).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum AgentMessageError {
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
pub(crate) enum RestoreError {
    #[error("The checkpoint's harness \"{stored}\" is not \"{expected}\"")]
    Harness { stored: String, expected: String },
    #[error(transparent)]
    CommandState(#[from] CommandStateError),
    #[error("The checkpoint's waiting input is invalid: {0}")]
    Input(String),
    #[error("The checkpoint's edit receipts repeat operation {0}")]
    Edits(String),
}

/// A failure as clients see it: the frame an `error` event becomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ErrorReport {
    pub(crate) message: String,
    pub(crate) code: Option<String>,
    pub(crate) diagnostics: Option<ProviderErrorDiagnostics>,
}

impl From<&ProviderFailure> for ErrorReport {
    fn from(failure: &ProviderFailure) -> Self {
        Self {
            message: failure.message.clone(),
            code: failure.code.as_ref().map(|code| code.as_str().to_owned()),
            diagnostics: failure.diagnostics.clone(),
        }
    }
}

impl From<StoreError> for ErrorReport {
    fn from(error: StoreError) -> Self {
        Self {
            message: error.to_string(),
            code: None,
            diagnostics: None,
        }
    }
}

/// A change of a session, reported once it is complete.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SessionEvent {
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
    /// The human steers waiting for a boundary changed.
    PendingSteersChanged {
        pending_steers: Vec<PendingSteer>,
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
    /// The action cannot do what it was asked.
    pub(crate) fn refused(message: impl Into<String>) -> Self {
        Self::Failed(Box::new(ErrorReport {
            message: message.into(),
            code: None,
            diagnostics: None,
        }))
    }
}

impl From<StoreError> for TurnError {
    fn from(error: StoreError) -> Self {
        Self::Failed(Box::new(error.into()))
    }
}

/// Whether the session will do anything more by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settle {
    /// An action runs or waits.
    Busy,
    Settled,
    /// Disposed: nothing will ever run again.
    Closed,
}

/// What the tree watches a session for: whether it acts, and what could
/// make it act later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Status {
    pub(crate) settle: Settle,
    /// A yield wakeup is scheduled, or fired and not yet written.
    pub(crate) wakeups: bool,
    /// An agent message waits for a boundary.
    pub(crate) agent_input: bool,
}

/// What a session is doing, as a supervisor observes it (`subagents.md`
/// § `demi agent show`); not the phase that `phase` frames carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Execution {
    Idle,
    ProviderStreaming,
    ToolExecuting,
    Compacting,
    /// The action ended and its checkpoint is being saved.
    Finalizing,
    /// Idle with a yield wakeup scheduled.
    PendingYield,
}

impl Execution {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::ProviderStreaming => "provider_streaming",
            Self::ToolExecuting => "tool_executing",
            Self::Compacting => "compacting",
            Self::Finalizing => "finalizing",
            Self::PendingYield => "pending_yield",
        }
    }
}

/// Ends a listener's subscription when dropped.
pub(crate) struct Subscription {
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
    /// The one order of saves and command-state commits: a save that started
    /// finishes before the next one starts.
    persist_gate: SerialGate,
    status: watch::Sender<Status>,
    /// Wakes the worker when an action starts.
    work: Rc<Notify>,
    /// Wakes the persister when a change makes a save due.
    persist_wake: Rc<Notify>,
    /// Wakes the wakeup driver when the scheduled wakeups change.
    replan: Rc<Notify>,
    /// Tells a waiting fork that the runtime is back in its slot.
    runtime_returned: Notify,
    runtime: Rc<dyn SessionRuntime>,
    store: Rc<dyn SessionStore>,
    ids: Rc<dyn IdSource>,
    config: SessionConfig,
    worker: RefCell<Option<AbortOnDropHandle<()>>>,
    persister: RefCell<Option<AbortOnDropHandle<()>>>,
    driver: RefCell<Option<AbortOnDropHandle<()>>>,
}

impl SessionShared {
    /// Changes the state in one synchronous step, then does what the change
    /// asked for: wakes the worker, the persister or the wakeup driver,
    /// publishes the session's status, and delivers the change's events.
    fn update<R>(&self, change: impl FnOnce(&mut SessionCore) -> R) -> R {
        let (result, effects) = {
            let mut core = self.core.borrow_mut();
            let result = change(&mut core);
            (result, core.take_effects())
        };
        if effects.wake_worker {
            self.work.notify_one();
        }
        if effects.save {
            self.persist_wake.notify_one();
        }
        if effects.replan_wakeups {
            self.replan.notify_one();
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

    /// Puts a run's runtime back in its slot.
    fn return_runtime(&self, runtime: Box<dyn ProviderRuntime>) {
        self.update(|core| core.provider = Some(runtime));
        self.runtime_returned.notify_waiters();
    }
}

/// A handle over one agent's session; clones share it.
#[derive(Clone)]
pub struct AgentSession {
    shared: Rc<SessionShared>,
}

impl AgentSession {
    /// A new session with an empty transcript; its node's first checkpoint is
    /// already in the store.
    pub(crate) fn create(init: SessionInit, deps: SessionDeps) -> Self {
        let transcript = TranscriptLog::new(Vec::new(), deps.ids.clone(), deps.clock.clone());
        let parts = CoreParts {
            id: init.id,
            cwd: init.cwd,
            harness: deps.runtime.harness_name().to_owned(),
            model: init.model,
            provider: init.runtime,
            transcript,
            commands: CommandStateHistory::new(),
            inputs: InputQueue::default(),
            wakeups: Wakeups::default(),
            edits: Vec::new(),
            held: false,
            ids: deps.ids.clone(),
            clock: deps.clock.clone(),
        };
        Self::start(SessionCore::new(parts), deps)
    }

    /// A session from its checkpoint (`runtime.md` § Restoring). A tool call
    /// still marked executing completes as interrupted and never runs again;
    /// the session is idle, with its wakeups armed, and hands back its queue
    /// and whether a turn was interrupted. Waiting input and due wakeups of an
    /// interrupted turn wait for the node's next action.
    pub(crate) fn restore(
        checkpoint: Checkpoint,
        id: NodeId,
        runtime: Box<dyn ProviderRuntime>,
        deps: SessionDeps,
    ) -> Result<(Self, Continuation), RestoreError> {
        let Checkpoint {
            state,
            transcript,
            command_state,
        } = checkpoint;
        let expected = deps.runtime.harness_name();
        if state.harness != expected {
            return Err(RestoreError::Harness {
                stored: state.harness,
                expected: expected.to_owned(),
            });
        }
        check_restored_input(&id, &transcript, &state.agent_inputs, &state.wakeups)?;
        let mut operations = HashSet::new();
        if let Some(receipt) = state
            .edits
            .iter()
            .find(|receipt| !operations.insert(receipt.operation_id.as_str()))
        {
            return Err(RestoreError::Edits(receipt.operation_id.to_string()));
        }
        let commands = CommandStateHistory::restore(command_state)?;
        let mut transcript = TranscriptLog::new(transcript, deps.ids.clone(), deps.clock.clone());
        for call in transcript.pending_tool_calls() {
            let text = format!(
                "Tool call interrupted: {} (the process died before a result was recorded)",
                call.tool_name
            );
            transcript.complete_tool_call(
                &call.tool_use_id,
                vec![ToolResultContentBlock::Text { text }],
                true,
                None,
            );
        }
        let interrupted = state.phase != SessionPhase::Idle;
        let mut wakeups = Wakeups::restored(state.wakeups);
        // A wakeup whose action the process died in starts its wait now.
        wakeups.arm(deps.clock.now());
        let parts = CoreParts {
            id,
            cwd: state.cwd,
            harness: state.harness,
            model: state.model,
            provider: runtime,
            transcript,
            commands,
            inputs: InputQueue::restored(state.agent_inputs),
            wakeups,
            edits: state.edits,
            held: interrupted,
            ids: deps.ids.clone(),
            clock: deps.clock.clone(),
        };
        let continuation = Continuation {
            interrupted,
            queued: state.queue,
        };
        let session = Self::start(SessionCore::new(parts), deps);
        // The rows the store holds are current; the completed calls mark
        // theirs for the next save.
        session.shared.update(SessionCore::commit);
        Ok((session, continuation))
    }

    fn start(core: SessionCore, deps: SessionDeps) -> Self {
        let status = core.status();
        let shared = Rc::new(SessionShared {
            core: RefCell::new(core),
            bus: EventBus::default(),
            persist_gate: SerialGate::new(),
            status: watch::Sender::new(status),
            work: Rc::new(Notify::new()),
            persist_wake: Rc::new(Notify::new()),
            replan: Rc::new(Notify::new()),
            runtime_returned: Notify::new(),
            runtime: deps.runtime,
            store: deps.store,
            ids: deps.ids,
            config: deps.config,
            worker: RefCell::new(None),
            persister: RefCell::new(None),
            driver: RefCell::new(None),
        });
        let worker =
            tokio::task::spawn_local(worker::run(Rc::downgrade(&shared), shared.work.clone()));
        let persister = tokio::task::spawn_local(persist::run(
            Rc::downgrade(&shared),
            shared.persist_wake.clone(),
            deps.config.persist_interval,
        ));
        let driver = tokio::task::spawn_local(wakeups::drive(
            Rc::downgrade(&shared),
            shared.replan.clone(),
            deps.clock,
        ));
        *shared.worker.borrow_mut() = Some(AbortOnDropHandle::new(worker));
        *shared.persister.borrow_mut() = Some(AbortOnDropHandle::new(persister));
        *shared.driver.borrow_mut() = Some(AbortOnDropHandle::new(driver));
        Self { shared }
    }

    /// A new node's first checkpoint: its state row and the empty initial
    /// command state, with no blocks.
    pub(crate) fn first_checkpoint(&self) -> CheckpointUpdate {
        self.shared.read(|core| CheckpointUpdate {
            state: core.checkpoint_state(),
            command_state: Some(core.commands.snapshot(None)),
            changed_blocks: Vec::new(),
            block_count: 0,
        })
    }

    pub fn id(&self) -> NodeId {
        self.shared.read(|core| core.id.clone())
    }

    pub fn phase(&self) -> SessionPhase {
        self.shared.read(SessionCore::phase)
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

    pub fn transcript(&self) -> TranscriptSnapshot {
        self.shared.read(|core| TranscriptSnapshot {
            blocks: core.transcript.blocks().to_vec(),
            version: core.transcript.version(),
        })
    }

    pub fn queued_messages(&self) -> Vec<QueuedMessage> {
        self.shared.read(SessionCore::queued_messages)
    }

    /// The human steers accepted and not yet written.
    pub fn pending_steers(&self) -> Vec<PendingSteer> {
        self.shared.read(SessionCore::pending_steers)
    }

    /// The text of the last `text` block, the result a child closes with.
    pub(crate) fn last_assistant_text(&self) -> String {
        self.shared
            .read(|core| last_assistant_text(core.transcript.blocks(), 0).to_owned())
    }

    /// Submits a message (`runtime.md` § Messages and the queue).
    pub(crate) fn send(
        &self,
        content: Vec<UserContentBlock>,
        id: TurnId,
    ) -> Result<ActionHandle, AdmissionError> {
        self.shared.update(|core| core.admit_send(id, content))
    }

    /// Rewinds the last input turn and runs it again
    /// (`failures-and-recovery.md` § Recovery is one mechanism).
    pub(crate) fn retry(&self) -> Result<ActionHandle, AdmissionError> {
        self.shared.update(|core| core.admit(ActionKind::Retry))
    }

    /// Unwinds the unfinished turn to its resume point and continues it.
    pub(crate) fn resume(&self) -> Result<ActionHandle, AdmissionError> {
        self.shared.update(|core| core.admit(ActionKind::Resume))
    }

    /// Runs one compaction pass (`compaction.md` § Compaction).
    pub(crate) fn compact(&self) -> Result<ActionHandle, AdmissionError> {
        self.shared.update(|core| core.admit(ActionKind::Compact))
    }

    pub(crate) fn dequeue_message(&self, id: &TurnId) -> bool {
        self.shared.update(|core| core.dequeue(id))
    }

    pub(crate) fn send_queued_message(&self, id: &TurnId) -> bool {
        self.shared.update(|core| core.send_next(id))
    }

    pub(crate) fn clear_message_queue(&self) -> usize {
        self.shared.update(SessionCore::clear_queue)
    }

    /// Adds input to the running turn at its next boundary
    /// (`runtime.md` § Steers).
    pub(crate) fn steer(
        &self,
        content: Vec<UserContentBlock>,
        id: BlockId,
    ) -> Result<(), SteerError> {
        self.shared.update(|core| core.steer(id, content))
    }

    /// Withdraws a pending steer; an id that is not pending changes nothing.
    pub(crate) fn cancel_pending_steer(&self, id: &BlockId) -> bool {
        self.shared.update(|core| core.cancel_steer(id))
    }

    /// Turns a queued message into a steer of the running turn; false when no
    /// queued message has the id.
    pub(crate) fn steer_queued_message(
        &self,
        message: &TurnId,
        steer: BlockId,
    ) -> Result<bool, SteerError> {
        self.shared.update(|core| core.steer_queued(message, steer))
    }

    /// Admits a message from another agent of the tree; it returns once the
    /// admission is saved (`subagents.md` § Delivery and scheduling).
    pub(crate) async fn accept_agent_message(
        &self,
        message: AgentMessage,
    ) -> Result<(), AgentMessageError> {
        if let Err(report) = garde::Validate::validate(&message) {
            return Err(AgentMessageError::Invalid(report.to_string()));
        }
        self.shared
            .update(|core| core.admit_agent_message(message))?;
        // A message admitted before is saved already, or its save is under
        // way ahead of this one; either way it is durable when this returns.
        persist::flush(&self.shared).await?;
        Ok(())
    }

    /// What the session knows of an edit request before its uploads are
    /// resolved (`message-editing.md` § Commit and idempotency), so that a
    /// repeated request writes no file.
    pub(crate) fn check_edit(
        &self,
        operation: &demi_core::OperationId,
        digest: &str,
        version: &TranscriptVersion,
    ) -> Result<EditCheck, EditError> {
        self.shared
            .read(|core| core.check_edit(operation, digest, version))
    }

    /// Replaces a user message and everything after it
    /// (`message-editing.md`); returns at durable acceptance, and the
    /// replacement's turn runs on. An operation accepted before returns its
    /// receipt again.
    pub(crate) async fn edit_and_send(
        &self,
        submission: EditSubmission,
    ) -> Result<crate::store::EditReceipt, EditError> {
        match self.shared.update(|core| core.admit_edit(submission))? {
            EditCheck::Accepted(receipt) => Ok(receipt),
            EditCheck::InFlight(acceptance) => accepted(acceptance).await,
            EditCheck::Proceed => unreachable!("an admitted edit is in flight"),
        }
    }

    /// A Fork's seed from this live session, through the completed text
    /// `target` (`conversation-fork.md` § The fork seed): captured in one
    /// step, with the model a switch the session accepted would use.
    pub(crate) fn prepare_fork(&self, target: &BlockId) -> Result<Checkpoint, ForkError> {
        self.shared.read(|core| {
            let state = crate::store::CheckpointState {
                model: core.latest_selection().clone(),
                ..core.checkpoint_state()
            };
            fork_seed(core.transcript.blocks(), &core.commands, state, target)
        })
    }

    /// Opens a continuation for input that waits, when nothing runs: the
    /// node's policy calls it once a restored session may act.
    pub(crate) fn wake(&self) {
        self.shared.update(SessionCore::wake);
    }

    /// Stops one thing (`runtime.md` § Stop): the running action, which has
    /// recorded the stop when this returns; else the first waiting action;
    /// else the oldest scheduled wakeup. Whether another `abort` would stop
    /// more is read when the stop is recorded.
    pub(crate) async fn abort(&self) -> AbortResult {
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

    /// Records a model switch; it lands at the next action, or also at the
    /// next continuation boundary when it is immediate. A runtime the switch
    /// replaces is closed once it serves no run.
    pub(crate) fn update_model(&self, switch: ModelSwitch) -> Result<(), AdmissionError> {
        self.shared.update(|core| core.record_switch(switch))
    }

    /// Whether `model` needs a runtime of its own: it belongs to another
    /// provider than the selection the next request would use.
    pub(crate) fn needs_runtime_for(&self, model: &ModelSelection) -> bool {
        self.shared
            .read(|core| core.latest_selection().provider_id != model.provider_id)
    }

    /// A runtime fork of the session's current runtime: the same provider
    /// and configuration, none of its execution state. A run in progress
    /// holds the runtime, so the fork waits until it is back.
    pub(crate) async fn fork_runtime(&self) -> Result<Box<dyn ProviderRuntime>, AdmissionError> {
        loop {
            let returned = self.shared.runtime_returned.notified();
            tokio::pin!(returned);
            returned.as_mut().enable();
            let fork = self.shared.read(|core| {
                if core.disposing {
                    return Some(Err(AdmissionError::Closed));
                }
                core.provider.as_ref().map(|runtime| Ok(runtime.fresh()))
            });
            if let Some(fork) = fork {
                return fork;
            }
            returned.await;
        }
    }

    /// Appends the interruption record of a turn the process died in, unless
    /// the transcript ends with one; [`flush`](Self::flush) saves it.
    pub(crate) fn record_interruption(&self) {
        self.shared.update(SessionCore::record_interruption);
    }

    /// Saves now, after the saves ahead in the session's order.
    pub(crate) async fn flush(&self) -> Result<(), StoreError> {
        persist::flush(&self.shared).await
    }

    /// Disposes the session (`runtime.md` § Dispose and restore): refuses
    /// new actions, stops a running one as a shutdown, keeps the queue and
    /// the wakeups, saves the final checkpoint and closes the provider
    /// runtimes. The error is a final checkpoint that could not be saved.
    pub(crate) async fn dispose(&self) -> Result<(), StoreError> {
        let first = self.shared.update(SessionCore::begin_dispose);
        let mut status = self.shared.status.subscribe();
        // The sender lives in the session this handle keeps alive.
        let _ = status
            .wait_for(|status| status.settle == Settle::Closed)
            .await;
        if !first {
            return Ok(());
        }
        // A fork waiting for the runtime learns the session is closing.
        self.shared.runtime_returned.notify_waiters();
        let saved = {
            let _turn = self.shared.persist_gate.acquire().await;
            // With the order held, the persister is between saves.
            self.shared.persister.borrow_mut().take();
            self.shared.driver.borrow_mut().take();
            persist::write_if_dirty(&self.shared, None, &Default::default()).await
        };
        let runtimes = self.shared.update(SessionCore::take_runtimes);
        for mut runtime in runtimes {
            runtime.close().await;
        }
        self.shared.runtime.dispose().await;
        saved
    }

    /// Calls `listener` with every event from now on, until the subscription
    /// is dropped.
    pub(crate) fn subscribe(&self, listener: impl FnMut(&SessionEvent) + 'static) -> Subscription {
        let id = self.shared.bus.subscribe(Box::new(listener));
        Subscription {
            session: Rc::downgrade(&self.shared),
            id,
        }
    }

    pub(crate) fn status_watch(&self) -> watch::Receiver<Status> {
        self.shared.status.subscribe()
    }

    pub(crate) fn status(&self) -> Status {
        *self.shared.status.borrow()
    }

    /// What the session is doing now, as a supervisor observes it.
    pub(crate) fn execution(&self) -> Execution {
        self.shared.read(SessionCore::execution)
    }

    /// Whether the block at `index` is assistant text.
    pub(crate) fn is_text_at(&self, index: usize) -> bool {
        self.shared
            .read(|core| matches!(core.transcript.blocks().get(index), Some(Block::Text(_))))
    }
}

/// The session's own checks of a checkpoint's waiting input: each agent
/// message is addressed to this node, its id is unique and not yet in the
/// transcript, and each wakeup id is unique.
fn check_restored_input(
    node: &NodeId,
    blocks: &[Block],
    agent_inputs: &[crate::store::PendingAgentInput],
    wakeups: &[crate::store::ScheduledWakeup],
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
    let mut wakeup_ids = HashSet::new();
    if let Some(wakeup) = wakeups
        .iter()
        .find(|wakeup| !wakeup_ids.insert(wakeup.id.as_str()))
    {
        return Err(RestoreError::Input(format!(
            "wakeup {} is scheduled twice",
            wakeup.id
        )));
    }
    Ok(())
}
