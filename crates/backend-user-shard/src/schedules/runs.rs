//! One run of a schedule.
//!
//! A run is recorded as it fires, before anything else, so it never starts
//! twice.
//!
//! It is skipped when:
//!
//! - its conversation's Host is offline, after the restart hold for one the
//!   backend's own restart disconnected;
//! - its conversation is gone or archived.
//!
//! Otherwise its message wakes the conversation: queued behind a turn that
//! runs, or, when the schedule steers, sent into it as the composer's Steer
//! sends a queued message. The run ends with the turn its message started, and
//! a once schedule is deleted with it, unless it was skipped.

use std::rc::Rc;

use demi_agent_server::AgentServer;
use demi_agent_store::StoreError;
use demi_backend_database::schedules::{RunCause, StoredRun, StoredSchedule};
use demi_backend_host_access::root_of;
use demi_backend_page_sync::Part;
use demi_shared_types::{
    Block, QueuedMessage, ScheduleId, ScheduleMark, ScheduleRunId, SessionPhase, Timestamp, TurnId, UserContentBlock,
};
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use demi_web_api_protocol::schedules::{IfBusy, RunNote, RunState, Task, Timing, Trigger};

use super::hooks::Call;
use super::message;
use crate::shard::Shard;

/// What fired a run, with what its message carries of it.
pub(crate) struct Fired {
    pub cause: RunCause,
    /// When the trigger fired: a time of the schedule, or now.
    pub at: Timestamp,
    /// The repeat's task it runs; none for a single trigger.
    pub task: Option<String>,
    pub payload: Payload,
}

pub(crate) enum Payload {
    None,
    /// A change watch's output before and now, each cut to its last 16 KB.
    Change { before: message::Output, now: message::Output },
    /// A URL's calls, oldest first.
    Calls(Vec<Call>),
}

impl Fired {
    pub(crate) fn time(at: Timestamp, task: Option<String>) -> Self {
        Self {
            cause: RunCause::Time,
            at,
            task,
            payload: Payload::None,
        }
    }
}

/// A repeat's tasks; none for a single trigger.
pub(crate) fn tasks_of(trigger: &Trigger) -> Option<&[Task]> {
    match trigger {
        Trigger::Time {
            timing: Timing::Repeat { tasks, .. },
        } => Some(tasks),
        _ => None,
    }
}

/// What a trigger's runs are fired by.
pub(crate) fn cause_of(trigger: &Trigger) -> RunCause {
    match trigger {
        Trigger::Time { .. } => RunCause::Time,
        Trigger::Change { .. } => RunCause::Change,
        Trigger::Url {} => RunCause::Url,
    }
}

fn new_run_id() -> ScheduleRunId {
    ScheduleRunId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty")
}

/// A run skipped as it fired, for `note`.
pub(crate) fn skipped(schedule: &StoredSchedule, cause: RunCause, at: Timestamp, note: RunNote) -> StoredRun {
    StoredRun {
        id: new_run_id(),
        schedule: schedule.id.clone(),
        cause,
        at,
        task: None,
        state: RunState::Skipped,
        note: Some(note),
        line: None,
        turn: None,
        notified: false,
    }
}

/// Why a conversation cannot run a run now.
pub(crate) enum Unrunnable {
    /// The conversation is gone or archived.
    Gone,
    Offline(String),
    /// A paired device whose runner is given the rest of its wait to come
    /// back, after its connection dropped or the backend restarted
    /// (`crates/backend-runners/src/devices.rs`).
    Returning { device: DeviceId, rest: std::time::Duration },
}

impl Shard {
    /// Whether `schedule`'s conversation can run a run now, or why not: it
    /// is archived or gone, or its Host is offline.
    pub(crate) async fn runnable(&self, schedule: &StoredSchedule) -> Result<(), Unrunnable> {
        let control = &self.services().control;
        let record = match control.conversation(schedule.conversation.clone()).await {
            Ok(Some(record)) if record.owner == *self.user() && !record.archived => record,
            _ => return Err(Unrunnable::Gone),
        };
        let device = self
            .host_shard()
            .resolve_target(&record)
            .await
            .ok()
            .and_then(|target| target.device().cloned());
        if let Some(device) = device
            && let Some(name) = self.offline_device(&device).await
        {
            return Err(match self.devices().returning(&device) {
                Some(rest) => Unrunnable::Returning { device, rest },
                None => Unrunnable::Offline(name),
            });
        }
        Ok(())
    }

    /// Whether `schedule`'s conversation can run a run, as `runnable`
    /// answers, once a returning device is back or its wait is over.
    pub(crate) async fn runnable_after_return(&self, schedule: &StoredSchedule) -> Result<(), Unrunnable> {
        match self.runnable(schedule).await {
            Err(Unrunnable::Returning { device, rest }) => {
                self.devices().returned(&device, rest).await;
                match self.runnable(schedule).await {
                    // The wait is over: the device is offline.
                    Err(Unrunnable::Returning { device, .. }) => Err(Unrunnable::Offline(
                        self.offline_device(&device).await.unwrap_or_else(|| device.to_string()),
                    )),
                    other => other,
                }
            }
            other => other,
        }
    }

    /// The name of `device` when it is a paired device without a live
    /// runner; none for one that is online and for the Cloud, which a run
    /// wakes.
    pub(crate) async fn offline_device(&self, device: &DeviceId) -> Option<String> {
        let record = self.services().control.device(device.clone()).await.ok().flatten()?;
        (record.kind == DeviceKind::User && !self.devices().connected(device)).then_some(record.name)
    }

    /// The schedule's run whose message the conversation works for, with when
    /// it came.
    ///
    /// The run covers every turn of the conversation's tree from that message
    /// until the user next writes to the conversation, whatever the run's state.
    /// None while the user's own message is the latest, or for a conversation no
    /// tree holds.
    pub(crate) fn schedule_input(&self, conversation: &ConversationId) -> Option<(ScheduleMark, Timestamp)> {
        let root = root_of(conversation);
        self.agent().node(&root, &root)?.session().schedule_input()
    }

    /// When the message of a run a URL started came, while the conversation
    /// works for it: each credential read asks then. A run whose record is
    /// gone counts as one, since what started it is no longer known.
    pub(crate) async fn url_message_at(&self, conversation: &ConversationId) -> Option<Timestamp> {
        let (mark, at) = self.schedule_input(conversation)?;
        match self.services().control.schedule_run(mark.run_id).await {
            Ok(Some(run)) if run.cause != RunCause::Url => None,
            _ => Some(at),
        }
    }

    /// Fires a run of `schedule`: records it, skipped when its conversation
    /// cannot run it, and otherwise carries it in a task of its own.
    pub(crate) async fn fire(&self, schedule: &StoredSchedule, fired: Fired) {
        let control = &self.services().control;
        let runnable = self.runnable(schedule).await;
        let skipped = matches!(runnable, Err(Unrunnable::Gone | Unrunnable::Offline(_)));
        let note = match &runnable {
            Err(Unrunnable::Offline(device)) => Some(RunNote::Offline { device: device.clone() }),
            _ => None,
        };
        let mut run = StoredRun {
            id: new_run_id(),
            schedule: schedule.id.clone(),
            cause: fired.cause,
            at: fired.at,
            task: fired.task.clone(),
            state: if skipped { RunState::Skipped } else { RunState::Running },
            note,
            line: None,
            turn: None,
            notified: false,
        };
        if let Err(error) = control.insert_schedule_run(run.clone()).await {
            tracing::error!(event = "schedule_run.start_failed", user = %self.user(), schedule = %schedule.id, run = %run.id, %error, "a run was not recorded, and did not start");
            return;
        }
        tracing::info!(
            event = "schedule.fired",
            user = %self.user(),
            schedule = %schedule.id,
            run = %run.id,
            trigger = cause_name(fired.cause),
            conversation = %schedule.conversation,
            skipped_reason = skipped.then(|| run.note.as_ref().map_or("gone", note_name)),
            "a schedule fired"
        );
        self.mark(Part::Metadata(schedule.conversation.clone()));
        if skipped {
            return;
        }
        let shard = self.this();
        let schedule = schedule.clone();
        self.tasks().spawn_local(async move {
            // A returning device is waited for in this task, the run
            // recorded already, so the scheduler goes on and never starts it
            // twice; still away after the wait, it is skipped.
            let runnable = match runnable {
                Err(Unrunnable::Returning { .. }) => shard.runnable_after_return(&schedule).await,
                runnable => runnable,
            };
            if let Err(unrunnable) = runnable {
                run.state = RunState::Skipped;
                run.note = match unrunnable {
                    Unrunnable::Gone => None,
                    Unrunnable::Offline(device) => Some(RunNote::Offline { device }),
                    Unrunnable::Returning { device, .. } => Some(RunNote::Offline { device: device.to_string() }),
                };
                shard.end_run(run, &schedule.conversation).await;
                return;
            }
            if let Err(message) = shard.carry(&schedule, &mut run, fired).await {
                run.state = RunState::Failed;
                run.note = Some(RunNote::TurnFailed { message, code: None });
                shard.end_run(run, &schedule.conversation).await;
            }
        });
    }

    /// Starts the run's turn: writes a URL's bodies into the conversation's
    /// scratch directory, and sends the message, which waits in the queue
    /// behind a turn that runs.
    async fn carry(&self, schedule: &StoredSchedule, run: &mut StoredRun, fired: Fired) -> Result<(), String> {
        let control = &self.services().control;
        let conversation = schedule.conversation.clone();
        let turn = TurnId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        run.turn = Some(turn.clone());
        control.update_schedule_run(run.clone()).await.map_err(|error| error.to_string())?;
        self.mark(Part::Conversation(conversation.clone()));
        let prompt = message::prompt_of(schedule, fired.task.as_deref());
        let text = match fired.payload {
            Payload::None => message::text(schedule, prompt, fired.cause, fired.at, &message::Extra::None),
            Payload::Change { before, now } => message::text(
                schedule,
                prompt,
                fired.cause,
                fired.at,
                &message::Extra::Change { before, now },
            ),
            Payload::Calls(calls) => {
                let written = self.write_bodies(&conversation, &run.id, &calls).await?;
                message::text(schedule, prompt, fired.cause, fired.at, &message::Extra::Calls { calls, written })
            }
        };
        self.restore_tree(&conversation).await?;
        let tree = self
            .agent()
            .tree(&root_of(&conversation))
            .ok_or("the conversation's tree closed before the message")?;
        let session = tree.root().session();
        session
            .send_message(QueuedMessage {
                comments: Vec::new(),
                id: turn.clone(),
                content: vec![UserContentBlock::Text { text }],
                schedule: Some(ScheduleMark {
                    schedule_id: schedule.id.clone(),
                    run_id: run.id.clone(),
                    name: schedule.name.clone(),
                    at: fired.at,
                }),
            })
            .map_err(|error| error.to_string())?;
        // A message that waits behind a running turn reaches it, as the
        // composer's Steer does; one that started a turn at once waits for
        // nothing, and the call changes nothing.
        if schedule.if_busy == IfBusy::Steer {
            session.send_queued_message(&turn);
        }
        Ok(())
    }

    /// Records the run's end, which woke `conversation`, and tells the
    /// pages; a once schedule whose time woke the conversation is deleted, and
    /// a URL's calls that came while it worked start the next run.
    pub(crate) async fn end_run(&self, run: StoredRun, conversation: &ConversationId) {
        let schedule = run.schedule.clone();
        let id = run.id.clone();
        // A schedule that fires once is spent when its time's message reached
        // the conversation. One the agent ran now still has its time ahead,
        // and one whose run was skipped or failed before its message went
        // stays, and its menu row says why.
        let spent = run.cause == RunCause::Time && run.turn.is_some();
        let ms = u64::try_from(self.services().control.now().as_millisecond() - run.at.as_millisecond()).unwrap_or(0);
        let outcome = match run.state {
            RunState::Running => "running",
            RunState::Done => "done",
            RunState::Notified => "notified",
            RunState::Skipped => "skipped",
            RunState::Failed => "failed",
        };
        if let Err(error) = self.services().control.update_schedule_run(run).await {
            tracing::warn!(event = "schedule_run.end_record_failed", %schedule, run = %id, %error, "a run's end was not recorded");
        }
        tracing::info!(
            event = "schedule_run.ended",
            user = %self.user(),
            %schedule,
            run = %id,
            %conversation,
            outcome,
            ms,
            "a schedule's run ended"
        );
        if spent {
            self.delete_spent_once(conversation, &schedule).await;
        }
        self.schedules_changed(conversation);
        self.run_ended(&schedule).await;
    }

    /// Deletes the schedule `id` of `conversation` when it fires once.
    async fn delete_spent_once(&self, conversation: &ConversationId, id: &ScheduleId) {
        let spent = match self.stored_schedule(conversation, id).await {
            Ok(schedule) => matches!(schedule.trigger, Trigger::Time { timing: Timing::Once { .. } }),
            Err(_) => false,
        };
        if spent && let Err(error) = self.delete_schedule(conversation, id).await {
            tracing::warn!(event = "schedule.once_not_deleted", %conversation, schedule = %id, %error, "a spent once schedule stays");
        }
    }

    /// The conversation's tree started or stopped working, or closed: each
    /// running run whose message went there ends once the turn it started
    /// has ended.
    pub(crate) fn follow_runs(&self, conversation: &ConversationId) {
        let shard = self.this();
        let conversation = conversation.clone();
        self.tasks().spawn_local(async move {
            shard.settle_runs(&conversation).await;
        });
    }

    async fn settle_runs(&self, conversation: &ConversationId) {
        let runs = match self.services().control.running_runs_in(conversation.clone()).await {
            Ok(runs) => runs,
            Err(error) => {
                tracing::warn!(event = "schedule_runs.read_failed", %conversation, %error, "the running runs were not read");
                return;
            }
        };
        for mut run in runs {
            let Some(turn) = run.turn.clone() else {
                continue;
            };
            let end = match turn_end(self.agent(), conversation, &turn).await {
                Ok(Some(end)) => end,
                Ok(None) => continue,
                Err(error) => {
                    // The run stays running; the next settle of the
                    // conversation reads it again.
                    tracing::warn!(event = "schedule_run.turn_read_failed", %conversation, run = %run.id, %error, "a run's turn was not read");
                    continue;
                }
            };
            match end {
                // A run whose agent ran `demi notify send` is Notified.
                TurnEnd::Answered(line) => {
                    run.state = if run.notified { RunState::Notified } else { RunState::Done };
                    run.line = line;
                }
                TurnEnd::Failed { message, code } => {
                    run.state = RunState::Failed;
                    run.note = Some(RunNote::TurnFailed { message, code });
                }
            }
            self.end_run(run, conversation).await;
        }
    }

    /// The runs a run of the backend before left running: each whose
    /// conversation no tree holds now ends as its checkpoint says; the
    /// others end with their turns.
    pub(crate) async fn settle_left_runs(&self) {
        let runs = match self.services().control.running_runs(self.user().clone()).await {
            Ok(runs) => runs,
            Err(error) => {
                tracing::warn!(event = "schedule_runs.left_read_failed", user = %self.user(), %error, "the runs left running were not read");
                return;
            }
        };
        let conversations: std::collections::BTreeSet<ConversationId> =
            runs.into_iter().map(|(_, conversation)| conversation).collect();
        for conversation in conversations {
            // A turn the shutdown interrupted resumes, which ends it; one
            // that lost its message failed to start.
            if let Err(error) = self.restore_tree(&conversation).await {
                tracing::warn!(event = "schedule_run.conversation_open_failed", %conversation, %error, "a run's conversation did not open");
                continue;
            }
            self.settle_runs(&conversation).await;
        }
    }
}

/// What started a run, as the log names it.
fn cause_name(cause: RunCause) -> &'static str {
    match cause {
        RunCause::Time => "time",
        RunCause::Change => "watch",
        RunCause::Url => "url",
        RunCause::Now => "now",
    }
}

/// Why a run was skipped, as the log names it.
fn note_name(note: &RunNote) -> &'static str {
    match note {
        RunNote::Offline { .. } => "offline",
        _ => "other",
    }
}

/// How a run's turn ended.
enum TurnEnd {
    /// With the start of its last answer, none when it gave none, or when
    /// the user stopped it before one.
    Answered(Option<String>),
    /// With its error block's message and code.
    Failed { message: String, code: Option<String> },
}

/// How the turn `turn` of the conversation's root ended, from its blocks
/// through the next message wherever they are (`AgentSession::turn_blocks`);
/// none while its message waits or it runs.
async fn turn_end<H: demi_agent_tools::HostResolver>(
    agent: &Rc<AgentServer<H>>,
    conversation: &ConversationId,
    turn: &TurnId,
) -> Result<Option<TurnEnd>, StoreError> {
    let Some(tree) = agent.tree(&root_of(conversation)) else {
        return Ok(None);
    };
    let session = tree.root().session();
    let resting = session.phase() == SessionPhase::Idle;
    Ok(session.turn_blocks(turn).await?.and_then(|blocks| turn_end_in(&blocks, resting)))
}

/// How a turn ended from `blocks`, the blocks after its message; none while
/// it runs: it is not `resting` and no other message follows it.
fn turn_end_in(blocks: &[Block], resting: bool) -> Option<TurnEnd> {
    let later = blocks.iter().position(|block| matches!(block, Block::User(_)));
    // A turn followed by another's message has ended; the last one ends
    // when the root rests.
    if later.is_none() && !resting {
        return None;
    }
    let end = later.unwrap_or(blocks.len());
    let mut answer = None;
    let mut failure = None;
    for block in &blocks[..end] {
        match block {
            Block::Text(text) if !demi_shared_types::is_blank(&text.text) => {
                answer = Some(text.text.clone());
                failure = None;
            }
            Block::Error(error) if !error.outside_turn => failure = Some((error.message.clone(), error.code.clone())),
            Block::Response(_) => failure = None,
            _ => {}
        }
    }
    Some(match failure {
        Some((message, code)) => TurnEnd::Failed { message, code },
        None => TurnEnd::Answered(answer.map(|answer| {
            answer
                .chars()
                .take(demi_web_api_protocol::push::ANSWER_START_CHARS)
                .collect()
        })),
    })
}
