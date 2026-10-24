//! The schedules of the user's conversations in their shard: what a
//! conversation's metadata lists and `demi schedule` reads and changes, the
//! scheduler that owns every trigger, a run from its trigger to its turn's
//! end, the change watches, and the calls of the URL triggers.
//!
//! A schedule belongs to one conversation and wakes only it; the conversation's
//! deletion deletes it, and while the conversation is archived it wakes nothing,
//! which is read from the conversation's archived state and never written to
//! the schedule's own `paused`.
//!
//! The scheduler is one task per shard. It reads the user's schedules when
//! the shard starts and whenever one changes, fires what is due, and sleeps
//! until the nearest next time or the next change; nothing polls the table.

pub(crate) mod commands;
pub(crate) mod holidays;
mod hooks;
mod message;
mod runs;
pub(crate) mod plan;
mod watch;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use base64::Engine;
use demi_backend_database::StorageError;
use demi_backend_database::schedules::{StoredRun, StoredSchedule};
use demi_backend_page_sync::Part;
use demi_backend_providers::vault::seal::Row;
use demi_shared_types::{ScheduleId, Timestamp};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::schedules::{
    ConversationSchedule, IfBusy, LastRun, MonthDay, NEW_NAME, RUNS_SHOWN, Repeat, RepeatUnit, ScheduleDto,
    SchedulePatch, ScheduleRunDto, Task, Timing, Trigger, Weekday,
};
use rand::RngCore;
use tokio::sync::Notify;

use crate::shard::Shard;
use plan::{Plan, PlanError};

pub use hooks::{CALLS_PER_MINUTE, HookAnswer, HookCall, HookLimits};

/// What the shard keeps of its schedules between their records' reads.
#[derive(Default)]
pub(crate) struct Schedules {
    /// Rearms the scheduler: a change leaves a permit, so a change made
    /// while it reads is not missed.
    wake: Notify,
    /// What each schedule has under way.
    pending: RefCell<HashMap<ScheduleId, Pending>>,
    /// When each change watch last tried its command, recorded or not, as
    /// for an offline device, from which its next check counts.
    tried: RefCell<HashMap<ScheduleId, Timestamp>>,
}

/// What a schedule has under way in the shard: a URL's burst waiting out its
/// quiet, and a change watch's check.
///
/// Pausing the schedule, editing its trigger or deleting it drops both, which
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

/// Why a change of a schedule was refused.
#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
    #[error("This conversation has no such schedule.")]
    NotFound,
    #[error("{0}")]
    Cron(String),
    #[error("{0}")]
    TimeZone(String),
    #[error("{0}")]
    Holidays(String),
    #[error("The schedule has no task {0}.")]
    NoTask(String),
    #[error("The schedule has several tasks: give the one to run.")]
    TaskNeeded,
    #[error("A single trigger has no tasks.")]
    NoTasks,
    #[error("The task has no message to send.")]
    NoMessage,
    #[error("A limit on runs a day is for a change watch or a URL.")]
    LimitNotTaken,
    #[error("Only a URL trigger has a secret.")]
    NoSecret,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl From<PlanError> for ScheduleError {
    fn from(error: PlanError) -> Self {
        match error {
            PlanError::TimeZone(_) => Self::TimeZone(error.to_string()),
            PlanError::Cron { .. } => Self::Cron(error.to_string()),
            PlanError::Holidays(_) => Self::Holidays(error.to_string()),
        }
    }
}

/// What a write of a schedule is.
#[derive(Clone, Copy)]
enum Save {
    Create,
    /// A change, which may replace a URL trigger's secret.
    Edit { new_secret: bool },
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

/// A new schedule's id.
fn new_schedule_id() -> ScheduleId {
    ScheduleId::try_from(random_id()).expect("32 digits are not empty")
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
    /// Whether work the schedule started at `generation` may still record
    /// what it found: nothing paused it, edited its trigger or deleted it
    /// since.
    fn schedule_current(&self, id: &ScheduleId, generation: u64) -> bool {
        self.schedules()
            .pending
            .borrow()
            .get(id)
            .is_some_and(|pending| pending.generation == generation)
    }

    /// Drops what the schedule has under way, its burst's wait and its
    /// watch's check, and moves its generation on.
    fn drop_pending(&self, id: &ScheduleId) {
        let (burst, check) = {
            let mut pending = self.schedules().pending.borrow_mut();
            let entry = pending.entry(id.clone()).or_default();
            entry.generation += 1;
            (entry.burst.take(), entry.check.take())
        };
        // Dropped outside the borrow: each stops its task.
        drop((burst, check));
    }

    /// The scheduler's wake-up.
    fn rearm_schedules(&self) {
        self.schedules().wake.notify_one();
    }

    /// The conversation's schedules changed: its pages receive them in its
    /// metadata, and the scheduler looks at their times again.
    pub(crate) fn schedules_changed(&self, conversation: &ConversationId) {
        self.mark(Part::Conversation(conversation.clone()));
        self.mark(Part::Metadata(conversation.clone()));
        self.rearm_schedules();
    }

    /// The conversation's schedules as `demi schedule list` shows them,
    /// newest first.
    pub async fn schedule_list(&self, conversation: &ConversationId) -> Result<Vec<ScheduleDto>, StorageError> {
        let stored = self.services().control.conversation_schedules(conversation.clone()).await?;
        let mut listed = Vec::with_capacity(stored.len());
        for schedule in stored {
            listed.push(self.schedule_dto(schedule).await?);
        }
        Ok(listed)
    }

    /// The conversation's schedules as its metadata lists them, newest first.
    pub(crate) async fn conversation_schedules(
        &self,
        conversation: &ConversationId,
    ) -> Result<Vec<ConversationSchedule>, StorageError> {
        let stored = self.services().control.conversation_schedules(conversation.clone()).await?;
        let mut listed = Vec::with_capacity(stored.len());
        for schedule in stored {
            let next_at = self.next_fire(&schedule).await?;
            let last_run = self
                .services()
                .control
                .schedule_runs(schedule.id.clone(), 1)
                .await?
                .into_iter()
                .next()
                .map(|run| LastRun {
                    at: run.at,
                    state: run.state,
                    note: run.note,
                });
            listed.push(ConversationSchedule {
                id: schedule.id,
                name: schedule.name,
                trigger: schedule.trigger,
                time_zone: schedule.time_zone,
                paused: schedule.paused,
                next_at,
                last_run,
            });
        }
        Ok(listed)
    }

    /// The conversation's schedule `id`, as it is stored, while the shard's
    /// user owns the conversation; any other is not found, so neither the page
    /// nor an agent reaches another user's schedule by its id, which a URL
    /// trigger's address shows.
    pub(crate) async fn stored_schedule(
        &self,
        conversation: &ConversationId,
        id: &ScheduleId,
    ) -> Result<StoredSchedule, ScheduleError> {
        self.services()
            .control
            .owned_schedule(self.user().clone(), conversation.clone(), id.clone())
            .await?
            .ok_or(ScheduleError::NotFound)
    }

    async fn schedule_dto(&self, schedule: StoredSchedule) -> Result<ScheduleDto, StorageError> {
        let runs = self
            .services()
            .control
            .schedule_runs(schedule.id.clone(), RUNS_SHOWN)
            .await?;
        let next_at = self.next_fire(&schedule).await?;
        Ok(ScheduleDto {
            id: schedule.id,
            name: schedule.name,
            trigger: schedule.trigger,
            time_zone: schedule.time_zone,
            max_runs_per_day: schedule.max_runs_per_day,
            if_busy: schedule.if_busy,
            prompt: schedule.prompt,
            paused: schedule.paused,
            next_at,
            created_at: schedule.created_at,
            runs: runs.into_iter().map(run_dto).collect(),
        })
    }

    /// When the schedule next fires by itself, as its menu says it.
    async fn next_fire(&self, schedule: &StoredSchedule) -> Result<Option<Timestamp>, StorageError> {
        if !schedule.on() {
            return Ok(None);
        }
        let now = self.services().control.now();
        Ok(match &schedule.trigger {
            Trigger::Time { timing } => {
                let Ok(plan) = Plan::new(timing, &schedule.time_zone, schedule.since.to_jiff()) else {
                    return Ok(None);
                };
                let last = self.services().control.schedule_last_fired(schedule.id.clone()).await?;
                let after = last.map_or(now, |last| last.max(now));
                plan.next_after(after.to_jiff()).map(Timestamp::truncate)
            }
            Trigger::Change { every_minutes, .. } => {
                let due = Timestamp::truncate(self.watch_due(schedule, *every_minutes));
                // At its limit it waits for the next day.
                Some(match self.limit_reached(schedule, now).await? {
                    Some(tomorrow) => tomorrow.max(due),
                    None => due,
                })
            }
            Trigger::Url {} => None,
        })
    }

    /// `demi schedule create`: a new schedule of `conversation` from its
    /// defaults, with `patch` over them.
    ///
    /// The defaults are *New Schedule*, paused, every day with one task at
    /// 09:00 in the user's time zone and no message, which waits while the
    /// conversation is busy. A URL trigger gets its secret at once; the answer
    /// carries it when there is one.
    pub async fn create_schedule(
        &self,
        conversation: &ConversationId,
        patch: SchedulePatch,
    ) -> Result<(ScheduleDto, Option<String>), ScheduleError> {
        let control = &self.services().control;
        let now = control.now();
        let names: HashSet<String> = control
            .conversation_schedules(conversation.clone())
            .await?
            .into_iter()
            .map(|schedule| schedule.name)
            .collect();
        let name = (1..)
            .map(|number| match number {
                1 => NEW_NAME.to_owned(),
                number => format!("{NEW_NAME} ({number})"),
            })
            .find(|name| !names.contains(name))
            .expect("the numbers run out after the names");
        let reported = demi_backend_runners::command_context::reported(control, self.user()).await?;
        let defaults = StoredSchedule {
            id: new_schedule_id(),
            conversation: conversation.clone(),
            name,
            trigger: Trigger::Time {
                timing: Timing::Repeat {
                    repeat: Repeat {
                        every: 1,
                        unit: RepeatUnit::Days,
                        start: None,
                    },
                    tasks: vec![new_task(String::new())],
                },
            },
            time_zone: match plan::zone(&reported.locale.time_zone) {
                Ok(_) => reported.locale.time_zone,
                Err(_) => "UTC".into(),
            },
            max_runs_per_day: None,
            if_busy: IfBusy::Queue,
            prompt: String::new(),
            paused: true,
            since: now,
            conversation_archived: false,
            has_secret: false,
            watch: None,
            created_at: now,
            updated_at: now,
        };
        self.save_schedule(defaults, patch, Save::Create).await
    }

    /// `demi schedule edit`; with `new_secret`, a URL trigger's secret is
    /// replaced, and the old one stops working at once.
    pub async fn change_schedule(
        &self,
        conversation: &ConversationId,
        id: &ScheduleId,
        patch: SchedulePatch,
        new_secret: bool,
    ) -> Result<(ScheduleDto, Option<String>), ScheduleError> {
        let stored = self.stored_schedule(conversation, id).await?;
        self.save_schedule(stored, patch, Save::Edit { new_secret }).await
    }

    /// Applies `patch` to `schedule`, checks the whole, and writes it.
    async fn save_schedule(
        &self,
        mut schedule: StoredSchedule,
        patch: SchedulePatch,
        save: Save,
    ) -> Result<(ScheduleDto, Option<String>), ScheduleError> {
        let new = matches!(save, Save::Create);
        // Turning a schedule off or on changes nothing the checks below read,
        // so a schedule a later time zone or holiday table no longer takes can
        // still be turned off.
        let only_paused = !new && patch == SchedulePatch { paused: patch.paused, ..SchedulePatch::default() };
        let control = &self.services().control;
        let now = control.now();
        let before = schedule.clone();
        if let Some(name) = patch.name {
            schedule.name = name.into_string();
        }
        if let Some(trigger) = patch.trigger {
            schedule.trigger = trigger;
        }
        if let Some(zone) = patch.time_zone {
            schedule.time_zone = zone;
        }
        if let Some(limit) = patch.max_runs_per_day {
            schedule.max_runs_per_day = limit;
        }
        if let Some(if_busy) = patch.if_busy {
            schedule.if_busy = if_busy;
        }
        if let Some(prompt) = patch.prompt {
            schedule.prompt = prompt;
        }
        if let Some(paused) = patch.paused {
            schedule.paused = paused;
        }
        // Each part is checked as a whole schedule needs it.
        if !only_paused {
            plan::zone(&schedule.time_zone)?;
            if let Trigger::Time { timing } = &schedule.trigger {
                Plan::new(timing, &schedule.time_zone, now.to_jiff())?;
            }
            // A task's place is checked even while it has no message or no
            // working day, so what the schedule holds is always a place the table has.
            for task in runs::tasks_of(&schedule.trigger).unwrap_or_default() {
                if let Some(place) = &task.holidays
                    && holidays::calendar(place).is_none()
                {
                    return Err(PlanError::Holidays(place.country.clone()).into());
                }
            }
        }
        if schedule.max_runs_per_day.is_some() && !schedule.trigger.takes_limit() {
            if patch.max_runs_per_day.flatten().is_some() {
                return Err(ScheduleError::LimitNotTaken);
            }
            // A trigger that changed to a time drops the limit it had.
            schedule.max_runs_per_day = None;
        }
        // A schedule counts from when it was set or turned on: what a paused
        // one would have fired never fires.
        if new || before.trigger != schedule.trigger || before.time_zone != schedule.time_zone
            || (before.paused && !schedule.paused)
        {
            schedule.since = now;
        }
        // A pause or another trigger drops what was pending, before the
        // change is written, so nothing from before it fires.
        let drops = !new && ((schedule.paused && !before.paused) || before.trigger != schedule.trigger);
        if drops {
            self.drop_pending(&schedule.id);
        }
        let url = matches!(schedule.trigger, Trigger::Url {});
        let mut secret_shown = None;
        let replaced = matches!(save, Save::Edit { new_secret: true });
        if replaced && !url {
            return Err(ScheduleError::NoSecret);
        }
        let secret = if url && (!schedule.has_secret || replaced) {
            let secret = new_secret();
            let sealed = self.seal_schedule(&schedule.id, secret.as_bytes());
            secret_shown = Some(secret);
            Some(Some(sealed))
        } else if !url && schedule.has_secret {
            Some(None)
        } else {
            None
        };
        schedule.updated_at = now;
        control.put_schedule(schedule.clone(), secret).await?;
        if drops {
            control.drop_pending_calls(schedule.id.clone()).await?;
        }
        // A watch whose command changed, or that was turned on, records its
        // output first: a change while it was off fires nothing.
        if let (Trigger::Change { command: old, .. }, Trigger::Change { command, .. }) =
            (&before.trigger, &schedule.trigger)
            && (old != command || (before.paused && !schedule.paused))
        {
            control.set_watch_output(schedule.id.clone(), None).await?;
        }
        self.schedules().tried.borrow_mut().remove(&schedule.id);
        self.schedules_changed(&schedule.conversation);
        let stored = self.stored_schedule(&schedule.conversation, &schedule.id).await?;
        Ok((self.schedule_dto(stored).await?, secret_shown))
    }

    fn seal_schedule(&self, id: &ScheduleId, plaintext: &[u8]) -> Vec<u8> {
        self.services().vault.key().seal(Row::ScheduleSecret(id.as_str()), plaintext)
    }

    /// The schedule's URL secret, opened; none for a schedule that has none.
    /// It checks a call of the URL; the agent is shown it once, when it is made.
    pub(crate) async fn schedule_secret(&self, id: &ScheduleId) -> Result<Option<String>, StorageError> {
        let Some(sealed) = self.services().control.schedule_secret(id.clone()).await? else {
            return Ok(None);
        };
        let corrupt = |reason: String| StorageError::Corrupt {
            table: "schedules",
            column: "secret",
            reason,
        };
        let bytes = self
            .services()
            .vault
            .key()
            .open(Row::ScheduleSecret(id.as_str()), &sealed)
            .map_err(|_| corrupt("a sealed secret does not open".into()))?;
        String::from_utf8(bytes).map(Some).map_err(|error| corrupt(error.to_string()))
    }

    /// `demi schedule delete`, and the user's Delete: the schedule goes with
    /// its runs' records.
    pub async fn delete_schedule(&self, conversation: &ConversationId, id: &ScheduleId) -> Result<(), ScheduleError> {
        let schedule = self.stored_schedule(conversation, id).await?;
        self.forget_schedule(&schedule.id);
        if !self.services().control.delete_schedule(schedule.id.clone()).await? {
            return Err(ScheduleError::NotFound);
        }
        self.schedules_changed(conversation);
        Ok(())
    }

    /// What the shard holds of the schedule `id`, which is deleted: its
    /// burst and its check stop, and its URL's call count and its watch's
    /// last try go.
    fn forget_schedule(&self, id: &ScheduleId) {
        self.drop_pending(id);
        let gone = self.schedules().pending.borrow_mut().remove(id);
        drop(gone);
        self.services().hook_limits.forget(id);
        self.schedules().tried.borrow_mut().remove(id);
    }

    /// The ids of the conversation's schedules, which its deletion reads
    /// before its records go, to forget them once they have
    /// ([`Self::conversation_deleted_schedules`]).
    pub(crate) async fn conversation_schedule_ids(&self, conversation: &ConversationId) -> Vec<ScheduleId> {
        match self.services().control.conversation_schedules(conversation.clone()).await {
            Ok(schedules) => schedules.into_iter().map(|schedule| schedule.id).collect(),
            Err(error) => {
                tracing::warn!(event = "schedules.forget_failed", %conversation, %error, "the schedules of a deleted conversation were not read");
                Vec::new()
            }
        }
    }

    /// The conversation is deleted, and its schedules `ids` with it: what the
    /// shard holds of them goes, a watch's check under way among it.
    pub(crate) fn conversation_deleted_schedules(&self, ids: &[ScheduleId]) {
        for id in ids {
            self.forget_schedule(id);
        }
        self.rearm_schedules();
    }

    /// `demi schedule run`: a run whatever the timing, the daily limit or
    /// whether the schedule is paused, of the repeat's task `task`, which may
    /// be left out when the repeat has only one.
    pub async fn run_schedule_now(
        &self,
        conversation: &ConversationId,
        id: &ScheduleId,
        task: Option<String>,
    ) -> Result<StoredSchedule, ScheduleError> {
        let schedule = self.stored_schedule(conversation, id).await?;
        let task = match (runs::tasks_of(&schedule.trigger), task) {
            (None, None) => None,
            (None, Some(_)) => return Err(ScheduleError::NoTasks),
            (Some([only]), None) => Some(only),
            (Some(_), None) => return Err(ScheduleError::TaskNeeded),
            (Some(tasks), Some(id)) => Some(tasks.iter().find(|each| each.id == id).ok_or(ScheduleError::NoTask(id))?),
        };
        if task.is_some_and(|task| demi_shared_types::is_blank(&task.prompt)) {
            return Err(ScheduleError::NoMessage);
        }
        let at = self.services().control.now();
        self.fire(
            &schedule,
            runs::Fired {
                cause: demi_backend_database::schedules::RunCause::Now,
                at,
                task: task.map(|task| task.id.clone()),
                payload: runs::Payload::None,
            },
        )
        .await;
        Ok(schedule)
    }

    /// The conversation was archived: its schedules wake nothing while it is,
    /// which their reads say by themselves, and what they had under way drops,
    /// as a pause drops it.
    pub(crate) async fn conversation_archived_schedules(&self, conversation: &ConversationId) {
        self.drop_pending_of(conversation).await;
        self.schedules_changed(conversation);
    }

    /// The conversation was unarchived: its schedules wake it again, counting
    /// from now, which the restore wrote with it.
    pub(crate) fn conversation_unarchived_schedules(&self, conversation: &ConversationId) {
        self.schedules_changed(conversation);
    }

    /// Drops what each of the conversation's schedules has under way.
    async fn drop_pending_of(&self, conversation: &ConversationId) {
        match self.services().control.conversation_schedules(conversation.clone()).await {
            Ok(schedules) => {
                for schedule in &schedules {
                    self.drop_pending(&schedule.id);
                    if let Err(error) = self.services().control.drop_pending_calls(schedule.id.clone()).await {
                        tracing::warn!(event = "schedules.calls_not_dropped", schedule = %schedule.id, %error, "a held schedule's pending calls stay");
                    }
                }
            }
            Err(error) => tracing::warn!(event = "schedules.read_failed", %conversation, %error, "the schedules were not read"),
        }
    }

    /// Fires the schedules of the user as their times come, until the shard
    /// closes.
    ///
    /// It settles the runs a run of the backend before left running, then fires
    /// what is due, starts the change watches whose time came, and sleeps until
    /// the nearest next time or a change of the schedules. A time that passed
    /// while the backend was down fires once.
    pub(crate) async fn follow_schedules(self: Rc<Self>) {
        self.settle_left_runs().await;
        self.resume_pending_calls().await;
        loop {
            let next = match self.fire_due().await {
                Ok(next) => next,
                Err(error) => {
                    // The next change or the next start reads them again.
                    tracing::warn!(event = "schedules.read_failed", user = %self.user(), %error, "the schedules were not read");
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
                () = self.schedules().wake.notified() => {}
                () = self.closed() => {
                    // The shard ends: its bursts' waits and its checks stop.
                    let pending = std::mem::take(&mut *self.schedules().pending.borrow_mut());
                    drop(pending);
                    return;
                }
            }
        }
    }

    /// Fires every schedule whose time came, and answers the nearest next
    /// time.
    async fn fire_due(&self) -> Result<Option<Timestamp>, StorageError> {
        let control = &self.services().control;
        let now = control.now();
        let mut nearest: Option<Timestamp> = None;
        let mut keep = |at: Timestamp| nearest = Some(nearest.map_or(at, |nearest| nearest.min(at)));
        for schedule in control.schedules(self.user().clone()).await? {
            if !schedule.on() {
                continue;
            }
            match &schedule.trigger {
                Trigger::Time { timing } => {
                    let Ok(plan) = Plan::new(timing, &schedule.time_zone, schedule.since.to_jiff())
                    else {
                        continue;
                    };
                    let last = control.schedule_last_fired(schedule.id.clone()).await?;
                    let mut after = last.unwrap_or(schedule.since).to_jiff();
                    // Each task due runs as its own run, once for its
                    // latest missed time; in time order, so the last fired
                    // is the latest.
                    for (due, task) in plan.due(after, now.to_jiff()) {
                        self.fire(&schedule, runs::Fired::time(Timestamp::truncate(due), task.map(str::to_owned)))
                            .await;
                        after = after.max(due);
                    }
                    if let Some(next) = plan.next_after(after) {
                        keep(Timestamp::truncate(next));
                    }
                }
                Trigger::Change { every_minutes, .. } => {
                    if self.schedule_checking(&schedule.id) {
                        continue;
                    }
                    let due = Timestamp::truncate(self.watch_due(&schedule, *every_minutes));
                    let held = self.limit_reached(&schedule, now).await?;
                    match held {
                        // The limit holds back the command only once it is
                        // due; until then nothing was skipped.
                        Some(tomorrow) if due <= now => {
                            self.count_held_back(&schedule, 0).await?;
                            keep(tomorrow);
                        }
                        Some(_) => keep(due),
                        None if due <= now => self.start_watch(schedule.clone()),
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
    fn watch_due(&self, schedule: &StoredSchedule, every_minutes: u32) -> jiff::Timestamp {
        let recorded = schedule.watch.as_ref().map(|watch| watch.checked_at);
        let tried = self.schedules().tried.borrow().get(&schedule.id).copied();
        match recorded.max(tried) {
            Some(last) => Plan::every(every_minutes, last.to_jiff())
                .next_after(last.to_jiff())
                .unwrap_or(last.to_jiff()),
            None => schedule.since.to_jiff(),
        }
    }

    /// When the schedule's daily limit is reached, the start of the next
    /// day, from which it runs again; none while runs are left today or it
    /// has no limit.
    async fn limit_reached(
        &self,
        schedule: &StoredSchedule,
        now: Timestamp,
    ) -> Result<Option<Timestamp>, StorageError> {
        let Some(limit) = schedule.max_runs_per_day else {
            return Ok(None);
        };
        let Ok(zone) = plan::zone(&schedule.time_zone) else {
            return Ok(None);
        };
        let (today, tomorrow) = plan::day_bounds(now.to_jiff(), &zone);
        let ran = self
            .services()
            .control
            .schedule_runs_since(schedule.id.clone(), Timestamp::truncate(today))
            .await?;
        Ok((ran >= limit).then(|| Timestamp::truncate(tomorrow)))
    }

    /// Counts a trigger the daily limit held back in the day's one skipped
    /// run, *Skipped, the limit of 10 runs a day was reached, 7 calls
    /// since*, made by the first and grown by each later one.
    async fn count_held_back(&self, schedule: &StoredSchedule, calls: u32) -> Result<(), StorageError> {
        let control = &self.services().control;
        let now = control.now();
        let Some(limit) = schedule.max_runs_per_day else {
            return Ok(());
        };
        let Ok(zone) = plan::zone(&schedule.time_zone) else {
            return Ok(());
        };
        let (today, _) = plan::day_bounds(now.to_jiff(), &zone);
        match control
            .schedule_limit_run(schedule.id.clone(), Timestamp::truncate(today))
            .await?
        {
            Some(mut run) => {
                if let Some(demi_web_api_protocol::schedules::RunNote::Limit { calls: count, .. }) = &mut run.note {
                    *count += calls;
                }
                control.update_schedule_run(run).await?;
            }
            None => {
                control
                    .insert_schedule_run(runs::skipped(
                        schedule,
                        runs::cause_of(&schedule.trigger),
                        now,
                        demi_web_api_protocol::schedules::RunNote::Limit { limit, calls },
                    ))
                    .await?;
            }
        }
        self.mark(Part::Metadata(schedule.conversation.clone()));
        Ok(())
    }
}

/// Opens the shard of every user who has a schedule, whose scheduler
/// then fires what a backend that was down missed and waits for the next
/// times.
pub async fn start_schedulers(
    control: &demi_backend_database::control::ControlService,
    shards: &crate::shard::Shards,
) -> Result<(), StorageError> {
    for user in control.users_with_schedules().await? {
        // The shard's first call starts its scheduler; a shard that cannot
        // be reached starts it on its next call.
        if let Err(error) = shards.of(&user).call(|_, _| async {}).await {
            tracing::warn!(event = "schedule_scheduler.start_failed", %user, %error, "the schedules' scheduler did not start");
        }
    }
    Ok(())
}

fn run_dto(run: StoredRun) -> ScheduleRunDto {
    ScheduleRunDto {
        id: run.id,
        at: run.at,
        task_id: run.task,
        state: run.state,
        note: run.note,
        line: run.line,
    }
}
