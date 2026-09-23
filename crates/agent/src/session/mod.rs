//! One agent's session (`runtime.md` § Sessions and turns): a cloneable
//! handle over the session's state, which one worker task changes by running
//! one action at a time while commands from the connection change it between
//! the worker's awaits. The worker, the persister and every command change
//! the state only through [`SessionShared::update`], whose synchronous
//! closure ends before any await, and each change's events reach the
//! listeners after the state is released.

mod bus;
mod cancel;
mod core;
mod persist;
mod runtime;
mod turn;
mod worker;

#[cfg(test)]
mod tests;

use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    rc::{Rc, Weak},
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use demi_agent_protocol::{AbortResult, ModelSwitchApply, TranscriptPatch, TranscriptVersion};
use demi_core::{
    Block, Clock, ModelSelection, NodeId, ProviderErrorDiagnostics, QueuedMessage, SessionPhase,
    ToolResultContentBlock, TurnId, UserContentBlock,
};
use demi_gates::SerialGate;
use demi_provider::{ProviderFailure, ProviderRuntime};
use tokio::sync::{Notify, oneshot, watch};
use tokio_util::task::AbortOnDropHandle;

pub(crate) use runtime::{SessionRuntime, ToolFailure, ToolInvocation, ToolOutcome};

use self::{
    bus::EventBus,
    core::{AbortStep, SessionCore},
};
use crate::{
    IdSource,
    store::{
        Checkpoint, CheckpointUpdate, CommandStateError, CommandStateHistory, SessionStore,
        StoreError,
    },
    transcript::TranscriptLog,
};

/// How a session saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// How long after the first unsaved change a save starts.
    pub persist_interval: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            persist_interval: Duration::from_secs(1),
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
    /// It left the queue without running: dequeued, cleared or stopped.
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
}

/// Why a checkpoint could not be restored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RestoreError {
    #[error("The checkpoint's harness \"{stored}\" is not \"{expected}\"")]
    Harness { stored: String, expected: String },
    #[error(transparent)]
    CommandState(#[from] CommandStateError),
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
    /// The provider's run failed.
    Provider(Box<ProviderFailure>),
    /// A save the turn waited for failed.
    Store(StoreError),
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
    settle: watch::Sender<Settle>,
    /// Wakes the worker when an action starts.
    work: Rc<Notify>,
    /// Wakes the persister when a change makes a save due.
    persist_wake: Rc<Notify>,
    runtime: Rc<dyn SessionRuntime>,
    store: Rc<dyn SessionStore>,
    ids: Rc<dyn IdSource>,
    worker: RefCell<Option<AbortOnDropHandle<()>>>,
    persister: RefCell<Option<AbortOnDropHandle<()>>>,
}

impl SessionShared {
    /// Changes the state in one synchronous step, then does what the change
    /// asked for: wakes the worker or the persister, publishes whether the
    /// session settled, and delivers the change's events.
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
        self.settle.send_if_modified(|settle| {
            let changed = *settle != effects.settle;
            *settle = effects.settle;
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
        let core = SessionCore::new(
            init.id,
            init.cwd,
            deps.runtime.harness_name().to_owned(),
            init.model,
            init.runtime,
            transcript,
            CommandStateHistory::new(),
        );
        Self::start(core, deps)
    }

    /// A session from its checkpoint (`runtime.md` § Restoring). A tool call
    /// still marked executing completes as interrupted and never runs again;
    /// the session is idle and hands back its queue and whether a turn was
    /// interrupted.
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
        let continuation = Continuation {
            interrupted: state.phase != SessionPhase::Idle,
            queued: state.queue,
        };
        let core = SessionCore::new(
            id,
            state.cwd,
            state.harness,
            state.model,
            runtime,
            transcript,
            commands,
        );
        let session = Self::start(core, deps);
        // The rows the store holds are current; the completed calls mark
        // theirs for the next save.
        session.shared.update(SessionCore::commit);
        Ok((session, continuation))
    }

    fn start(core: SessionCore, deps: SessionDeps) -> Self {
        let shared = Rc::new(SessionShared {
            core: RefCell::new(core),
            bus: EventBus::default(),
            persist_gate: SerialGate::new(),
            settle: watch::Sender::new(Settle::Settled),
            work: Rc::new(Notify::new()),
            persist_wake: Rc::new(Notify::new()),
            runtime: deps.runtime,
            store: deps.store,
            ids: deps.ids,
            worker: RefCell::new(None),
            persister: RefCell::new(None),
        });
        let worker =
            tokio::task::spawn_local(worker::run(Rc::downgrade(&shared), shared.work.clone()));
        let persister = tokio::task::spawn_local(persist::run(
            Rc::downgrade(&shared),
            shared.persist_wake.clone(),
            deps.config.persist_interval,
        ));
        *shared.worker.borrow_mut() = Some(AbortOnDropHandle::new(worker));
        *shared.persister.borrow_mut() = Some(AbortOnDropHandle::new(persister));
        Self { shared }
    }

    /// A new node's first checkpoint: its state row and the empty initial
    /// command state, with no blocks.
    pub(crate) fn first_checkpoint(&self) -> CheckpointUpdate {
        self.shared.read(|core| CheckpointUpdate {
            state: core.checkpoint_state(),
            command_state: Some(core.commands.snapshot()),
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
        let mut settle = self.shared.settle.subscribe();
        // The sender lives in the session this handle keeps alive.
        let _ = settle.wait_for(|settle| *settle != Settle::Busy).await;
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

    /// Submits a message (`runtime.md` § Messages and the queue).
    pub(crate) fn send(
        &self,
        content: Vec<UserContentBlock>,
        id: TurnId,
    ) -> Result<ActionHandle, AdmissionError> {
        self.shared.update(|core| core.admit_send(id, content))
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

    /// Stops one thing (`runtime.md` § Stop): the running action, which has
    /// recorded the stop when this returns; else the first waiting action.
    /// Whether another `abort` would stop more is read when the stop is
    /// recorded.
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
    /// new actions, stops a running one as a shutdown, keeps the queue, saves
    /// the final checkpoint and closes the provider runtimes. The error is a
    /// final checkpoint that could not be saved.
    pub(crate) async fn dispose(&self) -> Result<(), StoreError> {
        let first = self.shared.update(SessionCore::begin_dispose);
        let mut settle = self.shared.settle.subscribe();
        // The sender lives in the session this handle keeps alive.
        let _ = settle.wait_for(|settle| *settle == Settle::Closed).await;
        if !first {
            return Ok(());
        }
        let saved = {
            let _turn = self.shared.persist_gate.acquire().await;
            // With the order held, the persister is between saves.
            self.shared.persister.borrow_mut().take();
            persist::write_if_dirty(&self.shared, &Default::default()).await
        };
        let runtimes = self.shared.update(SessionCore::take_runtimes);
        for mut runtime in runtimes {
            runtime.close().await;
        }
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

    pub(crate) fn settle_watch(&self) -> watch::Receiver<Settle> {
        self.shared.settle.subscribe()
    }
}
