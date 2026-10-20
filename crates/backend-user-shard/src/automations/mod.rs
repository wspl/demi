//! The user's automations in their shard: what the Automations page and `demi
//! automation` read and change, the scheduler that owns every trigger, a run
//! from its trigger to its turn's end, the change watches, and the calls of the
//! URL triggers.
//!
//! The scheduler is one task per shard. It reads the user's automations when
//! the shard starts and whenever one changes, fires what is due, and sleeps
//! until the nearest next time or the next change; nothing polls the table.

pub(crate) mod commands;
pub(crate) mod holidays;
mod hooks;
mod message;
mod runs;
pub(crate) mod schedule;
mod watch;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use base64::Engine;
use demi_backend_database::StorageError;
use demi_backend_database::automations::{StoredAutomation, StoredRun};
use demi_backend_host_access::transition::ChangeRefusal;
use demi_backend_page_sync::Part;
use demi_backend_providers::vault::seal::Row;
use demi_shared_types::{AutomationId, Timestamp};
use demi_web_api_protocol::automations::{
    AutomationDto, AutomationPatch, AutomationRunDto, AutomationTarget, MonthDay, NEW_NAME, RUNS_SHOWN,
    Repeat, RepeatUnit, Schedule, Task, Trigger, Weekday,
};
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::ids::ConversationId;
use rand::RngCore;
use tokio::sync::Notify;

use crate::shard::Shard;
use schedule::{Plan, PlanError};

pub use hooks::{CALLS_PER_MINUTE, HookAnswer, HookCall, HookLimits, check_unknown};

/// The countries and regions whose holidays a task's working days can
/// follow.
pub fn holiday_countries() -> Vec<demi_web_api_protocol::automations::HolidayCountry> {
    holidays::countries()
}

/// What the shard keeps of its automations between their records' reads.
#[derive(Default)]
pub(crate) struct Automations {
    /// Rearms the scheduler: a change leaves a permit, so a change made
    /// while it reads is not missed.
    wake: Notify,
    /// What each automation has under way.
    pending: RefCell<HashMap<AutomationId, Pending>>,
    /// When each change watch last tried its command, recorded or not, as
    /// for an offline device, from which its next check counts.
    tried: RefCell<HashMap<AutomationId, Timestamp>>,
}

/// What an automation has under way in the shard: a URL's burst waiting out its
/// quiet, and a change watch's check.
///
/// Pausing the automation, editing its trigger or deleting it drops both, which
/// stops their tasks, and moves its generation on, so what either finishes after
/// is not recorded. The shard's end drops them all.
#[derive(Default)]
struct Pending {
    generation: u64,
    burst: Option<hooks::Burst>,
    /// Stops the check under way when dropped: its command ends on its
    /// Host, and its task records nothing.
    check: Option<tokio_util::sync::DropGuard>,
}

/// Why a change of an automation was refused.
#[derive(Debug, thiserror::Error)]
pub enum AutomationError {
    #[error("No such automation.")]
    NotFound,
    #[error("{0}")]
    Cron(String),
    #[error("{0}")]
    TimeZone(String),
    #[error("{0}")]
    Holidays(String),
    #[error("The automation has no task {0}.")]
    NoTask(String),
    #[error("The automation has several tasks: give the one to run.")]
    TaskNeeded,
    #[error("A single trigger has no tasks.")]
    NoTasks,
    #[error("The task has no message to send.")]
    NoMessage,
    #[error("The conversation is not one of your active conversations.")]
    ConversationUnavailable,
    #[error("A limit on runs a day is for a change watch or a URL.")]
    LimitNotTaken,
    #[error("Only a URL trigger has a secret.")]
    NotUrl,
    #[error("The conversation is not filed under an automation's run.")]
    NotFiled,
    #[error(transparent)]
    Change(#[from] ChangeRefusal),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl From<PlanError> for AutomationError {
    fn from(error: PlanError) -> Self {
        match error {
            PlanError::TimeZone(_) => Self::TimeZone(error.to_string()),
            PlanError::Cron { .. } => Self::Cron(error.to_string()),
            PlanError::Holidays(_) => Self::Holidays(error.to_string()),
        }
    }
}

/// A new secret: 32 random bytes in URL-safe base64.
fn new_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// 128 random bits as 32 hexadecimal digits.
fn random_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A new automation's id.
fn new_automation_id() -> AutomationId {
    AutomationId::try_from(random_id()).expect("32 digits are not empty")
}

/// A new task at the cycle's first moment, Monday, the 1st, in January, at
/// 09:00, as Add Task makes it.
pub(crate) fn new_task(prompt: String) -> Task {
    Task {
        id: random_id(),
        time: "09:00".into(),
        days: vec![Weekday::Mon],
        day: MonthDay::Date { date: 1 },
        month: 1,
        holidays: None,
        prompt,
    }
}

impl Shard {
    /// Whether work the automation started at `generation` may still record
    /// what it found: nothing paused it, edited its trigger or deleted it
    /// since.
    fn automation_current(&self, id: &AutomationId, generation: u64) -> bool {
        self.automations()
            .pending
            .borrow()
            .get(id)
            .is_some_and(|pending| pending.generation == generation)
    }

    /// Drops what the automation has under way, its burst's wait and its
    /// watch's check, and moves its generation on.
    fn drop_pending(&self, id: &AutomationId) {
        let (burst, check) = {
            let mut pending = self.automations().pending.borrow_mut();
            let entry = pending.entry(id.clone()).or_default();
            entry.generation += 1;
            (entry.burst.take(), entry.check.take())
        };
        // Dropped outside the borrow: each stops its task.
        drop((burst, check));
    }

    /// The scheduler's wake-up.
    fn rearm_automations(&self) {
        self.automations().wake.notify_one();
    }

    /// The user's automations changed: every page receives them, and the
    /// scheduler looks at their times again.
    pub(crate) fn automations_changed(&self) {
        self.mark(Part::Automations);
        self.rearm_automations();
    }

    /// The user's automations as the page lists them, newest first.
    pub async fn automation_list(&self) -> Result<Vec<AutomationDto>, StorageError> {
        let stored = self.services().control.automations(self.user().clone()).await?;
        let mut listed = Vec::with_capacity(stored.len());
        for automation in stored {
            listed.push(self.automation_dto(automation).await?);
        }
        Ok(listed)
    }

    /// The user's automation `id`, as it is stored.
    pub(crate) async fn stored_automation(&self, id: &AutomationId) -> Result<StoredAutomation, AutomationError> {
        self.services()
            .control
            .automation(self.user().clone(), id.clone())
            .await?
            .ok_or(AutomationError::NotFound)
    }

    async fn automation_dto(&self, automation: StoredAutomation) -> Result<AutomationDto, StorageError> {
        let runs = self
            .services()
            .control
            .automation_runs(automation.id.clone(), RUNS_SHOWN)
            .await?;
        let next_at = self.next_fire(&automation).await?;
        Ok(AutomationDto {
            id: automation.id,
            name: automation.name,
            trigger: automation.trigger,
            time_zone: automation.time_zone,
            max_runs_per_day: automation.max_runs_per_day,
            target: automation.target,
            prompt: automation.prompt,
            model: automation.model,
            paused: automation.paused,
            paused_because: automation.paused_because,
            stripe_signing: automation.stripe_signing,
            next_at,
            created_at: automation.created_at,
            runs: runs.into_iter().map(run_dto).collect(),
        })
    }

    /// When the automation next fires by itself, as its page says it.
    async fn next_fire(&self, automation: &StoredAutomation) -> Result<Option<Timestamp>, StorageError> {
        if automation.paused {
            return Ok(None);
        }
        let now = self.services().control.now();
        Ok(match &automation.trigger {
            Trigger::Time { schedule } => {
                let Ok(plan) = Plan::new(schedule, &automation.time_zone, automation.schedule_since.to_jiff()) else {
                    return Ok(None);
                };
                let last = self.services().control.automation_last_fired(automation.id.clone()).await?;
                let after = last.map_or(now, |last| last.max(now));
                plan.next_after(after.to_jiff()).map(Timestamp::truncate)
            }
            Trigger::Change { every_minutes, .. } => {
                let due = Timestamp::truncate(self.watch_due(automation, *every_minutes));
                // At its limit it waits for the next day.
                Some(match self.limit_reached(automation, now).await? {
                    Some(tomorrow) => tomorrow.max(due),
                    None => due,
                })
            }
            Trigger::Url {} => None,
        })
    }

    /// `POST /automations`, and `demi automation create`: a new automation from
    /// its defaults, with `patch` over them.
    ///
    /// The defaults are *New Automation*, paused, every day with one task at
    /// 09:00 in the user's time zone and no message, in a new conversation on
    /// the Cloud. A URL trigger gets its
    /// secret at once; the answer carries it when there is one, which the agent's
    /// command prints once.
    pub async fn create_automation(
        &self,
        patch: AutomationPatch,
    ) -> Result<(AutomationDto, Option<String>), AutomationError> {
        let control = &self.services().control;
        let now = control.now();
        let names: HashSet<String> = control
            .automations(self.user().clone())
            .await?
            .into_iter()
            .map(|automation| automation.name)
            .collect();
        let name = (1..)
            .map(|number| match number {
                1 => NEW_NAME.to_owned(),
                number => format!("{NEW_NAME} ({number})"),
            })
            .find(|name| !names.contains(name))
            .expect("the numbers run out after the names");
        let reported = demi_backend_runners::command_context::reported(control, self.user()).await?;
        let defaults = StoredAutomation {
            id: new_automation_id(),
            user: self.user().clone(),
            name,
            trigger: Trigger::Time {
                schedule: Schedule::Repeat {
                    repeat: Repeat {
                        every: 1,
                        unit: RepeatUnit::Days,
                        start: None,
                    },
                    tasks: vec![new_task(String::new())],
                },
            },
            time_zone: match schedule::zone(&reported.locale.time_zone) {
                Ok(_) => reported.locale.time_zone,
                Err(_) => "UTC".into(),
            },
            max_runs_per_day: None,
            target: AutomationTarget::New {
                target: ConversationTarget::Cloud { path: None },
            },
            prompt: String::new(),
            // The model the user's next new conversation outside projects
            // starts with, as the composer's; none before the first choice.
            model: control.preferences(self.user().clone()).await?.last_model,
            paused: true,
            paused_because: None,
            schedule_since: now,
            has_secret: false,
            stripe_signing: false,
            watch: None,
            created_at: now,
            updated_at: now,
        };
        self.save_automation(defaults, patch, true).await
    }

    /// `PATCH /automations/:id`, and `demi automation edit`.
    pub async fn change_automation(
        &self,
        id: &AutomationId,
        patch: AutomationPatch,
    ) -> Result<(AutomationDto, Option<String>), AutomationError> {
        let stored = self.stored_automation(id).await?;
        self.save_automation(stored, patch, false).await
    }

    /// Applies `patch` to `automation`, checks the whole, and writes it.
    async fn save_automation(
        &self,
        mut automation: StoredAutomation,
        patch: AutomationPatch,
        new: bool,
    ) -> Result<(AutomationDto, Option<String>), AutomationError> {
        let control = &self.services().control;
        let now = control.now();
        let before = automation.clone();
        if let Some(name) = patch.name {
            automation.name = name.into_string();
        }
        if let Some(trigger) = patch.trigger {
            automation.trigger = trigger;
        }
        if let Some(zone) = patch.time_zone {
            automation.time_zone = zone;
        }
        if let Some(limit) = patch.max_runs_per_day {
            automation.max_runs_per_day = limit;
        }
        if let Some(target) = patch.target {
            automation.target = target;
        }
        if let Some(prompt) = patch.prompt {
            automation.prompt = prompt;
        }
        if let Some(model) = patch.model {
            if let Some(settings) = &model {
                self.chosen_selection(settings).await?;
            }
            automation.model = model;
        }
        if let Some(paused) = patch.paused {
            automation.paused = paused;
            if !paused {
                automation.paused_because = None;
            }
        }
        // Each part is checked as a whole automation needs it.
        schedule::zone(&automation.time_zone)?;
        if let Trigger::Time { schedule } = &automation.trigger {
            Plan::new(schedule, &automation.time_zone, now.to_jiff())?;
        }
        // A task's place is checked even while it has no message or no
        // working day, so what the page shows is always a place the table has.
        for task in runs::tasks_of(&automation.trigger).unwrap_or_default() {
            if let Some(place) = &task.holidays
                && holidays::calendar(place).is_none()
            {
                return Err(PlanError::Holidays(place.country.clone()).into());
            }
        }
        if automation.max_runs_per_day.is_some() && !automation.trigger.takes_limit() {
            if patch.max_runs_per_day.flatten().is_some() {
                return Err(AutomationError::LimitNotTaken);
            }
            // A trigger that changed to a time drops the limit it had.
            automation.max_runs_per_day = None;
        }
        if before.target != automation.target || (!automation.paused && before.paused) || new {
            self.check_target(&automation.target).await?;
        }
        // A schedule counts from when it was set or turned on: what a paused
        // one would have fired never fires.
        if new || before.trigger != automation.trigger || before.time_zone != automation.time_zone
            || (before.paused && !automation.paused)
        {
            automation.schedule_since = now;
        }
        // A pause or another trigger drops what was pending, before the
        // change is written, so nothing from before it fires.
        let drops = !new && ((automation.paused && !before.paused) || before.trigger != automation.trigger);
        if drops {
            self.drop_pending(&automation.id);
        }
        let url = matches!(automation.trigger, Trigger::Url {});
        let mut secret_shown = None;
        let secret = if url && !automation.has_secret {
            let secret = new_secret();
            let sealed = self.seal_automation(&automation.id, false, secret.as_bytes());
            secret_shown = Some(secret);
            Some(Some(sealed))
        } else if !url && automation.has_secret {
            Some(None)
        } else {
            None
        };
        let stripe = match patch.stripe_secret {
            Some(Some(secret)) => Some(Some(self.seal_automation(&automation.id, true, secret.trim().as_bytes()))),
            Some(None) => Some(None),
            None if !url && automation.stripe_signing => Some(None),
            None => None,
        };
        automation.updated_at = now;
        control.put_automation(automation.clone(), secret, stripe).await?;
        if drops {
            control.drop_pending_calls(automation.id.clone()).await?;
        }
        // A watch whose command changed records its new output first.
        if let (Trigger::Change { command: old, .. }, Trigger::Change { command, .. }) =
            (&before.trigger, &automation.trigger)
            && old != command
        {
            control.set_watch_output(automation.id.clone(), None).await?;
        }
        self.automations().tried.borrow_mut().remove(&automation.id);
        self.automations_changed();
        let stored = self.stored_automation(&automation.id).await?;
        Ok((self.automation_dto(stored).await?, secret_shown))
    }

    /// Checks that an existing conversation a target names is one of the
    /// user's active ones.
    async fn check_target(&self, target: &AutomationTarget) -> Result<(), AutomationError> {
        if let AutomationTarget::Existing { conversation_id, .. } = target {
            let record = self.services().control.conversation(conversation_id.clone()).await?;
            if !record.is_some_and(|record| record.owner == *self.user() && !record.archived) {
                return Err(AutomationError::ConversationUnavailable);
            }
        }
        Ok(())
    }

    fn seal_automation(&self, id: &AutomationId, stripe: bool, plaintext: &[u8]) -> Vec<u8> {
        let key = self.services().vault.key();
        let row = if stripe {
            Row::AutomationStripeSecret(id.as_str())
        } else {
            Row::AutomationSecret(id.as_str())
        };
        key.seal(row, plaintext)
    }

    /// The automation's URL secret and Stripe signing secret, opened.
    async fn automation_secrets(&self, id: &AutomationId) -> Result<(Option<String>, Option<String>), StorageError> {
        let (secret, stripe) = self.services().control.automation_secrets(id.clone()).await?;
        let key = self.services().vault.key();
        let open = |row: Row<'_>, sealed: Option<Vec<u8>>| -> Result<Option<String>, StorageError> {
            sealed
                .map(|sealed| {
                    let bytes = key.open(row, &sealed).map_err(|_| StorageError::Corrupt {
                        table: "automations",
                        column: "secret",
                        reason: "a sealed secret does not open".into(),
                    })?;
                    String::from_utf8(bytes).map_err(|error| StorageError::Corrupt {
                        table: "automations",
                        column: "secret",
                        reason: error.to_string(),
                    })
                })
                .transpose()
        };
        Ok((
            open(Row::AutomationSecret(id.as_str()), secret)?,
            open(Row::AutomationStripeSecret(id.as_str()), stripe)?,
        ))
    }

    /// `GET /automations/:id/secret`, for the page's Show; logged by the
    /// automation, never with the value.
    pub async fn automation_secret(&self, id: &AutomationId) -> Result<String, AutomationError> {
        let automation = self.stored_automation(id).await?;
        let (secret, _) = self.automation_secrets(&automation.id).await?;
        let secret = secret.ok_or(AutomationError::NotUrl)?;
        tracing::info!(automation = %automation.id, "a page revealed an automation's secret");
        Ok(secret)
    }

    /// `POST /automations/:id/secret`: Regenerate Secret; the old one is
    /// refused from then on.
    pub async fn regenerate_automation_secret(&self, id: &AutomationId) -> Result<String, AutomationError> {
        let automation = self.stored_automation(id).await?;
        if !matches!(automation.trigger, Trigger::Url {}) {
            return Err(AutomationError::NotUrl);
        }
        let secret = new_secret();
        let sealed = self.seal_automation(&automation.id, false, secret.as_bytes());
        self.services()
            .control
            .put_automation(automation, Some(Some(sealed)), None)
            .await?;
        self.automations_changed();
        Ok(secret)
    }

    /// `DELETE /automations/:id`, and `demi automation delete`: its runs'
    /// records go; their conversations stay and join the sidebar.
    pub async fn delete_automation(&self, id: &AutomationId) -> Result<(), AutomationError> {
        self.drop_pending(id);
        let unfiled = self
            .services()
            .control
            .delete_automation(self.user().clone(), id.clone())
            .await?
            .ok_or(AutomationError::NotFound)?;
        let gone = self.automations().pending.borrow_mut().remove(id);
        drop(gone);
        // Its URL's call count goes with it.
        self.services().hook_limits.forget(id);
        self.automations().tried.borrow_mut().remove(id);
        for conversation in unfiled {
            self.mark(Part::Conversation(conversation));
        }
        self.mark(Part::ConversationOrder);
        self.automations_changed();
        Ok(())
    }

    /// Run Now, and `demi automation run`: a run whatever the schedule, the
    /// daily limit or whether the automation is paused, of the repeat's task
    /// `task`, which may be left out when the repeat has only one.
    pub async fn run_automation_now(
        &self,
        id: &AutomationId,
        task: Option<String>,
    ) -> Result<StoredAutomation, AutomationError> {
        let automation = self.stored_automation(id).await?;
        let task = match (runs::tasks_of(&automation.trigger), task) {
            (None, None) => None,
            (None, Some(_)) => return Err(AutomationError::NoTasks),
            (Some([only]), None) => Some(only),
            (Some(_), None) => return Err(AutomationError::TaskNeeded),
            (Some(tasks), Some(id)) => Some(tasks.iter().find(|each| each.id == id).ok_or(AutomationError::NoTask(id))?),
        };
        if task.is_some_and(|task| demi_shared_types::is_blank(&task.prompt)) {
            return Err(AutomationError::NoMessage);
        }
        let at = self.services().control.now();
        self.fire(
            &automation,
            runs::Fired {
                cause: demi_backend_database::automations::RunCause::Now,
                at,
                task: task.map(|task| task.id.clone()),
                payload: runs::Payload::None,
            },
        )
        .await;
        Ok(automation)
    }

    /// Move to Conversations: the run's conversation joins the sidebar.
    pub async fn move_to_conversations(&self, conversation: &ConversationId) -> Result<(), AutomationError> {
        self.host_shard()
            .owned_conversation(conversation)
            .await
            .map_err(|_| AutomationError::NotFiled)?;
        if !self.services().control.unfile_conversation(conversation.clone()).await? {
            return Err(AutomationError::NotFiled);
        }
        self.mark(Part::Conversation(conversation.clone()));
        self.mark(Part::Automations);
        Ok(())
    }

    /// The user sent the conversation a message: a run's conversation joins
    /// the sidebar.
    pub(crate) async fn answered_by_user(&self, conversation: &ConversationId) {
        match self.services().control.unfile_conversation(conversation.clone()).await {
            Ok(true) => {
                self.mark(Part::Conversation(conversation.clone()));
                self.mark(Part::Automations);
            }
            Ok(false) => {}
            Err(error) => tracing::warn!(%conversation, "a run's conversation stays filed: {error}"),
        }
    }

    /// The conversation was archived or deleted: the automations that wake
    /// it pause, saying why.
    pub(crate) async fn automation_target_gone(
        &self,
        conversation: &ConversationId,
        reason: demi_web_api_protocol::automations::PauseReason,
    ) {
        match self
            .services()
            .control
            .conversation_gone_from_automations(conversation.clone(), reason)
            .await
        {
            Ok(true) => self.automations_changed(),
            Ok(false) => {}
            Err(error) => tracing::warn!(%conversation, "the automations of a gone conversation were not paused: {error}"),
        }
    }

    /// Fires the automations of the user as their times come, until the shard
    /// closes.
    ///
    /// It settles the runs a run of the backend before left running, then fires
    /// what is due, starts the change watches whose time came, and sleeps until
    /// the nearest next time or a change of the automations. A time that passed
    /// while the backend was down fires once.
    pub(crate) async fn follow_automations(self: Rc<Self>) {
        self.settle_left_runs().await;
        self.resume_pending_calls().await;
        loop {
            let next = match self.fire_due().await {
                Ok(next) => next,
                Err(error) => {
                    // The next change or the next start reads them again.
                    tracing::warn!(user = %self.user(), "the automations were not read: {error}");
                    None
                }
            };
            let now = self.services().control.now();
            let wait = next.map(|next| {
                Duration::try_from(next.to_jiff().duration_since(now.to_jiff())).unwrap_or(Duration::ZERO)
            });
            tokio::select! {
                () = async {
                    match wait {
                        Some(wait) => tokio::time::sleep(wait).await,
                        None => std::future::pending().await,
                    }
                } => {}
                () = self.automations().wake.notified() => {}
                () = self.closed() => {
                    // The shard ends: its bursts' waits and its checks stop.
                    let pending = std::mem::take(&mut *self.automations().pending.borrow_mut());
                    drop(pending);
                    return;
                }
            }
        }
    }

    /// Fires every automation whose time came, and answers the nearest next
    /// time.
    async fn fire_due(&self) -> Result<Option<Timestamp>, StorageError> {
        let control = &self.services().control;
        let now = control.now();
        let mut nearest: Option<Timestamp> = None;
        let mut keep = |at: Timestamp| nearest = Some(nearest.map_or(at, |nearest| nearest.min(at)));
        for automation in control.automations(self.user().clone()).await? {
            if automation.paused {
                continue;
            }
            match &automation.trigger {
                Trigger::Time { schedule } => {
                    let Ok(plan) = Plan::new(schedule, &automation.time_zone, automation.schedule_since.to_jiff())
                    else {
                        continue;
                    };
                    let last = control.automation_last_fired(automation.id.clone()).await?;
                    let mut after = last.unwrap_or(automation.schedule_since).to_jiff();
                    // Each task due runs as its own run, once for its
                    // latest missed time; in time order, so the last fired
                    // is the latest.
                    for (due, task) in plan.due(after, now.to_jiff()) {
                        self.fire(&automation, runs::Fired::time(Timestamp::truncate(due), task.map(str::to_owned)))
                            .await;
                        after = after.max(due);
                    }
                    if let Some(next) = plan.next_after(after) {
                        keep(Timestamp::truncate(next));
                    }
                }
                Trigger::Change { every_minutes, .. } => {
                    if self.automation_checking(&automation.id) {
                        continue;
                    }
                    let due = Timestamp::truncate(self.watch_due(&automation, *every_minutes));
                    let held = self.limit_reached(&automation, now).await?;
                    match held {
                        // The limit holds back the command only once it is
                        // due; until then nothing was skipped.
                        Some(tomorrow) if due <= now => {
                            self.count_held_back(&automation, 0).await?;
                            keep(tomorrow);
                        }
                        Some(_) => keep(due),
                        None if due <= now => self.start_watch(automation.clone()),
                        None => keep(due),
                    }
                }
                Trigger::Url {} => {}
            }
        }
        Ok(nearest)
    }

    /// When a change watch next runs its command: an interval after its
    /// last try, recorded or not, and at once before its first.
    fn watch_due(&self, automation: &StoredAutomation, every_minutes: u32) -> jiff::Timestamp {
        let recorded = automation.watch.as_ref().map(|watch| watch.checked_at);
        let tried = self.automations().tried.borrow().get(&automation.id).copied();
        match recorded.max(tried) {
            Some(last) => Plan::every(every_minutes, last.to_jiff())
                .next_after(last.to_jiff())
                .unwrap_or(last.to_jiff()),
            None => automation.schedule_since.to_jiff(),
        }
    }

    /// When the automation's daily limit is reached, the start of the next
    /// day, from which it runs again; none while runs are left today or it
    /// has no limit.
    async fn limit_reached(
        &self,
        automation: &StoredAutomation,
        now: Timestamp,
    ) -> Result<Option<Timestamp>, StorageError> {
        let Some(limit) = automation.max_runs_per_day else {
            return Ok(None);
        };
        let Ok(zone) = schedule::zone(&automation.time_zone) else {
            return Ok(None);
        };
        let (today, tomorrow) = schedule::day_bounds(now.to_jiff(), &zone);
        let ran = self
            .services()
            .control
            .automation_runs_since(automation.id.clone(), Timestamp::truncate(today))
            .await?;
        Ok((ran >= limit).then(|| Timestamp::truncate(tomorrow)))
    }

    /// Counts a trigger the daily limit held back in the day's one skipped
    /// run, *Skipped, the limit of 10 runs a day was reached, 7 calls
    /// since*, made by the first and grown by each later one.
    async fn count_held_back(&self, automation: &StoredAutomation, calls: u32) -> Result<(), StorageError> {
        let control = &self.services().control;
        let now = control.now();
        let Some(limit) = automation.max_runs_per_day else {
            return Ok(());
        };
        let Ok(zone) = schedule::zone(&automation.time_zone) else {
            return Ok(());
        };
        let (today, _) = schedule::day_bounds(now.to_jiff(), &zone);
        match control
            .automation_limit_run(automation.id.clone(), Timestamp::truncate(today))
            .await?
        {
            Some(mut run) => {
                if let Some(demi_web_api_protocol::automations::RunNote::Limit { calls: count, .. }) = &mut run.note {
                    *count += calls;
                }
                control.update_automation_run(run).await?;
            }
            None => {
                control
                    .insert_automation_run(runs::skipped(
                        automation,
                        runs::cause_of(&automation.trigger),
                        now,
                        demi_web_api_protocol::automations::RunNote::Limit { limit, calls },
                    ))
                    .await?;
            }
        }
        self.mark(Part::Automations);
        Ok(())
    }
}

/// Opens the shard of every user who has an automation, whose scheduler
/// then fires what a backend that was down missed and waits for the next
/// times.
pub async fn start_schedulers(
    control: &demi_backend_database::control::ControlService,
    shards: &crate::shard::Shards,
) -> Result<(), StorageError> {
    for user in control.users_with_automations().await? {
        // The shard's first call starts its scheduler; a shard that cannot
        // be reached starts it on its next call.
        if let Err(error) = shards.of(&user).call(|_, _| async {}).await {
            tracing::warn!(%user, "the automations' scheduler did not start: {error}");
        }
    }
    Ok(())
}

fn run_dto(run: StoredRun) -> AutomationRunDto {
    AutomationRunDto {
        id: run.id,
        at: run.at,
        task_id: run.task,
        state: run.state,
        note: run.note,
        line: run.line,
        conversation_id: run.conversation,
        unread: run.unread,
        filed: run.filed,
    }
}
