//! A session's state and every decision about it (`runtime.md` § Sessions
//! and turns): one status, the actions waiting to run, the input waiting for
//! a boundary, the scheduled wakeups, the transcript and command state, the
//! model selection with its provider runtime, and what the next save writes.
//! Every method here is synchronous; the worker and the turn loop await
//! between calls, never inside one.

use std::{collections::VecDeque, mem, num::NonZeroU32, rc::Rc, sync::Arc};

use demi_agent_protocol::{AbortTarget, ModelSwitchApply, TranscriptPatch};
use demi_core::{
    AgentMessage, Block, BlockId, Clock, FailureSource, ModelSelection, NodeId, PendingSteer,
    ProviderErrorDiagnostics, QueuedMessage, SessionPhase, ToolResultContentBlock, TurnId,
    UserContentBlock, WakeupId, WakeupPlacement,
};
use demi_provider::{
    InferenceRequest, ProviderEvent, ProviderFailure, ProviderRuntime, ToolDefinition,
};
use tokio_util::sync::CancellationToken;

use super::{
    ActionEnd, ActionHandle, ActionReply, AdmissionError, AgentMessageError, ErrorReport,
    Execution, ModelSwitch, SessionEvent, Settle, Status, SteerError,
    cancel::{CancelReason, TurnCancel},
    editing::{EditCheck, EditError, EditInFlight, EditSubmission},
    input::{Input, InputQueue, Take, Wakeups},
    persist::{PersistMarks, TakenMarks},
    runtime::ToolOutcome,
    storage::Generation,
};
use crate::{
    IdSource,
    store::{
        BoundaryEdge, CheckpointState, CheckpointUpdate, CommandStateHistory, CommandStateSnapshot,
        CommandStorageKey, CommandVersion, EditReceipt, PendingAgentInput, ScheduledWakeup,
    },
    transcript::{
        INTERRUPTED_CODE, INTERRUPTED_TURN_MESSAGE, TranscriptLog, opens_input_turn, replay,
    },
};

pub(crate) struct SessionCore {
    pub(super) id: NodeId,
    pub(super) cwd: String,
    pub(super) harness: String,
    /// The selection current now; every block records the one current when
    /// it was written.
    pub(super) model: ModelSelection,
    /// The runtime that serves requests; empty while a run holds it.
    pub(super) provider: Option<Box<dyn ProviderRuntime>>,
    /// A recorded model switch that has not landed yet.
    pub(super) switch: Option<ModelSwitch>,
    /// Runtimes a later switch replaced before they served a request, closed
    /// when the next switch lands or at dispose.
    pub(super) retired: Vec<Box<dyn ProviderRuntime>>,
    pub(super) transcript: TranscriptLog,
    pub(super) commands: CommandStateHistory,
    /// The command-storage generation of the jobs started now: a history
    /// rewrite or dispose cancels it, so that a storage message of an older
    /// job never commits into the history that replaced its own.
    pub(super) generation: Generation,
    /// The actions waiting, in the order they run; the queue is its sends.
    pub(super) pending: VecDeque<PendingAction>,
    pub(super) inputs: InputQueue,
    pub(super) wakeups: Wakeups,
    pub(super) edits: Vec<EditReceipt>,
    /// The edit being prepared or run, from its admission until its action
    /// ends.
    pub(super) editing: Option<EditInFlight>,
    pub(super) activity: Activity,
    /// The last turn was interrupted and the node has not acted since, as a
    /// restored root whose turn the process died in: waiting input and due
    /// wakeups wait for the user's next action.
    pub(super) held: bool,
    /// Dispose started: every admission is refused.
    pub(super) disposing: bool,
    pub(super) persist: PersistMarks,
    /// Events of the current change, delivered once it is complete.
    pub(super) outbox: Vec<SessionEvent>,
    ids: Rc<dyn IdSource>,
    clock: Arc<dyn Clock>,
    requests: Requests,
    published: Published,
}

/// What the session is doing.
pub(super) enum Activity {
    Idle,
    Running(ActionRun),
    /// The action ended and its closing save is in progress. The save
    /// records the session idle, while clients see it running until that
    /// save has committed (`runtime.md` § A turn).
    Finishing,
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
    /// The transcript's revision when the action started: an action that
    /// wrote nothing since has not begun.
    started_at: u64,
    /// Taken by the worker when it starts the action.
    start: Option<StartedAction>,
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

pub(super) enum ActionKind {
    /// A message: appends a `user` block and runs a turn.
    Send {
        content: Vec<UserContentBlock>,
    },
    /// Input that arrived while nothing ran, a fired yield wakeup or agent
    /// messages: it opens a turn of its own, never shown in the queue.
    Continue,
    Retry,
    Resume,
    Compact,
    /// An admitted edit, whose submission waits in the session's edit state.
    Edit,
}

/// What one `abort` found to stop.
pub(super) enum AbortStep {
    /// The running action, which records the stop; `abort` waits for that.
    Running {
        target: AbortTarget,
        cancel: Rc<TurnCancel>,
    },
    /// Something that had not run yet: a waiting action or a scheduled
    /// wakeup.
    Removed(AbortTarget),
    Nothing,
}

/// Where a recorded model switch may land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SwitchPoint {
    /// The start of an action: every switch lands.
    ActionStart,
    /// A continuation boundary inside a turn: an immediate switch lands.
    Continuation,
}

/// What the wrapper does once a change is complete.
#[derive(Default)]
struct Requests {
    wake_worker: bool,
    save: bool,
}

/// The derived values listeners and the save were last told about.
struct Published {
    queue: Vec<TurnId>,
    phase: SessionPhase,
    pending_steers: Vec<PendingSteer>,
    agent_inputs: Vec<BlockId>,
    wakeups: Vec<ScheduledWakeup>,
    edits: usize,
}

/// What a complete change leaves to do after the core is released.
pub(super) struct Effects {
    pub(super) events: Vec<SessionEvent>,
    pub(super) wake_worker: bool,
    pub(super) save: bool,
    /// The scheduled wakeups changed: the wakeup driver plans again.
    pub(super) replan_wakeups: bool,
    pub(super) status: Status,
}

/// What a new or restored session starts with.
pub(super) struct CoreParts {
    pub(super) id: NodeId,
    pub(super) cwd: String,
    pub(super) harness: String,
    pub(super) model: ModelSelection,
    pub(super) provider: Box<dyn ProviderRuntime>,
    pub(super) transcript: TranscriptLog,
    pub(super) commands: CommandStateHistory,
    pub(super) inputs: InputQueue,
    pub(super) wakeups: Wakeups,
    pub(super) edits: Vec<EditReceipt>,
    pub(super) held: bool,
    pub(super) ids: Rc<dyn IdSource>,
    pub(super) clock: Arc<dyn Clock>,
}

impl SessionCore {
    pub(super) fn new(parts: CoreParts) -> Self {
        let mut core = Self {
            id: parts.id,
            cwd: parts.cwd,
            harness: parts.harness,
            model: parts.model,
            provider: Some(parts.provider),
            switch: None,
            retired: Vec::new(),
            transcript: parts.transcript,
            commands: parts.commands,
            generation: Generation::first(),
            pending: VecDeque::new(),
            inputs: parts.inputs,
            wakeups: parts.wakeups,
            edits: parts.edits,
            editing: None,
            activity: Activity::Idle,
            held: parts.held,
            disposing: false,
            persist: PersistMarks::default(),
            outbox: Vec::new(),
            ids: parts.ids,
            clock: parts.clock,
            requests: Requests::default(),
            published: Published {
                queue: Vec::new(),
                phase: SessionPhase::Idle,
                pending_steers: Vec::new(),
                agent_inputs: Vec::new(),
                wakeups: Vec::new(),
                edits: 0,
            },
        };
        // What the checkpoint holds counts as published: only later changes
        // make a save due.
        core.published.agent_inputs = core.agent_input_ids();
        core.published.wakeups = core.checkpoint_wakeups();
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
    /// digest matches; otherwise the edit needs a settled session with
    /// nothing waiting and a current snapshot.
    pub(super) fn check_edit(
        &self,
        operation: &demi_core::OperationId,
        digest: &str,
        version: &demi_agent_protocol::TranscriptVersion,
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
        if self.disposing {
            return Err(EditError::Closed);
        }
        let waiting = !self.wakeups.is_empty()
            || self.inputs.has_agent_input()
            || self.inputs.has_fired_wakeup()
            || !self.inputs.pending_steers().is_empty();
        if self.settle() != Settle::Settled || self.editing.is_some() || waiting {
            return Err(EditError::Busy);
        }
        if version != &self.transcript.version() {
            return Err(EditError::Stale);
        }
        Ok(EditCheck::Proceed)
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
    /// command state before the target, the receipt, the model and the
    /// runtime fork the replacement runs on. Returns the runtimes it
    /// discarded, for closing.
    pub(super) fn adopt_edit(
        &mut self,
        blocks: Vec<Block>,
        commands: CommandStateHistory,
        runtime: Box<dyn ProviderRuntime>,
        model: ModelSelection,
        receipt: EditReceipt,
    ) -> Vec<Box<dyn ProviderRuntime>> {
        self.adopt_rewrite(blocks, commands);
        self.edits.push(receipt.clone());
        let mut discarded: Vec<_> = self.provider.replace(runtime).into_iter().collect();
        discarded.extend(self.switch.take().and_then(|switch| switch.runtime));
        discarded.append(&mut self.retired);
        self.model = model;
        if let Some(edit) = &mut self.editing {
            edit.accepted = true;
            edit.resolve(Ok(receipt));
        }
        discarded
    }

    /// Rejects the edit being prepared: nothing of the history changed.
    pub(super) fn reject_edit(&mut self, error: EditError) {
        if let Some(edit) = self.editing.take() {
            edit.resolve(Err(error));
        }
    }

    // Command storage.

    /// The current version's value of `key`, and the node's revision.
    pub(super) fn storage_value(
        &self,
        key: &CommandStorageKey,
    ) -> (Option<serde_json::Value>, u64) {
        (
            self.commands.values().get(key).cloned(),
            self.commands.revision(),
        )
    }

    /// The current version's keys that start with `prefix`, sorted.
    pub(super) fn storage_keys(&self, prefix: &str) -> Vec<String> {
        self.commands
            .values()
            .keys()
            .map(|key| key.as_str())
            .filter(|key| key.starts_with(prefix))
            .map(str::to_owned)
            .collect()
    }

    /// The version a write makes: `key` set to `value`, or removed; none
    /// when the values would not change.
    pub(super) fn storage_write(
        &self,
        key: CommandStorageKey,
        value: Option<serde_json::Value>,
    ) -> Option<CommandVersion> {
        let mut values = self.commands.values().clone();
        match value {
            Some(value) => values.insert(key, value),
            None => values.remove(&key),
        };
        self.commands.prepare(values)
    }

    pub(super) fn accept_storage_version(&mut self, version: CommandVersion) {
        self.commands.accept(version);
    }

    /// Admits a message: it starts at once when nothing runs and waits in the
    /// queue otherwise. A message whose id the session already knows is
    /// acknowledged without a second turn.
    pub(super) fn admit_send(
        &mut self,
        id: TurnId,
        content: Vec<UserContentBlock>,
    ) -> Result<ActionHandle, AdmissionError> {
        self.refuse_admission()?;
        if self.knows_turn(&id) {
            return Ok(ActionHandle::ended(ActionEnd::Duplicate));
        }
        Ok(self.push_action(ActionKind::Send { content }, id))
    }

    /// Admits a retry, a resume or a compaction pass.
    pub(super) fn admit(&mut self, kind: ActionKind) -> Result<ActionHandle, AdmissionError> {
        self.refuse_admission()?;
        let turn = self.new_turn();
        Ok(self.push_action(kind, turn))
    }

    fn push_action(&mut self, kind: ActionKind, turn: TurnId) -> ActionHandle {
        let (reply, handle) = ActionHandle::channel();
        self.pending.push_back(PendingAction {
            kind,
            turn,
            reply: Some(reply),
        });
        if matches!(self.activity, Activity::Idle) {
            self.start_next();
        }
        handle
    }

    /// Whether `id` is the running turn, a queued message or the turn of a
    /// `user` block.
    fn knows_turn(&self, id: &TurnId) -> bool {
        let running = matches!(&self.activity, Activity::Running(run) if &run.turn == id);
        running
            || self
                .pending
                .iter()
                .any(|action| action.is_send() && &action.turn == id)
            || self.transcript.has_user_turn(id)
    }

    /// Starts the first waiting action, or a continuation for input that
    /// waits, or settles: idle, or closed once dispose began.
    pub(super) fn start_next(&mut self) {
        if self.disposing {
            self.activity = Activity::Closed { interrupted: false };
            return;
        }
        let (kind, turn, reply) = match self.pending.pop_front() {
            Some(action) => (action.kind, action.turn, action.reply),
            None if self.wants_continuation() => (ActionKind::Continue, self.new_turn(), None),
            None => {
                self.activity = Activity::Idle;
                return;
            }
        };
        let cancel = TurnCancel::new();
        self.held = false;
        self.activity = Activity::Running(ActionRun {
            turn: turn.clone(),
            cancel: cancel.clone(),
            stage: TurnStage::Preparing,
            started_at: self.transcript.version().revision,
            start: Some(StartedAction {
                kind,
                turn,
                cancel,
                reply,
            }),
        });
        self.requests.wake_worker = true;
    }

    /// Whether waiting input would open a continuation of its own: a fired
    /// wakeup, or agent messages unless the user stopped the last turn. A
    /// held session waits for the user.
    fn wants_continuation(&self) -> bool {
        if self.held {
            return false;
        }
        let stopped = matches!(self.transcript.blocks().last(), Some(Block::Abort(_)));
        self.inputs.has_fired_wakeup() || (self.inputs.has_agent_input() && !stopped)
    }

    /// Opens a continuation when waiting input wants one and nothing runs or
    /// waits.
    pub(super) fn wake(&mut self) {
        if matches!(self.activity, Activity::Idle) && self.pending.is_empty() {
            self.start_next();
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
        match &self.activity {
            Activity::Running(run) => run.turn.clone(),
            _ => unreachable!("only a running action writes a turn"),
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
    /// still pending are dropped, and the wakeups it scheduled start their
    /// wait.
    pub(super) fn end_action(&mut self) {
        self.activity = Activity::Finishing;
        self.editing = None;
        self.inputs.discard_steers();
        self.wakeups.arm(self.clock.now());
    }

    /// Dispose stopped the running action, and the worker ends with it. An
    /// action that wrote into the transcript is recorded as interrupted, so
    /// the final checkpoint says it was running; a message whose turn wrote
    /// nothing yet goes back to the front of the queue instead, and another
    /// action that wrote nothing ends with the session. Returns whether a
    /// turn was interrupted.
    pub(super) fn shut_down(&mut self, kind: ActionKind, turn: &TurnId) -> bool {
        let began = matches!(
            &self.activity,
            Activity::Running(run) if run.started_at != self.transcript.version().revision
        );
        self.reject_edit(EditError::Closed);
        if began {
            self.record_stop(CancelReason::Shutdown);
        } else if let ActionKind::Send { content } = kind {
            self.pending.push_front(PendingAction {
                kind: ActionKind::Send { content },
                turn: turn.clone(),
                reply: None,
            });
        }
        self.wakeups.arm(self.clock.now());
        self.activity = Activity::Closed { interrupted: began };
        began
    }

    // Stop and dispose.

    /// Finds the one thing an `abort` stops: the running action, else the
    /// first waiting action, else the oldest scheduled wakeup. A session
    /// being disposed stops nothing more.
    pub(super) fn abort_step(&mut self) -> AbortStep {
        if self.disposing {
            return AbortStep::Nothing;
        }
        if let Activity::Running(run) = &self.activity
            && !run.cancel.is_cancelled()
        {
            let target = match run.stage {
                TurnStage::Preparing => AbortTarget::ActiveTurn,
                TurnStage::Streaming => AbortTarget::ActiveProviderStream,
                TurnStage::Tools => AbortTarget::ActiveTool,
                TurnStage::Compacting => AbortTarget::ActiveCompaction,
            };
            run.cancel.cancel(CancelReason::Stop);
            return AbortStep::Running {
                target,
                cancel: run.cancel.clone(),
            };
        }
        if let Some(mut action) = self.pending.pop_front() {
            action.end(ActionEnd::Dropped);
            let target = match action.kind {
                ActionKind::Send { .. } => AbortTarget::QueuedMessage,
                _ => AbortTarget::QueuedAction,
            };
            return AbortStep::Removed(target);
        }
        if self.wakeups.cancel_oldest() {
            return AbortStep::Removed(AbortTarget::PendingYieldWakeup);
        }
        AbortStep::Nothing
    }

    /// Stops the running action, as the user's Stop would, and nothing
    /// that waits: a waiting action and a scheduled wakeup stay. The answer
    /// is the stopped action's token, which says when it recorded the stop.
    pub(super) fn stop_running(&mut self) -> Option<Rc<TurnCancel>> {
        if self.disposing {
            return None;
        }
        match &self.activity {
            Activity::Running(run) if !run.cancel.is_cancelled() => {
                run.cancel.cancel(CancelReason::Stop);
                Some(run.cancel.clone())
            }
            _ => None,
        }
    }

    /// Whether another `abort` would stop something.
    pub(super) fn can_abort_again(&self) -> bool {
        let running =
            matches!(&self.activity, Activity::Running(run) if !run.cancel.is_cancelled());
        !self.disposing && (running || !self.pending.is_empty() || !self.wakeups.is_empty())
    }

    /// A stopped action records the stop: it writes the human steers still
    /// pending and, for the user's Stop, the wakeups that fired, then
    /// completes running calls as aborted and appends the stopped marker, or
    /// the interruption record when the session is shutting down.
    pub(super) fn record_stop(&mut self, reason: CancelReason) {
        let take = match reason {
            CancelReason::Stop => Take::AllButAgentMessages,
            CancelReason::Shutdown => Take::Steers,
        };
        self.write_inputs(take);
        self.abort_executing_calls();
        match reason {
            CancelReason::Stop => self.transcript.push_abort(&self.model),
            CancelReason::Shutdown => self.append_interruption(),
        }
        self.commit();
    }

    /// A failed action writes all waiting input and completes the calls it
    /// did not run, then reports the failure.
    pub(super) fn fail(&mut self, report: &ErrorReport) {
        self.write_inputs(Take::Everything);
        self.abort_executing_calls();
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
        );
    }

    fn abort_executing_calls(&mut self) {
        for call in self.transcript.pending_tool_calls() {
            let text = format!("Tool call aborted: {}", call.tool_name);
            self.transcript.complete_tool_call(
                &call.tool_use_id,
                vec![ToolResultContentBlock::Text { text }],
                true,
                None,
            );
        }
    }

    /// Starts dispose: admissions end, command-storage handles lapse, the
    /// running action is stopped as a shutdown, and queued messages lose
    /// their callers but stay for the final checkpoint. Returns false when
    /// dispose already started.
    pub(super) fn begin_dispose(&mut self) -> bool {
        if self.disposing {
            return false;
        }
        self.disposing = true;
        self.generation.token.cancel();
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
        true
    }

    /// Every provider runtime the session holds, for closing.
    pub(super) fn take_runtimes(&mut self) -> Vec<Box<dyn ProviderRuntime>> {
        let mut runtimes: Vec<_> = self.provider.take().into_iter().collect();
        runtimes.extend(self.switch.take().and_then(|switch| switch.runtime));
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
        action.end(ActionEnd::Dropped);
        true
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

    pub(super) fn clear_queue(&mut self) -> usize {
        if self.disposing {
            return 0;
        }
        let mut cleared = 0;
        self.pending.retain_mut(|action| {
            if !action.is_send() {
                return true;
            }
            action.end(ActionEnd::Dropped);
            cleared += 1;
            false
        });
        cleared
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
                ActionKind::Send { content } => Some(QueuedMessage {
                    id: action.turn.clone(),
                    content: content.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    // Steers and agent messages.

    /// Accepts a human steer for the running action's next boundary.
    pub(super) fn steer(
        &mut self,
        id: BlockId,
        content: Vec<UserContentBlock>,
    ) -> Result<(), SteerError> {
        let turn = self.steerable_turn()?;
        self.inputs.add(Input::Steer(PendingSteer {
            id,
            turn_id: turn,
            model: self.model.clone(),
            content,
        }));
        Ok(())
    }

    /// The turn a steer joins now: the running action's, unless it was
    /// stopped or nothing runs.
    fn steerable_turn(&self) -> Result<TurnId, SteerError> {
        if self.preparing_edit() {
            return Err(SteerError::Editing);
        }
        match &self.activity {
            Activity::Running(run) if run.cancel.is_cancelled() => Err(SteerError::Stopped),
            Activity::Running(run) => Ok(run.turn.clone()),
            Activity::Finishing => Err(SteerError::Finishing),
            Activity::Idle | Activity::Closed { .. } => Err(SteerError::NotRunning),
        }
    }

    pub(super) fn cancel_steer(&mut self, id: &BlockId) -> bool {
        self.inputs.cancel_steer(id)
    }

    /// Turns a queued message into a steer of the running action; false when
    /// no queued message has the id. A refused steer leaves the message
    /// queued.
    pub(super) fn steer_queued(
        &mut self,
        message: &TurnId,
        steer: BlockId,
    ) -> Result<bool, SteerError> {
        self.steerable_turn()?;
        let Some(PendingAction { kind, reply, .. }) = self.take_queued(message) else {
            return Ok(false);
        };
        let ActionKind::Send { content } = kind else {
            unreachable!("a queued message is a send");
        };
        if let Some(reply) = reply {
            // A caller that dropped its handle wants no answer.
            let _ = reply.send(Ok(ActionEnd::Dropped));
        }
        self.steer(steer, content)?;
        Ok(true)
    }

    /// Admits an agent message for the next boundary. A message the session
    /// already has, pending or written, is admitted once: the same content
    /// counts as admitted, other content under its id is refused.
    pub(super) fn admit_agent_message(
        &mut self,
        message: AgentMessage,
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
        let existing = self.inputs.agent_message(&message.id).or_else(|| {
            self.transcript
                .find(&message.id)
                .and_then(|block| match block {
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
        if self.inputs.contains(message.id.as_str()) || self.transcript.find(&message.id).is_some()
        {
            return Err(AgentMessageError::Conflict);
        }
        let turn_id = match &self.activity {
            Activity::Running(run) => run.turn.clone(),
            _ => TurnId::try_from(message.id.as_str()).expect("a message id is never empty"),
        };
        self.inputs.add(Input::Agent(PendingAgentInput {
            turn_id,
            model: self.model.clone(),
            message,
        }));
        self.wake();
        Ok(())
    }

    /// Writes the waiting input `take` names into the running turn, in
    /// arrival order. Returns whether an agent message was written, which is
    /// saved at once.
    pub(super) fn write_inputs(&mut self, take: Take) -> bool {
        let inputs = self.inputs.take(take);
        if inputs.is_empty() {
            return false;
        }
        let turn = self.turn();
        let mut agent_message = false;
        for input in inputs {
            match input {
                Input::Steer(steer) => {
                    self.transcript
                        .push_steer(steer.id, turn.clone(), &steer.model, steer.content);
                }
                Input::Wakeup(wakeup) => {
                    self.transcript.push_wakeup(
                        wakeup_block_id(&wakeup.id),
                        turn.clone(),
                        &self.model,
                        WakeupPlacement::Steer,
                    );
                }
                Input::Agent(input) => {
                    agent_message = true;
                    self.transcript
                        .push_agent_message(turn.clone(), &input.model, input.message);
                }
            }
        }
        self.commit();
        agent_message
    }

    /// Writes what a continuation opens with: the first fired wakeup as a
    /// `wakeup` block that opens the turn, or else the waiting agent
    /// messages. False when nothing waits any more, and the continuation
    /// ends without a turn.
    pub(super) fn open_continuation(&mut self) -> Option<bool> {
        if let Some(wakeup) = self.inputs.take_first_wakeup() {
            let turn = self.turn();
            self.transcript.push_wakeup(
                wakeup_block_id(&wakeup.id),
                turn,
                &self.model,
                WakeupPlacement::NewTurn,
            );
            self.commit();
            return Some(false);
        }
        if self.inputs.has_agent_input() {
            return Some(self.write_inputs(Take::Everything));
        }
        None
    }

    // Yield wakeups.

    /// Schedules a wakeup its action arms when it ends.
    pub(super) fn schedule_wakeup(&mut self, duration_ms: u32) -> WakeupId {
        let id = WakeupId::try_from(self.ids.next_id())
            .expect("an id source never gives an empty identity");
        self.wakeups.schedule(id.clone(), duration_ms);
        id
    }

    /// Fires the wakeups due now: each joins the running action at its next
    /// boundary, or opens a continuation.
    pub(super) fn fire_due_wakeups(&mut self) {
        for wakeup in self.wakeups.take_due(self.clock.now()) {
            self.inputs.add(Input::Wakeup(wakeup));
        }
        self.wake();
    }

    pub(super) fn next_wakeup(&self) -> Option<demi_core::Timestamp> {
        self.wakeups.next_due()
    }

    // The model selection.

    pub(super) fn record_switch(&mut self, switch: ModelSwitch) -> Result<(), AdmissionError> {
        self.refuse_admission()?;
        if let Some(replaced) = self.switch.replace(switch) {
            self.retired.extend(replaced.runtime);
        }
        Ok(())
    }

    /// The model the recorded switch lands at `point` with, if it lands
    /// there.
    pub(super) fn switch_target(&self, point: SwitchPoint) -> Option<ModelSelection> {
        let switch = self.switch.as_ref()?;
        let lands = match point {
            SwitchPoint::ActionStart => true,
            SwitchPoint::Continuation => switch.apply == ModelSwitchApply::Immediate,
        };
        lands.then(|| switch.model.clone())
    }

    /// Makes the recorded switch current, and returns the runtimes it
    /// replaced for closing. Runs between two requests, when the runtime is
    /// in its slot.
    pub(super) fn install_switch(&mut self) -> Vec<Box<dyn ProviderRuntime>> {
        let Some(switch) = self.switch.take() else {
            return Vec::new();
        };
        let mut replaced = mem::take(&mut self.retired);
        if let Some(runtime) = switch.runtime {
            replaced.extend(self.provider.replace(runtime));
        }
        if self.model != switch.model {
            self.model = switch.model;
            self.persist.dirty = true;
            self.requests.save = true;
        }
        replaced
    }

    /// The selection the next request will use once a recorded switch lands.
    pub(super) fn latest_selection(&self) -> &ModelSelection {
        self.switch
            .as_ref()
            .map_or(&self.model, |switch| &switch.model)
    }

    // The transcript during a turn.

    /// Appends the message's `user` block, and records the command state
    /// current when the turn started as its `before_user` boundary.
    pub(super) fn push_user(
        &mut self,
        turn: TurnId,
        content: Vec<UserContentBlock>,
        preamble: Option<String>,
        revision: u64,
    ) {
        let id = self
            .transcript
            .push_user(turn, &self.model, content, preamble);
        self.commands
            .capture(id, BoundaryEdge::BeforeUser, revision);
        self.commit();
    }

    pub(super) fn push_context(&mut self, text: String) {
        let turn = self.turn();
        self.transcript.push_context(turn, &self.model, text);
        self.commit();
    }

    /// Appends a `resume` block: the turn continues after a cut.
    pub(super) fn push_resume(&mut self) {
        let turn = self.turn();
        self.transcript.push_resume(turn, &self.model);
        self.commit();
    }

    pub(super) fn mark_abort_resumed(&mut self) {
        self.transcript.mark_latest_abort_resumed();
        self.commit();
    }

    /// Applies one provider event. `thinking_started` says a thinking start
    /// came before it, which opens a reasoning block first.
    pub(super) fn apply_event(&mut self, event: ProviderEvent, thinking_started: bool) {
        match event {
            ProviderEvent::ThinkingDelta(text) if thinking_started => {
                self.transcript.open_thinking(&self.model, text);
            }
            event => {
                if thinking_started {
                    self.transcript.open_thinking(&self.model, String::new());
                }
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
                    ProviderEvent::ToolCall(call) => {
                        self.transcript.push_tool_call(&self.model, call)
                    }
                    ProviderEvent::Response(usage) => {
                        self.transcript.push_response(&self.model, usage)
                    }
                    ProviderEvent::Error(failure) => self.push_failure(&failure),
                }
            }
        }
        self.commit();
    }

    /// Records a provider failure as an `error` block.
    pub(super) fn record_failure(&mut self, failure: &ProviderFailure) {
        self.push_failure(failure);
        self.commit();
    }

    fn push_failure(&mut self, failure: &ProviderFailure) {
        self.transcript.push_error(
            &self.model,
            failure.message.clone(),
            failure.code.as_ref().map(|code| code.as_str().to_owned()),
            failure.diagnostics.clone(),
        );
    }

    /// Marks the answer text at the end complete, recording the command state
    /// current then as its `after_assistant` boundary.
    pub(super) fn complete_tail_text(&mut self) {
        if let Some(id) = self.transcript.complete_tail_text() {
            let revision = self.commands.revision();
            self.commands
                .capture(id, BoundaryEdge::AfterAssistant, revision);
        }
        self.commit();
    }

    pub(super) fn complete_tool_call(&mut self, tool_use_id: &str, outcome: ToolOutcome) {
        self.transcript.complete_tool_call(
            tool_use_id,
            outcome.output,
            outcome.is_error,
            outcome.view,
        );
        self.commit();
    }

    pub(super) fn inference_request(
        &self,
        system_prompt: String,
        tools: Arc<[ToolDefinition]>,
        request_id: String,
        cancel: CancellationToken,
    ) -> InferenceRequest {
        InferenceRequest {
            session_id: self.id.to_string(),
            turn_id: self.turn().to_string(),
            request_id,
            model_id: self.model.model.id.clone(),
            output_limit: self.model.model.output_limit.and_then(NonZeroU32::new),
            system_prompt,
            items: replay(self.transcript.blocks()).into(),
            tools,
            thinking: self.model.thinking.clone(),
            service_tier_id: self.model.service_tier_id.clone(),
            cancel,
        }
    }

    /// Drains the transcript's patches into one event, marks their rows for
    /// the next save, and records each changed block's command-state
    /// boundaries.
    pub(super) fn commit(&mut self) {
        let Some(batch) = self.transcript.take_patches() else {
            return;
        };
        self.persist.rows.mark(&batch.patches);
        self.persist.dirty = true;
        self.requests.save = true;
        let rewritten = batch.patches.iter().any(|patch| {
            matches!(
                patch,
                TranscriptPatch::Remove { .. } | TranscriptPatch::Replace { .. }
            )
        });
        if rewritten {
            self.commands.retain_boundaries(self.transcript.blocks());
        }
        let revision = self.commands.revision();
        for id in batch.touched {
            let Some(block) = self.transcript.find(&id) else {
                continue;
            };
            if opens_input_turn(block) && !self.commands.has_boundary(&id, BoundaryEdge::BeforeUser)
            {
                self.commands
                    .capture(id.clone(), BoundaryEdge::BeforeUser, revision);
            }
            self.commands
                .capture(id, BoundaryEdge::AfterBlock, revision);
        }
        self.outbox.push(SessionEvent::TranscriptChanged {
            patches: batch.patches,
            revision: batch.revision,
        });
    }

    // History rewrites.

    /// The whole checkpoint of a rewritten history: every row of `blocks`,
    /// the state row, and `commands`.
    pub(super) fn rewrite_update(
        &self,
        blocks: &[Block],
        commands: CommandStateSnapshot,
    ) -> CheckpointUpdate {
        CheckpointUpdate {
            state: self.checkpoint_state(),
            command_state: Some(commands),
            changed_blocks: blocks.iter().cloned().enumerate().collect(),
            block_count: blocks.len(),
        }
    }

    /// Adopts a rewritten history its save made durable: the transcript and
    /// command state are replaced and published as one `replace` patch,
    /// command-storage handles of the replaced history lapse, and the rows
    /// the save wrote are current.
    pub(super) fn adopt_rewrite(&mut self, blocks: Vec<Block>, commands: CommandStateHistory) {
        self.commands = commands;
        self.generation = self.generation.next();
        self.transcript.replace_all(blocks);
        let batch = self
            .transcript
            .take_patches()
            .expect("a replacement records its patch");
        self.persist.rows = Default::default();
        self.outbox.push(SessionEvent::TranscriptChanged {
            patches: batch.patches,
            revision: batch.revision,
        });
    }

    // Saving.

    /// The next save, when one is due: the changed rows, the state row, and
    /// the command state when it changed or `pending`, a command-storage
    /// version, makes it current.
    pub(super) fn prepare_checkpoint(
        &mut self,
        pending: Option<&CommandVersion>,
    ) -> Option<(CheckpointUpdate, TakenMarks)> {
        self.commit();
        if !self.persist.dirty && pending.is_none() {
            return None;
        }
        self.persist.dirty = false;
        let command_state = self.commands.take_update(pending);
        let rows = mem::take(&mut self.persist.rows);
        let blocks = self.transcript.blocks();
        let changed_blocks = rows
            .indices(blocks.len())
            .into_iter()
            .map(|index| (index, blocks[index].clone()))
            .collect();
        let taken = TakenMarks {
            rows,
            command_state: command_state.is_some(),
        };
        let update = CheckpointUpdate {
            state: self.checkpoint_state(),
            command_state,
            changed_blocks,
            block_count: blocks.len(),
        };
        Some((update, taken))
    }

    /// Puts back what a failed save took.
    pub(super) fn restore_marks(&mut self, taken: TakenMarks) {
        self.persist.dirty = true;
        self.persist.rows.merge(taken.rows);
        if taken.command_state {
            self.commands.mark_dirty();
        }
    }

    pub(super) fn checkpoint_state(&self) -> CheckpointState {
        CheckpointState {
            phase: self.recorded_phase(),
            queue: self.queued_messages(),
            agent_inputs: self.inputs.agent_inputs(),
            wakeups: self.checkpoint_wakeups(),
            cwd: self.cwd.clone(),
            model: self.model.clone(),
            harness: self.harness.clone(),
            edits: self.edits.clone(),
        }
    }

    /// Every wakeup not yet written: the scheduled ones, then those that
    /// fired and wait for a boundary.
    fn checkpoint_wakeups(&self) -> Vec<ScheduledWakeup> {
        self.wakeups
            .scheduled()
            .iter()
            .chain(self.inputs.fired_wakeups())
            .cloned()
            .collect()
    }

    fn agent_input_ids(&self) -> Vec<BlockId> {
        self.inputs
            .agent_inputs()
            .into_iter()
            .map(|input| input.message.id)
            .collect()
    }

    // What clients see, derived from the one status.

    /// The phase clients see. An action is running until its closing save
    /// has committed, and the next waiting action starts without the session
    /// going idle between them, so a client that sees `idle` can edit at once
    /// (`runtime.md` § A turn).
    pub(super) fn phase(&self) -> SessionPhase {
        match &self.activity {
            Activity::Running(run) if run.stage == TurnStage::Compacting => {
                SessionPhase::Compacting
            }
            Activity::Running(_) | Activity::Finishing => SessionPhase::Running,
            Activity::Closed { interrupted: true } => SessionPhase::Running,
            Activity::Idle | Activity::Closed { interrupted: false } => SessionPhase::Idle,
        }
    }

    /// The phase a checkpoint records. The closing save of an action records
    /// the session idle, so a checkpoint that says an action was running is
    /// one the process died in (`runtime.md` § Saving).
    fn recorded_phase(&self) -> SessionPhase {
        match &self.activity {
            Activity::Finishing => SessionPhase::Idle,
            _ => self.phase(),
        }
    }

    pub(super) fn execution(&self) -> Execution {
        match &self.activity {
            Activity::Running(run) => match run.stage {
                TurnStage::Preparing | TurnStage::Streaming => Execution::ProviderStreaming,
                TurnStage::Tools => Execution::ToolExecuting,
                TurnStage::Compacting => Execution::Compacting,
            },
            Activity::Finishing => Execution::Finalizing,
            Activity::Idle | Activity::Closed { .. } if !self.wakeups.is_empty() => {
                Execution::PendingYield
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
            wakeups: !self.wakeups.is_empty() || self.inputs.has_fired_wakeup(),
            agent_input: self.inputs.has_agent_input(),
        }
    }

    pub(super) fn pending_steers(&self) -> Vec<PendingSteer> {
        self.inputs.pending_steers()
    }

    /// Ends a change: the events it made, then the queue, the pending steers
    /// and the phase when they changed. A change of the queue, the waiting
    /// agent messages, the wakeups or the edit receipts is also due for a
    /// save.
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
        let wakeups = self.checkpoint_wakeups();
        let replan_wakeups = wakeups != self.published.wakeups;
        if replan_wakeups {
            self.published.wakeups = wakeups;
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
        let phase = self.phase();
        if phase != self.published.phase {
            self.published.phase = phase;
            events.push(SessionEvent::PhaseChanged { phase });
        }
        let requests = mem::take(&mut self.requests);
        Effects {
            events,
            wake_worker: requests.wake_worker,
            save: requests.save,
            replan_wakeups,
            status: self.status(),
        }
    }

    fn mark_state_changed(&mut self) {
        self.persist.dirty = true;
        self.requests.save = true;
    }
}

/// The id of the block a wakeup becomes.
fn wakeup_block_id(id: &WakeupId) -> BlockId {
    BlockId::try_from(id.as_str()).expect("a wakeup id is never empty")
}

/// A provider failure's diagnostics as the session records them: with the
/// attempt's client request id, and the source `unknown` when the provider
/// named none.
pub(super) fn with_request_id(mut failure: ProviderFailure, request_id: &str) -> ProviderFailure {
    let diagnostics = failure.diagnostics.get_or_insert(ProviderErrorDiagnostics {
        source: FailureSource::Unknown,
        client_request_id: None,
        provider_request_id: None,
        provider_response_id: None,
        provider_code: None,
        http_status: None,
        upstream: None,
    });
    diagnostics.client_request_id = Some(request_id.to_owned());
    failure
}
