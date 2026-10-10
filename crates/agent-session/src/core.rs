//! A session's state and every decision about it (`runtime.md` § Sessions
//! and turns): one status, the actions waiting to run, the input waiting for
//! a boundary, the commands that report to it, the transcript, the model
//! selection with its provider runtime, and what the next save writes.
//! Every method here is synchronous; the worker and the turn loop await
//! between calls, never inside one.

use std::{
    collections::{HashSet, VecDeque},
    mem,
    num::NonZeroU32,
    rc::Rc,
    sync::Arc,
};

use demi_agent_store::{
    CheckpointState, CheckpointUpdate, CommandInterval, EditReceipt, PendingAgentInput,
    media::{self, HeldMedia, ModelView},
};
use demi_agent_transcript::{
    DirtyRows, INTERRUPTED_CODE, INTERRUPTED_TURN_MESSAGE, IdSource, RequestView, TranscriptLog,
    estimate::{blocks_tokens, context_anchor},
    replay, replay_start,
};
use demi_conversation_socket_protocol::AbortTarget;
use demi_provider_common::{
    EntriesOf, InferenceRequest, PromptCache, ProviderEvent, ProviderFailure, ProviderRuntime,
    RequestBlock, RequestLimits, ToolDefinition,
};
use demi_shared_types::{
    AgentMessage, BlobRef, Block, BlockId, Clock, CommandId, FailureSource,
    ModelSelection, NodeId, PendingCall, PendingSteer, ProviderErrorDiagnostics, QueuedMessage, SessionPhase,
    ToolResultContentBlock, ToolView, TurnId, UserContentBlock, WakeupPlacement,
};
use tokio_util::sync::CancellationToken;

use super::{
    ActionEnd, ActionHandle, ActionReply, AdmissionError, AgentMessageError, ErrorReport,
    Execution, ModelSwitch, SessionEvent, Settle, Status, SteerError,
    cancel::{CancelReason, TurnCancel},
    editing::{EditCheck, EditError, EditInFlight, EditSubmission},
    input::{Input, InputQueue, Take},
    persist::PersistMarks,
    reports::Watched,
    runtime::Arrivals,
};

pub(crate) struct SessionCore {
    /// The calls of the running request that the model is writing, live
    /// only (`runtime.md` § Calls being written).
    pub(super) writing: Vec<WritingCall>,
    /// The blocks the running request's entries name (`EntriesOf`), live
    /// only.
    request_blocks: RequestBlocks,
    pub(super) id: NodeId,
    pub(super) cwd: String,
    /// The selection current now; every block records the one current when
    /// it was written.
    pub(super) model: ModelSelection,
    /// The runtime that serves requests, or what it takes while a run holds it.
    pub(super) provider: ProviderSlot,
    /// A recorded model switch that has not landed yet.
    pub(super) switch: Option<ModelSwitch>,
    /// A switch that arrived while an edit was being prepared: it waits for
    /// the edit, and is recorded once the edit is accepted or rejected
    /// (`runtime.md` § Model switch).
    pub(super) waiting_switch: Option<ModelSwitch>,
    /// Runtimes a later switch replaced before they served a request, closed
    /// when the next switch lands or at dispose.
    pub(super) retired: Vec<Box<dyn ProviderRuntime>>,
    pub(super) transcript: TranscriptLog,
    /// What the session holds for the media its replayed blocks and its
    /// waiting input reference (`runtime.md` § Media).
    pub(super) media: HeldMedia,
    /// The tools its requests declare, which replay reads.
    pub(super) tools: Arc<[ToolDefinition]>,
    /// The actions waiting, in the order they run; the queue is its sends.
    pub(super) pending: VecDeque<PendingAction>,
    pub(super) inputs: InputQueue,
    /// The commands the session's calls left running, which report to it.
    pub(super) watched: Watched,
    pub(super) edits: Vec<EditReceipt>,
    /// The edit being prepared or run, from its admission until its action
    /// ends.
    pub(super) editing: Option<EditInFlight>,
    pub(super) activity: Activity,
    /// Waiting input opens no continuation until the node's next action:
    /// the session is a restored root whose last turn was interrupted, or a
    /// closing child. A restore reads it from the checkpoint again, so it is
    /// not saved.
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
    /// The blobs a send's message references until its `user` block is
    /// written: waiting input meanwhile, whose held bytes stay
    /// (`runtime.md` § Media).
    unwritten_media: Vec<BlobRef>,
    /// What the user sent now, which the turn takes at its next boundary.
    send_now: Option<SendNow>,
    /// Taken by the worker when it starts the action.
    start: Option<StartedAction>,
}

/// What the user sent now (`runtime.md` § Send now). A queued message
/// outranks a steer: both are written at the boundary, and the turn ends.
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
    /// Input that arrived while nothing ran, command reports or agent
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

/// A call the model is writing: what the page is told, and the input
/// written so far, from which its description is read.
pub(super) struct WritingCall {
    call: PendingCall,
    input: String,
}

/// The derived values listeners and the save were last told about.
struct Published {
    queue: Vec<TurnId>,
    phase: SessionPhase,
    pending_steers: Vec<PendingSteer>,
    pending_calls: Vec<PendingCall>,
    agent_inputs: Vec<BlockId>,
    reports: Vec<String>,
    intervals: Vec<CommandInterval>,
    edits: usize,
}

/// What a complete change leaves to do after the core is released.
pub(super) struct Effects {
    pub(super) events: Vec<SessionEvent>,
    pub(super) wake_worker: bool,
    pub(super) save: bool,
    pub(super) status: Status,
}

/// Where a session's provider runtime is. A run takes it out of its slot
/// for the length of a request, and the session goes on weighing its next
/// request meanwhile, as when a page opens the conversation while a turn
/// streams and is told the usage (`compaction.md` § Context estimate): so
/// the slot keeps what the runtime takes in one request of the current
/// model, which does not change while the runtime is out.
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
    pub(super) cwd: String,
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
    pub(super) held: bool,
    pub(super) ids: Rc<dyn IdSource>,
    pub(super) clock: Arc<dyn Clock>,
}

impl SessionCore {
    pub(super) fn new(parts: CoreParts) -> Self {
        let mut core = Self {
            writing: Vec::new(),
            request_blocks: RequestBlocks::default(),
            id: parts.id,
            cwd: parts.cwd,
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
            watched: parts.watched,
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
                pending_calls: Vec::new(),
                agent_inputs: Vec::new(),
                reports: Vec::new(),
                intervals: Vec::new(),
                edits: 0,
            },
        };
        // What the checkpoint holds counts as published: only later changes
        // make a save due.
        core.published.agent_inputs = core.agent_input_ids();
        core.published.reports = core.inputs.reports();
        core.published.intervals = core.watched.intervals();
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
        operation: &demi_shared_types::OperationId,
        digest: &str,
        version: &demi_conversation_socket_protocol::TranscriptVersion,
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
        let waiting = self.inputs.has_agent_input()
            || self.inputs.has_report()
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
    /// receipt, the model and the runtime fork the replacement runs on.
    /// Returns the runtimes it discarded, for closing.
    pub(super) fn adopt_edit(
        &mut self,
        blocks: Vec<Block>,
        runtime: Box<dyn ProviderRuntime>,
        model: ModelSelection,
        receipt: EditReceipt,
    ) -> Vec<Box<dyn ProviderRuntime>> {
        self.adopt_rewrite(blocks);
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
        self.start_action(kind, turn, reply);
    }

    /// Runs `kind` as the session's action, writing `turn`.
    fn start_action(&mut self, kind: ActionKind, turn: TurnId, reply: Option<ActionReply>) {
        let cancel = TurnCancel::new();
        self.held = false;
        let unwritten_media = match &kind {
            ActionKind::Send { content } => media::content_references(content).cloned().collect(),
            _ => Vec::new(),
        };
        self.activity = Activity::Running(ActionRun {
            turn: turn.clone(),
            cancel: cancel.clone(),
            stage: TurnStage::Preparing,
            started_at: self.transcript.version().revision,
            unwritten_media,
            send_now: None,
            start: Some(StartedAction {
                kind,
                turn,
                cancel,
                reply,
            }),
        });
        self.requests.wake_worker = true;
    }

    /// Whether waiting input would open a continuation of its own: command
    /// reports or agent messages, unless the session is held.
    fn wants_continuation(&self) -> bool {
        !self.held && (self.inputs.has_report() || self.inputs.has_agent_input())
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
    /// waits.
    pub(super) fn wake(&mut self) {
        if matches!(self.activity, Activity::Idle) && self.pending.is_empty() {
            self.start_next();
        }
    }

    /// Opens a continuation for the user's decision on a permission request
    /// when nothing runs or waits, also after the user stopped the last turn
    /// or while the session is held: the decision is the user's own next
    /// action (`permissions.md` § The decision's message).
    fn wake_for_user(&mut self) {
        if matches!(self.activity, Activity::Idle) && self.pending.is_empty() && !self.disposing {
            let turn = self.new_turn();
            self.start_action(ActionKind::Continue, turn, None);
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
    /// still pending are dropped, and a later Stop stops none of the
    /// commands it left running.
    pub(super) fn end_action(&mut self) {
        self.activity = Activity::Finishing;
        self.editing = None;
        self.inputs.discard_steers();
        self.watched.end_action();
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

    /// Dispose stopped the running action, and the worker ends with it. An
    /// action that wrote into the transcript is recorded as interrupted, so
    /// the final checkpoint says it was running; a message whose turn wrote
    /// nothing yet goes back to the front of the queue instead, and another
    /// action that wrote nothing ends with the session. Returns whether a
    /// turn was interrupted.
    pub(super) fn shut_down(&mut self, kind: ActionKind, turn: &TurnId) -> bool {
        let began = self.action_began();
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
                TurnStage::Compacting => AbortTarget::ActiveCompaction,
            };
            run.cancel.cancel(CancelReason::Stop);
            let cancel = run.cancel.clone();
            self.watched.stop_action();
            return AbortStep::Running { target, cancel };
        }
        if let Some(mut action) = self.pending.pop_front() {
            action.end(ActionEnd::Dropped);
            let target = match action.kind {
                ActionKind::Send { .. } => AbortTarget::QueuedMessage,
                _ => AbortTarget::QueuedAction,
            };
            return AbortStep::Removed(target);
        }
        AbortStep::Nothing
    }

    /// Stops the running action, as the user's Stop would, and nothing
    /// that waits: a waiting action stays. The answer is the stopped
    /// action's token, which says when it recorded the stop.
    pub(super) fn stop_running(&mut self) -> Option<Rc<TurnCancel>> {
        if self.disposing {
            return None;
        }
        let cancel = match &self.activity {
            Activity::Running(run) if !run.cancel.is_cancelled() => run.cancel.clone(),
            _ => return None,
        };
        cancel.cancel(CancelReason::Stop);
        self.watched.stop_action();
        Some(cancel)
    }

    /// Whether another `abort` would stop something.
    pub(super) fn can_abort_again(&self) -> bool {
        let running =
            matches!(&self.activity, Activity::Running(run) if !run.cancel.is_cancelled());
        !self.disposing && (running || !self.pending.is_empty())
    }

    /// A stopped action records the stop: for the user's Stop, it writes
    /// all the input waiting for its next boundary, and for a shutdown the
    /// human steers alone; then it completes running calls as aborted and
    /// appends the stopped marker, or the interruption record when the
    /// session is shutting down. A Stop of an action that wrote nothing,
    /// with what the stop wrote counted, appends no marker: it began no turn
    /// (`failures-and-recovery.md` § The unfinished turn). The Stop holds
    /// nothing afterwards (`runtime.md` § Stop).
    pub(super) fn record_stop(&mut self, reason: CancelReason) {
        let take = match reason {
            CancelReason::Stop => Take::Everything,
            CancelReason::Shutdown => Take::Steers,
        };
        self.write_inputs(take);
        self.abort_executing_calls();
        match reason {
            CancelReason::Stop if self.action_began() => self.transcript.push_abort(&self.model),
            CancelReason::Stop => {}
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
            false,
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

    /// Sends a queued message now (`runtime.md` § Send now): it moves to the
    /// front of the queue and, while a turn runs, cuts the turn's round
    /// short and ends the turn at the boundary. False when no queued message
    /// has the id.
    pub(super) fn send_queued_now(&mut self, id: &TurnId) -> bool {
        if !self.send_next(id) {
            return false;
        }
        self.cut_round(SendNow::Message);
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

    /// Delivers the pending steer `id` now (`runtime.md` § Send now): the
    /// turn's round is cut short, and the turn goes on with the steer. A
    /// steer that is no longer pending changes nothing.
    pub(super) fn steer_now(&mut self, id: &BlockId) {
        if self.inputs.has_steer(id) {
            self.cut_round(SendNow::Steer);
        }
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

    /// What the windows of the running calls watch for
    /// (`runtime.md` § The window).
    pub(super) fn window_arrivals(&self) -> Arrivals {
        Arrivals {
            joining: self.inputs.joining(),
            send_now: self.send_now_pending(),
        }
    }

    /// Takes what the user sent now, at a boundary.
    pub(super) fn take_send_now(&mut self) -> Option<SendNow> {
        match &mut self.activity {
            Activity::Running(run) => run.send_now.take(),
            _ => None,
        }
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
        let from_product = message.event.is_from_product();
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

    /// Writes the waiting input `take` names into the running turn, in
    /// arrival order; the command reports among it are one `wakeup` block,
    /// one paragraph each, where the first of them arrived. Returns whether
    /// an agent message was written, which is saved at once.
    pub(super) fn write_inputs(&mut self, take: Take) -> bool {
        let inputs = self.inputs.take(take);
        if inputs.is_empty() {
            return false;
        }
        let turn = self.turn();
        let mut reports: Vec<String> = inputs
            .iter()
            .filter_map(|input| match input {
                Input::Report(report) => Some(report.clone()),
                _ => None,
            })
            .collect();
        let mut agent_message = false;
        for input in inputs {
            match input {
                Input::Steer(steer) => {
                    self.transcript
                        .push_steer(steer.id, turn.clone(), &steer.model, steer.content);
                }
                Input::Report(_) if reports.is_empty() => {}
                Input::Report(_) => {
                    let text = mem::take(&mut reports).join("\n\n");
                    self.push_reports(turn.clone(), WakeupPlacement::Steer, text);
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

    /// Writes what a continuation opens with: the command reports as one
    /// `wakeup` block that opens the turn, or else the waiting agent
    /// messages. False when nothing waits any more, and the continuation
    /// ends without a turn.
    pub(super) fn open_continuation(&mut self) -> Option<bool> {
        let reports = self.inputs.take_reports();
        if !reports.is_empty() {
            let turn = self.turn();
            self.push_reports(turn, WakeupPlacement::NewTurn, reports.join("\n\n"));
            self.commit();
            return Some(false);
        }
        if self.inputs.has_agent_input() {
            return Some(self.write_inputs(Take::Everything));
        }
        None
    }

    /// A `wakeup` block of command reports, `text` one paragraph each.
    fn push_reports(&mut self, turn: TurnId, placement: WakeupPlacement, text: String) {
        let id = BlockId::try_from(self.ids.next_id())
            .expect("an id source never gives an empty identity");
        self.transcript
            .push_wakeup(id, turn, &self.model, placement, text);
    }

    // Command reports.

    /// Watches `command`, which a call of the running action titled `title`
    /// left running, so that it reports every `interval_ms`, or only its end
    /// when none.
    pub(super) fn watch_command(
        &mut self,
        command: CommandId,
        interval_ms: Option<u32>,
        title: String,
    ) {
        self.watched.add(command, interval_ms, title, true);
    }

    /// Admits a command's report for the next boundary: it joins the running
    /// turn, or opens a continuation. A session that is closing keeps no
    /// more input.
    pub(super) fn admit_report(&mut self, text: String) {
        if self.disposing {
            return;
        }
        self.inputs.add(Input::Report(text));
        self.wake();
    }

    // The model selection.

    /// Records `switch` in place of the pending one (`runtime.md` § Model
    /// switch); while an edit is being prepared, in place of the one that
    /// waits for it. A session that is closing gives it back.
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
        if let Some(runtime) = switch.runtime {
            replaced.extend(self.place_runtime(runtime));
        }
        if self.model != *switch.model {
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
        preamble: Option<String>,
    ) {
        self.transcript
            .push_user(turn, &self.model, content, preamble);
        if let Activity::Running(run) = &mut self.activity {
            run.unwritten_media.clear();
        }
        self.commit();
    }

    pub(super) fn push_context(&mut self, news: crate::runtime::NewContext) {
        let turn = self.turn();
        self.transcript
            .push_context(turn, &self.model, news.source, news.text, news.instructions);
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
        // Entries write no block, so a thinking start waits on.
        if let ProviderEvent::Entries { of, entries } = event {
            self.keep_entries(of, entries);
            return;
        }
        let before = self.transcript.blocks().len();
        self.apply_event_to_transcript(event, thinking_started);
        // The blocks a run's events write, which its entries name by
        // position (`EntriesOf::Output`).
        let written = self.transcript.blocks()[before..]
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
    pub(super) fn keep_entries(&mut self, of: EntriesOf, entries: Vec<serde_json::Value>) {
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

    fn apply_event_to_transcript(&mut self, event: ProviderEvent, thinking_started: bool) {
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
                    ProviderEvent::Entries { .. } => {
                        unreachable!("apply_event keeps entries before it writes blocks")
                    }
                }
            }
        }
    }

    /// The request ended, failed or was cancelled: no call is being written
    /// any more.
    pub(super) fn end_writing(&mut self) {
        self.writing.clear();
    }

    /// The calls the model is writing, oldest first.
    pub(super) fn pending_calls(&self) -> Vec<PendingCall> {
        self.writing.iter().map(|writing| writing.call.clone()).collect()
    }

    /// Records a failed provider request, the turn's own or a compaction's
    /// summary request, as an `error` block
    /// (`failures-and-recovery.md` § The failure record).
    pub(super) fn record_failure(&mut self, failure: &ErrorReport) {
        self.push_failure(failure);
        self.commit();
    }

    /// A failure of an action that has not begun ended no turn, as a
    /// `compact` action whose summary request failed: its record says so
    /// (`failures-and-recovery.md` § Retries).
    fn push_failure(&mut self, failure: &ErrorReport) {
        let outside_turn = !self.action_began();
        self.transcript.push_error(
            &self.model,
            failure.message.clone(),
            failure.code.clone(),
            failure.diagnostics.clone(),
            outside_turn,
        );
    }

    /// Marks the answer text at the end complete.
    pub(super) fn complete_tail_text(&mut self) {
        self.transcript.complete_tail_text();
        self.commit();
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
        self.transcript
            .complete_tool_call(tool_use_id, output, is_error, view);
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
        let replayed = replay(&self.request_view(view));
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
        InferenceRequest {
            session_id: self.id.to_string(),
            turn_id: self.turn().to_string(),
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
        }
    }

    /// What the current model's vendor takes in one request
    /// (`models.md` § Request limits): from the runtime in its slot, or as
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
    fn replayed(&self) -> &[Block] {
        let blocks = self.transcript.blocks();
        &blocks[replay_start(blocks)..]
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
            if let ActionKind::Send { content } = &action.kind {
                referenced.extend(media::content_references(content).cloned());
            }
        }
        for steer in self.inputs.pending_steers() {
            referenced.extend(media::content_references(&steer.content).cloned());
        }
        if let Activity::Running(run) = &self.activity {
            referenced.extend(run.unwritten_media.iter().cloned());
        }
        referenced
    }

    /// Lets go of the media nothing references any more (`runtime.md`
    /// § Media).
    pub(super) fn release_media(&mut self) {
        let referenced = self.referenced_media();
        self.media.retain(&referenced);
    }

    /// The replayed blocks as the model receives them, with what the session
    /// holds for their media; or, while it holds nothing for some of them,
    /// their blobs.
    pub(super) fn model_view(&self) -> Result<ModelView, Vec<BlobRef>> {
        let blocks = self.transcript.blocks();
        let start = replay_start(blocks);
        ModelView::of(start, &blocks[start..], &self.media)
    }

    /// The estimate of the next request of the current model
    /// (`compaction.md` § Context estimate), or the blobs to read first. Only
    /// the blocks after the anchor weigh, so only their media need to be
    /// held: a session restored with media, which holds none, tells it
    /// without a read while those blocks reference none.
    pub(super) fn context_estimate(&self) -> Result<u64, Vec<BlobRef>> {
        let blocks = self.transcript.blocks();
        let start = replay_start(blocks);
        let replayed = &blocks[start..];
        let anchor = context_anchor(replayed, &self.model.model);
        let weighed = ModelView::of(start + anchor.from, &replayed[anchor.from..], &self.media)?;
        Ok(anchor.tokens + blocks_tokens(&weighed.blocks, &self.request_view(&weighed)))
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
            revision: batch.revision,
        });
    }

    // History rewrites.

    /// The whole checkpoint of a rewritten history: every row of `blocks`
    /// and the state row.
    pub(super) fn rewrite_update(&self, blocks: &[Block]) -> CheckpointUpdate {
        CheckpointUpdate {
            state: self.checkpoint_state(),
            changed_blocks: blocks.iter().cloned().enumerate().collect(),
            block_count: blocks.len(),
        }
    }

    /// Adopts a rewritten history its save made durable: the transcript is
    /// replaced and published as one `replace` patch, and the rows the save
    /// wrote are current.
    pub(super) fn adopt_rewrite(&mut self, blocks: Vec<Block>) {
        let batch = self.transcript.replace_all(blocks);
        self.persist.rows = Default::default();
        self.outbox.push(SessionEvent::TranscriptChanged {
            patches: batch.patches,
            revision: batch.revision,
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
        let blocks = self.transcript.blocks();
        let changed_blocks = rows
            .indices(blocks.len())
            .into_iter()
            .map(|index| (index, blocks[index].clone()))
            .collect();
        let update = CheckpointUpdate {
            state: self.checkpoint_state(),
            changed_blocks,
            block_count: blocks.len(),
        };
        Some((update, rows))
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
            reports: self.inputs.reports(),
            intervals: self.watched.intervals(),
            cwd: self.cwd.clone(),
            model: self.model.clone(),
            edits: self.edits.clone(),
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
        }
    }

    pub(super) fn pending_steers(&self) -> Vec<PendingSteer> {
        self.inputs.pending_steers()
    }

    /// Ends a change: the events it made, then the queue, the pending steers
    /// and the phase when they changed. A change of the queue, the waiting
    /// agent messages and command reports, the commands that report or the
    /// edit receipts is also due for a save.
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
        let reports = self.inputs.reports();
        if reports != self.published.reports {
            self.published.reports = reports;
            self.mark_state_changed();
        }
        let intervals = self.watched.intervals();
        if intervals != self.published.intervals {
            self.published.intervals = intervals;
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
        let pending_calls = self.pending_calls();
        if pending_calls != self.published.pending_calls {
            self.published.pending_calls = pending_calls.clone();
            events.push(SessionEvent::PendingCallsChanged { pending_calls });
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

/// Puts `switch` in `slot` in place of the switch there. A switch within the
/// replaced switch's provider brings no runtime and lands on the replaced
/// one's; one that brings its own retires the replaced one's, which closes
/// when a switch lands.
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
