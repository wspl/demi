//! Schedules: what a conversation's Schedules menu lists, and the rules each
//! part of a schedule follows wherever it arrives, an agent's `demi schedule`
//! alike. The backend's scheduler reads the timing; this crate only carries
//! it.

use demi_shared_types::{Nullable, ScheduleId, ScheduleRunId, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::{double_option, unwrap_or_skip};

use crate::text::Trimmed;

/// The runs `demi schedule show` lists, newest first; older ones are kept
/// until the schedule is deleted, and are not shown.
///
/// Bounded because the list goes into the agent's context window and a
/// schedule's runs have no end.
pub const RUNS_SHOWN: usize = 50;
/// The name a new schedule starts with.
pub const NEW_NAME: &str = "New Schedule";

/// A day of the week, as a weekly task names the days it runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    pub const ALL: [Weekday; 7] = [
        Self::Mon,
        Self::Tue,
        Self::Wed,
        Self::Thu,
        Self::Fri,
        Self::Sat,
        Self::Sun,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mon => "mon",
            Self::Tue => "tue",
            Self::Wed => "wed",
            Self::Thu => "thu",
            Self::Fri => "fri",
            Self::Sat => "sat",
            Self::Sun => "sun",
        }
    }

    /// The day `text` names, `mon` to `sun`.
    pub fn named(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|day| day.as_str() == text)
    }
}

/// The unit a repeat's cycle counts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RepeatUnit {
    Days,
    Weeks,
    Months,
    Years,
}

impl RepeatUnit {
    pub const ALL: [RepeatUnit; 4] = [Self::Days, Self::Weeks, Self::Months, Self::Years];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Days => "days",
            Self::Weeks => "weeks",
            Self::Months => "months",
            Self::Years => "years",
        }
    }
}

/// A repeating cycle, as Calendar's repeat: every `every` days, weeks,
/// months or years. A cycle of several units counts from the day, week,
/// month or year of `start`, its first cycle, and runs its tasks in each
/// cycle's first unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Repeat {
    #[garde(range(min = 1))]
    pub every: u32,
    pub unit: RepeatUnit,
    /// The first cycle's day, as `2026-01-01`, for a cycle of several
    /// units; null for one unit.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(custom(start_of(self.every)))]
    pub start: Option<String>,
}

/// A cycle of several units has a first cycle, and one of one unit none.
fn start_of(every: u32) -> impl FnOnce(&Option<String>, &()) -> garde::Result {
    move |start, _| match (every > 1, start) {
        (true, Some(day)) if day.parse::<jiff::civil::Date>().is_ok() && day.len() == 10 => Ok(()),
        (true, Some(_)) => Err(garde::Error::new("is not a day such as 2026-01-01")),
        (true, None) => Err(garde::Error::new("a cycle of several units needs its first cycle's day")),
        (false, Some(_)) => Err(garde::Error::new("a cycle of one unit has no first cycle's day")),
        (false, None) => Ok(()),
    }
}

/// Which of a month's weekdays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WeekOfMonth {
    First,
    Second,
    Third,
    Fourth,
    Last,
}

/// The first or the last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FirstOrLast {
    First,
    Last,
}

/// A day of a month: a date, which a month without it skips; the last day;
/// a weekday's first to fourth or last; or the first or last working day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MonthDay {
    Date {
        #[garde(range(min = 1, max = 31))]
        date: u8,
    },
    LastDay {},
    Weekday {
        #[garde(skip)]
        week: WeekOfMonth,
        #[garde(skip)]
        weekday: Weekday,
    },
    WorkingDay {
        #[garde(skip)]
        which: FirstOrLast,
    },
}

/// A country by its ISO 3166 code, and one of its regions whose holidays
/// differ, by the code the holidays' table gives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HolidayPlace {
    #[garde(length(min = 2, max = 3))]
    pub country: String,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(inner(length(min = 1)))]
    pub region: Option<String>,
}

/// A task of a repeat: a moment in its cycle and the message it sends.
///
/// Its moment is `time` and, as the repeat's unit asks, `days` (weeks),
/// `day` (months), or `month` and `day` (years); the parts another unit
/// reads are kept, so a repeat changed back finds them as they were.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Task {
    /// Unique among the repeat's tasks; the page or the command makes it.
    #[garde(length(min = 1))]
    pub id: String,
    /// A clock time as `09:00`.
    #[garde(pattern(r"^([01][0-9]|2[0-3]):[0-5][0-9]$"))]
    pub time: String,
    /// One to seven distinct days.
    #[garde(length(min = 1, max = 7), custom(distinct_days))]
    pub days: Vec<Weekday>,
    #[garde(dive)]
    pub day: MonthDay,
    /// 1 to 12.
    #[garde(range(min = 1, max = 12))]
    pub month: u8,
    /// The country, and its region, whose working week and public
    /// holidays a first or last working day follows; null for Monday to
    /// Friday.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<HolidayPlace>")]
    #[garde(dive)]
    pub holidays: Option<HolidayPlace>,
    /// Empty until written; a task without a message does not run.
    #[garde(skip)]
    pub prompt: String,
}

fn distinct_tasks(tasks: &[Task], _: &()) -> garde::Result {
    let mut seen = std::collections::BTreeSet::new();
    if tasks.iter().all(|task| seen.insert(task.id.as_str())) {
        Ok(())
    } else {
        Err(garde::Error::new("a task's id is used twice"))
    }
}

/// When a time trigger fires, in the schedule's time zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Timing {
    /// Once, at this moment.
    Once {
        #[garde(skip)]
        at: Timestamp,
    },
    /// On a repeating cycle, each of its tasks at its moment in it.
    Repeat {
        #[garde(dive)]
        repeat: Repeat,
        #[garde(length(min = 1), custom(distinct_tasks), dive)]
        tasks: Vec<Task>,
    },
    /// Every `minutes`, from when the schedule was set.
    Every {
        #[garde(range(min = 1))]
        minutes: u32,
    },
    /// A cron expression of five fields: minute, hour, day of month, month
    /// and day of the week, which the backend reads.
    Cron {
        #[garde(length(chars, min = 9))]
        expression: String,
    },
}

fn distinct_days(days: &[Weekday], _: &()) -> garde::Result {
    let mut seen = std::collections::BTreeSet::new();
    if days.iter().all(|day| seen.insert(*day)) {
        Ok(())
    } else {
        Err(garde::Error::new("a day is named twice"))
    }
}

/// What starts a schedule's runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Trigger {
    /// At a time.
    Time {
        #[garde(dive)]
        timing: Timing,
    },
    /// When the output of `command`, run every `everyMinutes` on the
    /// conversation's Host in its directory, changes.
    Change {
        #[garde(length(min = 1), custom(not_blank))]
        command: String,
        #[garde(range(min = 1))]
        every_minutes: u32,
    },
    /// When its URL, `<server>/hooks/<id>`, is called with its secret.
    Url {},
}

impl Trigger {
    /// Whether it takes a limit on runs a day: a change watch and a URL do.
    pub fn takes_limit(&self) -> bool {
        !matches!(self, Self::Time { .. })
    }
}

fn not_blank(text: &str, _: &()) -> garde::Result {
    if demi_shared_types::is_blank(text) {
        Err(garde::Error::new("is blank"))
    } else {
        Ok(())
    }
}

/// What a run's message does while its conversation is busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IfBusy {
    /// It waits until the running turn ends.
    Queue,
    /// It reaches the running turn, as the composer's Steer does.
    Steer,
}

/// Where a run stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    Done,
    /// Its agent ran `demi notify`.
    Notified,
    Skipped,
    Failed,
}

/// Why a run was skipped or failed, which the page words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RunNote {
    /// Skipped: the conversation's device was offline.
    Offline { device: String },
    /// Skipped: the day's limit of runs was reached; `calls` URL calls, or
    /// changes, came since, and started nothing.
    Limit { limit: u32, calls: u32 },
    /// Failed: the watch's command ran over 60 seconds and was stopped.
    CommandTimedOut,
    /// Failed: the watch's command could not run, for this reason.
    CommandFailed { message: String },
    /// Failed: the run could not start its turn, or its turn failed, with
    /// this message and, from the turn's error block, its code, which the
    /// page words as the transcript words it.
    TurnFailed {
        message: String,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<String>")]
        code: Option<String>,
    },
}

/// One run of a schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRunDto {
    pub id: ScheduleRunId,
    /// When its trigger fired.
    pub at: Timestamp,
    /// The repeat's task that started it; null for a single trigger.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub task_id: Option<String>,
    pub state: RunState,
    /// Why it was skipped or failed.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<RunNote>")]
    pub note: Option<RunNote>,
    /// The start of its agent's last answer.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub line: Option<String>,
}

/// A schedule as `demi schedule` shows it, with its latest runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleDto {
    pub id: ScheduleId,
    pub name: String,
    pub trigger: Trigger,
    /// The IANA time zone its timing and its daily limit count in.
    pub time_zone: String,
    /// A change watch's or a URL's limit on runs a day; null for none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    pub max_runs_per_day: Option<u32>,
    pub if_busy: IfBusy,
    /// What a single trigger's runs send; a repeat's tasks hold their own.
    pub prompt: String,
    pub paused: bool,
    /// When it next fires by itself; null while paused or with no time
    /// left, and for a URL.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    pub next_at: Option<Timestamp>,
    pub created_at: Timestamp,
    /// Its latest runs, newest first, [`RUNS_SHOWN`] at most.
    pub runs: Vec<ScheduleRunDto>,
}

/// What `demi schedule create` and `edit` change; a new schedule starts from
/// its defaults, *New Schedule*, paused, every day with one task at 09:00 and
/// no message, which waits if the conversation is busy, and takes these over
/// them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchedulePatch {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Trimmed")]
    #[garde(inner(length(chars, min = 1)))]
    pub name: Option<Trimmed>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Trigger")]
    #[garde(dive)]
    pub trigger: Option<Trigger>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(inner(length(min = 1)))]
    pub time_zone: Option<String>,
    /// Null removes the limit.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    #[schemars(with = "Nullable<u32>")]
    #[garde(inner(inner(range(min = 1))))]
    pub max_runs_per_day: Option<Option<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "IfBusy")]
    #[garde(skip)]
    pub if_busy: Option<IfBusy>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub paused: Option<bool>,
}

/// A schedule of a conversation as its Schedules menu lists it: what wakes
/// the conversation, when it next will, and how it last did. The user pauses,
/// resumes and deletes it there; the agent does the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSchedule {
    pub id: ScheduleId,
    pub name: String,
    pub trigger: Trigger,
    /// The IANA time zone its times are read in.
    pub time_zone: String,
    /// Whether the user or the agent paused it. A schedule of an archived
    /// conversation wakes nothing either, which the conversation's own
    /// archived state says.
    pub paused: bool,
    /// When it next wakes the conversation; null while paused, while its
    /// conversation is archived, with no time left, and for a URL.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    pub next_at: Option<Timestamp>,
    /// Its last run; null before it woke the conversation.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<LastRun>")]
    pub last_run: Option<LastRun>,
}

/// `PATCH /conversations/:id/schedules/:schedule`: the user pauses or resumes
/// one of the conversation's schedules from its Schedules menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleChange {
    #[garde(skip)]
    pub paused: bool,
}

/// How a schedule last woke its conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LastRun {
    pub at: Timestamp,
    pub state: RunState,
    /// Why it was skipped or failed.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<RunNote>")]
    pub note: Option<RunNote>,
}
