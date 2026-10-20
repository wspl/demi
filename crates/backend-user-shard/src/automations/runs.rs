//! One run of an automation.
//!
//! A run is recorded as it fires, before anything else, so it never starts
//! twice.
//!
//! It is skipped when:
//!
//! - its target's device is offline, after the restart hold for one the
//!   backend's own restart disconnected;
//! - its conversation is gone.
//!
//! Otherwise its message goes to a new conversation, filed under the
//! automation, or wakes the existing one, queued behind a turn that runs. The
//! run ends with the turn its message started.

use std::rc::Rc;

use demi_agent_server::AgentServer;
use demi_agent_store::StoreError;
use demi_backend_database::automations::{RunCause, StoredAutomation, StoredRun};
use demi_backend_database::conversation_index::Creation;
use demi_backend_host_access::root_of;
use demi_backend_page_sync::Part;
use demi_shared_types::{
    AutomationMark, AutomationRunId, Block, QueuedMessage, SessionPhase, Timestamp, TurnId, UserContentBlock,
};
use demi_web_api_protocol::automations::{AutomationTarget, PauseReason, RunNote, RunState, Trigger};
use demi_web_api_protocol::conversations::{ConversationTarget, CreateConversation, ModelChoice};
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};

use super::hooks::Call;
use super::message;
use crate::shard::Shard;

/// What fired a run, with what its message carries of it.
pub(crate) struct Fired {
    pub cause: RunCause,
    /// When the trigger fired: a time of the schedule, or now.
    pub at: Timestamp,
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
    pub(crate) fn time(at: Timestamp) -> Self {
        Self {
            cause: RunCause::Time,
            at,
            payload: Payload::None,
        }
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

fn new_run_id() -> AutomationRunId {
    AutomationRunId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty")
}

/// A run skipped as it fired, for `note`.
pub(crate) fn skipped(automation: &StoredAutomation, cause: RunCause, at: Timestamp, note: RunNote) -> StoredRun {
    StoredRun {
        id: new_run_id(),
        automation: automation.id.clone(),
        cause,
        at,
        state: RunState::Skipped,
        note: Some(note),
        line: None,
        conversation: None,
        turn: None,
        filed: false,
        unread: false,
        notified: false,
    }
}

/// Where a run works, once its target was checked.
pub(crate) enum Place {
    /// A new conversation there; with the model it starts with.
    New { target: ConversationTarget },
    Existing(ConversationId),
}

/// Why a target cannot run a run now.
pub(crate) enum Unrunnable {
    Offline(String),
    /// A paired device whose runner is given the rest of its wait to come
    /// back, after its connection dropped or the backend restarted
    /// (`crates/backend-runners/src/devices.rs`).
    Returning { device: DeviceId, rest: std::time::Duration },
    Gone(PauseReason),
}

impl Shard {
    /// Where `automation`'s target runs, or why it cannot now: its existing
    /// conversation archived or deleted, or its device offline.
    pub(crate) async fn place_of(&self, automation: &StoredAutomation) -> Result<Place, Unrunnable> {
        let control = &self.services().control;
        let (place, device) = match &automation.target {
            AutomationTarget::Existing { conversation_id } => {
                let record = match control.conversation(conversation_id.clone()).await {
                    Ok(Some(record)) if record.owner == *self.user() => record,
                    _ => return Err(Unrunnable::Gone(PauseReason::ConversationDeleted)),
                };
                if record.archived {
                    return Err(Unrunnable::Gone(PauseReason::ConversationArchived));
                }
                let device = self
                    .host_shard()
                    .resolve_target(&record)
                    .await
                    .ok()
                    .and_then(|target| target.device().cloned());
                (Place::Existing(record.id), device)
            }
            AutomationTarget::New { target } => {
                let device = match target {
                    ConversationTarget::Cloud { .. } => None,
                    ConversationTarget::Device { device_id, .. } => Some(device_id.clone()),
                    ConversationTarget::Workspace { workspace_id } => control
                        .workspace(workspace_id.clone())
                        .await
                        .ok()
                        .flatten()
                        .map(|workspace| workspace.device),
                };
                (Place::New { target: target.clone() }, device)
            }
        };
        if let Some(device) = device
            && let Some(name) = self.offline_device(&device).await
        {
            return Err(match self.devices().returning(&device) {
                Some(rest) => Unrunnable::Returning { device, rest },
                None => Unrunnable::Offline(name),
            });
        }
        Ok(place)
    }

    /// Where `automation`'s target runs, as `place_of` answers, once a
    /// returning device is back or its wait is over.
    pub(crate) async fn place_after_return(&self, automation: &StoredAutomation) -> Result<Place, Unrunnable> {
        match self.place_of(automation).await {
            Err(Unrunnable::Returning { device, rest }) => {
                self.devices().returned(&device, rest).await;
                match self.place_of(automation).await {
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

    /// The automation's run whose message the conversation works for, with when
    /// it came.
    ///
    /// The run covers every turn of the conversation's tree from that message
    /// until the user next writes to the conversation, whatever the run's state.
    /// None while the user's own message is the latest, or for a conversation no
    /// tree holds.
    pub(crate) fn automation_input(&self, conversation: &ConversationId) -> Option<(AutomationMark, Timestamp)> {
        let root = root_of(conversation);
        self.agent().node(&root, &root)?.session().automation_input()
    }

    /// When the message of a run a URL started came, while the conversation
    /// works for it: each credential read asks then. A run whose record is
    /// gone counts as one, since what started it is no longer known.
    pub(crate) async fn url_message_at(&self, conversation: &ConversationId) -> Option<Timestamp> {
        let (mark, at) = self.automation_input(conversation)?;
        match self.services().control.automation_run(mark.run_id).await {
            Ok(Some(run)) if run.cause != RunCause::Url => None,
            _ => Some(at),
        }
    }

    /// Fires a run of `automation`: records it, skipped when its target
    /// cannot run it, and otherwise carries it in a task of its own.
    pub(crate) async fn fire(&self, automation: &StoredAutomation, fired: Fired) {
        let control = &self.services().control;
        let place = self.place_of(automation).await;
        let note = match &place {
            // A new conversation starts with the automation's model, and
            // without one none starts.
            Ok(Place::New { .. }) if automation.model.is_none() => Some(RunNote::NoModel),
            Ok(_) | Err(Unrunnable::Returning { .. }) => None,
            Err(Unrunnable::Offline(device)) => Some(RunNote::Offline { device: device.clone() }),
            Err(Unrunnable::Gone(reason)) => {
                self.automation_target_gone(&target_conversation(automation), *reason).await;
                Some(RunNote::TargetGone)
            }
        };
        let mut run = StoredRun {
            id: new_run_id(),
            automation: automation.id.clone(),
            cause: fired.cause,
            at: fired.at,
            state: RunState::Running,
            note: None,
            line: None,
            conversation: None,
            turn: None,
            filed: false,
            unread: false,
            notified: false,
        };
        let skipped = note.is_some();
        if let Some(note) = note {
            run.state = RunState::Skipped;
            run.note = Some(note);
        }
        if let Err(error) = control.insert_automation_run(run.clone()).await {
            tracing::error!(automation = %automation.id, "a run was not recorded, and did not start: {error}");
            return;
        }
        self.mark(Part::Automations);
        if skipped {
            return;
        }
        let shard = self.this();
        let automation = automation.clone();
        self.tasks().spawn_local(async move {
            // A returning device is waited for in this task, the run
            // recorded already, so the scheduler goes on and never starts it
            // twice; still away after the wait, it is skipped.
            let place = match place {
                Err(Unrunnable::Returning { .. }) => shard.place_after_return(&automation).await,
                place => place,
            };
            let place = match place {
                Ok(place) => place,
                Err(unrunnable) => {
                    run.state = RunState::Skipped;
                    run.note = Some(match unrunnable {
                        Unrunnable::Gone(reason) => {
                            shard.automation_target_gone(&target_conversation(&automation), reason).await;
                            RunNote::TargetGone
                        }
                        Unrunnable::Offline(device) => RunNote::Offline { device },
                        Unrunnable::Returning { device, .. } => RunNote::Offline { device: device.to_string() },
                    });
                    shard.end_run(run).await;
                    return;
                }
            };
            if let Err(message) = shard.carry(&automation, &mut run, place, fired).await {
                run.state = RunState::Failed;
                run.note = Some(RunNote::TurnFailed { message, code: None });
                shard.end_run(run).await;
            }
        });
    }

    /// Starts the run's turn: makes its new conversation, filed under the
    /// automation, writes a URL's bodies into the conversation's scratch
    /// directory, and sends the message, which waits in the queue behind a
    /// turn that runs.
    async fn carry(
        &self,
        automation: &StoredAutomation,
        run: &mut StoredRun,
        place: Place,
        fired: Fired,
    ) -> Result<(), String> {
        let control = &self.services().control;
        let conversation = match place {
            Place::Existing(conversation) => conversation,
            Place::New { target } => {
                let id = ConversationId::try_from(uuid::Uuid::new_v4().to_string())
                    .expect("a UUID is not empty");
                let model = automation.model.clone();
                let created = self
                    .create_conversation(CreateConversation {
                        id: id.clone(),
                        title: None,
                        pinned: None,
                        model: model.as_ref().map(|model| ModelChoice {
                            provider_id: model.provider_id.clone(),
                            model_id: model.model_id.clone(),
                        }),
                        thinking_effort: model.as_ref().and_then(|model| model.thinking_effort.clone()),
                        service_tier_id: model.as_ref().map(|model| model.service_tier_id.clone()),
                        target: Some(target),
                        workflow: None,
                    })
                    .await
                    .map_err(|refusal| refusal.to_string())?;
                let Creation::Created(record) = created else {
                    return Err("the run's conversation could not be made".into());
                };
                run.filed = true;
                record.id
            }
        };
        let turn = TurnId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        run.conversation = Some(conversation.clone());
        run.turn = Some(turn.clone());
        control.update_automation_run(run.clone()).await.map_err(|error| error.to_string())?;
        self.mark(Part::Conversation(conversation.clone()));
        self.mark(Part::ConversationOrder);
        self.mark(Part::Automations);
        let text = match fired.payload {
            Payload::None => message::text(automation, fired.cause, fired.at, &message::Extra::None),
            Payload::Change { before, now } => message::text(
                automation,
                fired.cause,
                fired.at,
                &message::Extra::Change { before, now },
            ),
            Payload::Calls(calls) => {
                let written = self.write_bodies(&conversation, &run.id, &calls).await?;
                message::text(automation, fired.cause, fired.at, &message::Extra::Calls { calls, written })
            }
        };
        self.restore_tree(&conversation).await?;
        let tree = self
            .agent()
            .tree(&root_of(&conversation))
            .ok_or("the conversation's tree closed before the message")?;
        tree.root()
            .session()
            .send_message(QueuedMessage {
                comments: Vec::new(),
                id: turn,
                content: vec![UserContentBlock::Text { text }],
                automation: Some(AutomationMark {
                    automation_id: automation.id.clone(),
                    run_id: run.id.clone(),
                    name: automation.name.clone(),
                    at: fired.at,
                }),
            })
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Records the run's end and tells the pages; a URL's calls that came
    /// while it worked start the next run.
    pub(crate) async fn end_run(&self, run: StoredRun) {
        let automation = run.automation.clone();
        if let Err(error) = self.services().control.update_automation_run(run).await {
            tracing::warn!(%automation, "a run's end was not recorded: {error}");
        }
        self.mark(Part::Automations);
        self.rearm_automations();
        self.run_ended(&automation).await;
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
                tracing::warn!(%conversation, "the running runs were not read: {error}");
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
                    tracing::warn!(%conversation, "a run's turn was not read: {error}");
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
            self.end_run(run).await;
        }
    }

    /// The runs a run of the backend before left running: each whose
    /// conversation no tree holds now ends as its checkpoint says; the
    /// others end with their turns.
    pub(crate) async fn settle_left_runs(&self) {
        let runs = match self.services().control.running_runs(self.user().clone()).await {
            Ok(runs) => runs,
            Err(error) => {
                tracing::warn!(user = %self.user(), "the runs left running were not read: {error}");
                return;
            }
        };
        let conversations: std::collections::BTreeSet<ConversationId> =
            runs.into_iter().filter_map(|run| run.conversation).collect();
        for conversation in conversations {
            // A turn the shutdown interrupted resumes, which ends it; one
            // that lost its message failed to start.
            if let Err(error) = self.restore_tree(&conversation).await {
                tracing::warn!(%conversation, "a run's conversation did not open: {error}");
                continue;
            }
            self.settle_runs(&conversation).await;
        }
    }
}

/// The conversation an existing target names; a new one's has none, and a
/// gone target is only ever an existing one.
fn target_conversation(automation: &StoredAutomation) -> ConversationId {
    match &automation.target {
        AutomationTarget::Existing { conversation_id } => conversation_id.clone(),
        AutomationTarget::New { .. } => unreachable!("only an existing conversation is gone"),
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
