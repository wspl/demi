//! A session's state and every decision about it (`runtime.md` § Sessions
//! and turns): one status, the actions waiting to run, the transcript and
//! command state, the model selection with its provider runtime, and what the
//! next save writes. Every method here is synchronous; the worker and the
//! turn loop await between calls, never inside one.

use std::{collections::VecDeque, mem, num::NonZeroU32, rc::Rc, sync::Arc};

use demi_agent_protocol::{AbortTarget, ModelSwitchApply, TranscriptPatch};
use demi_core::{
    FailureSource, ModelSelection, NodeId, ProviderErrorDiagnostics, QueuedMessage, SessionPhase,
    ToolResultContentBlock, TurnId, UserContentBlock,
};
use demi_provider::{
    InferenceRequest, ProviderEvent, ProviderFailure, ProviderRuntime, ToolDefinition,
};
use tokio_util::sync::CancellationToken;

use super::{
    ActionEnd, ActionHandle, ActionReply, AdmissionError, ErrorReport, ModelSwitch, SessionEvent,
    Settle,
    cancel::{CancelReason, TurnCancel},
    persist::{PersistMarks, TakenMarks},
    runtime::ToolOutcome,
};
use crate::{
    store::{BoundaryEdge, CheckpointState, CheckpointUpdate, CommandStateHistory},
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
    /// The actions waiting, in the order they run; the queue is its sends.
    pub(super) pending: VecDeque<PendingAction>,
    pub(super) activity: Activity,
    /// Dispose started: every admission is refused.
    pub(super) disposing: bool,
    pub(super) persist: PersistMarks,
    /// Events of the current change, delivered once it is complete.
    pub(super) outbox: Vec<SessionEvent>,
    requests: Requests,
    published: Published,
}

/// What the session is doing.
pub(super) enum Activity {
    Idle,
    Running(ActionRun),
    /// The action ended and its checkpoint is being saved; clients see the
    /// session idle.
    Finishing,
    /// Dispose is complete but for the final save; `interrupted` when it
    /// stopped a running turn, whose final checkpoint says `running`.
    Closed {
        interrupted: bool,
    },
}

pub(super) struct ActionRun {
    pub(super) turn: TurnId,
    pub(super) cancel: Rc<TurnCancel>,
    pub(super) stage: TurnStage,
    /// Taken by the worker when it starts the action.
    start: Option<StartedAction>,
}

/// Where a running action is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnStage {
    /// Before a request: the first one, or the next one after tools ran.
    Preparing,
    Streaming,
    Tools,
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
}

pub(super) enum ActionKind {
    /// A message: appends a `user` block and runs a turn.
    Send {
        id: TurnId,
        content: Vec<UserContentBlock>,
    },
}

impl ActionKind {
    fn send_id(&self) -> Option<&TurnId> {
        match self {
            Self::Send { id, .. } => Some(id),
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
    /// The first waiting action, which left the queue.
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

/// The derived values listeners were last told about.
struct Published {
    queue: Vec<TurnId>,
    phase: SessionPhase,
}

/// What a complete change leaves to do after the core is released.
pub(super) struct Effects {
    pub(super) events: Vec<SessionEvent>,
    pub(super) wake_worker: bool,
    pub(super) save: bool,
    pub(super) settle: Settle,
}

impl SessionCore {
    pub(super) fn new(
        id: NodeId,
        cwd: String,
        harness: String,
        model: ModelSelection,
        provider: Box<dyn ProviderRuntime>,
        transcript: TranscriptLog,
        commands: CommandStateHistory,
    ) -> Self {
        Self {
            id,
            cwd,
            harness,
            model,
            provider: Some(provider),
            switch: None,
            retired: Vec::new(),
            transcript,
            commands,
            pending: VecDeque::new(),
            activity: Activity::Idle,
            disposing: false,
            persist: PersistMarks::default(),
            outbox: Vec::new(),
            requests: Requests::default(),
            published: Published {
                queue: Vec::new(),
                phase: SessionPhase::Idle,
            },
        }
    }

    // Admission and the start of actions.

    /// Admits a message: it starts at once when nothing runs and waits in the
    /// queue otherwise. A message whose id the session already knows is
    /// acknowledged without a second turn.
    pub(super) fn admit_send(
        &mut self,
        id: TurnId,
        content: Vec<UserContentBlock>,
    ) -> Result<ActionHandle, AdmissionError> {
        if self.disposing {
            return Err(AdmissionError::Closed);
        }
        if self.knows_turn(&id) {
            return Ok(ActionHandle::ended(ActionEnd::Duplicate));
        }
        let (reply, handle) = ActionHandle::channel();
        self.pending.push_back(PendingAction {
            kind: ActionKind::Send { id, content },
            reply: Some(reply),
        });
        if matches!(self.activity, Activity::Idle) {
            self.start_next();
        }
        Ok(handle)
    }

    /// Whether `id` is the running turn, a queued message or the turn of a
    /// `user` block.
    fn knows_turn(&self, id: &TurnId) -> bool {
        let running = matches!(&self.activity, Activity::Running(run) if &run.turn == id);
        running
            || self
                .pending
                .iter()
                .any(|action| action.kind.send_id() == Some(id))
            || self.transcript.has_user_turn(id)
    }

    /// Starts the first waiting action, or settles: idle, or closed once
    /// dispose began.
    pub(super) fn start_next(&mut self) {
        if self.disposing {
            self.activity = Activity::Closed { interrupted: false };
            return;
        }
        let Some(action) = self.pending.pop_front() else {
            self.activity = Activity::Idle;
            return;
        };
        let turn = match &action.kind {
            ActionKind::Send { id, .. } => id.clone(),
        };
        let cancel = TurnCancel::new();
        self.activity = Activity::Running(ActionRun {
            turn: turn.clone(),
            cancel: cancel.clone(),
            stage: TurnStage::Preparing,
            start: Some(StartedAction {
                kind: action.kind,
                turn,
                cancel,
                reply: action.reply,
            }),
        });
        self.requests.wake_worker = true;
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

    /// The action ended; its checkpoint is saved next.
    pub(super) fn end_action(&mut self) {
        self.activity = Activity::Finishing;
    }

    /// Dispose stopped the running action, and the worker ends with it. A
    /// turn that began is recorded as interrupted, so the final checkpoint
    /// says it was running; a message whose turn wrote nothing yet goes back
    /// to the front of the queue instead. Returns whether a turn was
    /// interrupted.
    pub(super) fn shut_down(&mut self, kind: ActionKind, turn: &TurnId) -> bool {
        let began = self.transcript.has_user_turn(turn);
        if began {
            self.record_stop(CancelReason::Shutdown);
        } else {
            self.pending.push_front(PendingAction { kind, reply: None });
        }
        self.activity = Activity::Closed { interrupted: began };
        began
    }

    // Stop and dispose.

    /// Finds the one thing an `abort` stops: the running action, else the
    /// first waiting action. A session being disposed stops nothing more.
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
            };
            run.cancel.cancel(CancelReason::Stop);
            return AbortStep::Running {
                target,
                cancel: run.cancel.clone(),
            };
        }
        let Some(mut action) = self.pending.pop_front() else {
            return AbortStep::Nothing;
        };
        action.end(ActionEnd::Dropped);
        let target = match action.kind {
            ActionKind::Send { .. } => AbortTarget::QueuedMessage,
        };
        AbortStep::Removed(target)
    }

    /// Whether another `abort` would stop something.
    pub(super) fn can_abort_again(&self) -> bool {
        let running =
            matches!(&self.activity, Activity::Running(run) if !run.cancel.is_cancelled());
        !self.disposing && (running || !self.pending.is_empty())
    }

    /// A stopped action records its stop: running calls complete as
    /// aborted, then the stopped marker, or the interruption record when the
    /// session is shutting down.
    pub(super) fn record_stop(&mut self, reason: CancelReason) {
        self.abort_executing_calls();
        match reason {
            CancelReason::Stop => self.transcript.push_abort(&self.model),
            CancelReason::Shutdown => self.append_interruption(),
        }
        self.commit();
    }

    /// A failed action completes the calls it did not run, then reports the
    /// failure.
    pub(super) fn fail(&mut self, report: &ErrorReport) {
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

    /// Starts dispose: admissions end, the running action is stopped as a
    /// shutdown, and queued messages lose their callers but stay for the
    /// final checkpoint. Returns false when dispose already started.
    pub(super) fn begin_dispose(&mut self) -> bool {
        if self.disposing {
            return false;
        }
        self.disposing = true;
        for action in &mut self.pending {
            action.end(ActionEnd::Detached);
        }
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
        let Some(index) = self.queue_position(id) else {
            return false;
        };
        let mut action = self
            .pending
            .remove(index)
            .expect("the position is in the queue");
        action.end(ActionEnd::Dropped);
        true
    }

    /// Moves a queued message ahead of every other, so it runs next.
    pub(super) fn send_next(&mut self, id: &TurnId) -> bool {
        if self.disposing {
            return false;
        }
        let Some(index) = self.queue_position(id) else {
            return false;
        };
        let action = self
            .pending
            .remove(index)
            .expect("the position is in the queue");
        let front = self
            .pending
            .iter()
            .position(|action| action.kind.send_id().is_some())
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
            if action.kind.send_id().is_none() {
                return true;
            }
            action.end(ActionEnd::Dropped);
            cleared += 1;
            false
        });
        cleared
    }

    fn queue_position(&self, id: &TurnId) -> Option<usize> {
        self.pending
            .iter()
            .position(|action| action.kind.send_id() == Some(id))
    }

    pub(super) fn queued_messages(&self) -> Vec<QueuedMessage> {
        self.pending
            .iter()
            .map(|action| match &action.kind {
                ActionKind::Send { id, content } => QueuedMessage {
                    id: id.clone(),
                    content: content.clone(),
                },
            })
            .collect()
    }

    // The model selection.

    pub(super) fn record_switch(&mut self, switch: ModelSwitch) -> Result<(), AdmissionError> {
        if self.disposing {
            return Err(AdmissionError::Closed);
        }
        if let Some(replaced) = self.switch.replace(switch) {
            self.retired.extend(replaced.runtime);
        }
        Ok(())
    }

    /// The recorded switch, when it lands at `point`.
    pub(super) fn take_switch(&mut self, point: SwitchPoint) -> Option<ModelSwitch> {
        let lands = self.switch.as_ref().is_some_and(|switch| match point {
            SwitchPoint::ActionStart => true,
            SwitchPoint::Continuation => switch.apply == ModelSwitchApply::Immediate,
        });
        if lands { self.switch.take() } else { None }
    }

    /// Makes `switch` current, and returns the runtimes it replaced for
    /// closing. Runs between two requests, when the runtime is in its slot.
    pub(super) fn install(&mut self, switch: ModelSwitch) -> Vec<Box<dyn ProviderRuntime>> {
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

    pub(super) fn push_context(&mut self, turn: TurnId, text: String) {
        self.transcript.push_context(turn, &self.model, text);
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
        turn: &TurnId,
        system_prompt: String,
        tools: Arc<[ToolDefinition]>,
        request_id: String,
        cancel: CancellationToken,
    ) -> InferenceRequest {
        InferenceRequest {
            session_id: self.id.to_string(),
            turn_id: turn.to_string(),
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

    // Saving.

    /// The next save, when one is due: the changed rows, the state row, and
    /// the command state when it changed.
    pub(super) fn prepare_checkpoint(&mut self) -> Option<(CheckpointUpdate, TakenMarks)> {
        self.commit();
        if !self.persist.dirty {
            return None;
        }
        self.persist.dirty = false;
        let command_state = self.commands.take_update();
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
            phase: self.phase(),
            queue: self.queued_messages(),
            cwd: self.cwd.clone(),
            model: self.model.clone(),
            harness: self.harness.clone(),
        }
    }

    // What clients see, derived from the one status.

    pub(super) fn phase(&self) -> SessionPhase {
        match &self.activity {
            Activity::Idle | Activity::Finishing => SessionPhase::Idle,
            Activity::Running(_) => SessionPhase::Running,
            Activity::Closed { interrupted: true } => SessionPhase::Running,
            Activity::Closed { interrupted: false } => SessionPhase::Idle,
        }
    }

    pub(super) fn settle(&self) -> Settle {
        match &self.activity {
            Activity::Closed { .. } => Settle::Closed,
            Activity::Idle if self.pending.is_empty() => Settle::Settled,
            _ => Settle::Busy,
        }
    }

    /// Ends a change: the events it made, then the queue and the phase when
    /// they changed. A change of the queue is also due for a save.
    pub(super) fn take_effects(&mut self) -> Effects {
        let mut events = mem::take(&mut self.outbox);
        let queue: Vec<TurnId> = self
            .pending
            .iter()
            .filter_map(|action| action.kind.send_id().cloned())
            .collect();
        if queue != self.published.queue {
            self.published.queue = queue;
            self.persist.dirty = true;
            self.requests.save = true;
            events.push(SessionEvent::QueueChanged {
                queue: self.queued_messages(),
            });
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
            settle: self.settle(),
        }
    }
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
