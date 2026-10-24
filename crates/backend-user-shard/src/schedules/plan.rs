//! When a schedule fires by itself.
//!
//! This module finds the next time of a schedule after a moment, in its time
//! zone, and the latest one a backend that was down missed, which fires once:
//! for a repeat, once for each task, at its own latest missed time.
//!
//! A repeat's next time is the earliest of its tasks' with a message. A task
//! runs in each cycle's first unit: every N days from its first cycle's day,
//! in the first week (from Monday) of every N, the first month of every N, or
//! the first year of every N, counted from the day, week, month or year of
//! its first cycle, or from any with a cycle of one unit. Nothing runs before
//! the first cycle's day, even in its unit. A date a month
//! lacks is skipped that month; a working day follows the task's country and
//! region (`holidays.rs`), or Monday to Friday.
//!
//! Daylight saving time follows cron:
//!
//! - A clock time that a change to summer time skips fires at the first minute
//!   after the gap.
//! - One that the change back repeats fires the first time.
//!
//! No crate the workspace uses reads cron expressions over `jiff`'s time zones,
//! so the five fields are read here.

use demi_web_api_protocol::schedules::{FirstOrLast, MonthDay, Repeat, RepeatUnit, Timing, Task, WeekOfMonth, Weekday};
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{Timestamp, ToSpan};

/// How many days ahead a schedule is searched: a cron expression whose
/// only date is February 29 waits up to eight years for a Monday one.
///
/// Bounded so a schedule that never occurs ends the search instead of looking
/// without end.
const SEARCH_DAYS: i32 = 366 * 8 + 7;

/// A schedule ready to be read: its time zone found, its cron expression
/// read.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    kind: Kind,
    zone: TimeZone,
    /// When the schedule was set or turned on: no time before it fires, and
    /// an interval counts from it.
    since: Timestamp,
}

#[derive(Debug, Clone)]
enum Kind {
    Once(Timestamp),
    /// A repeat's tasks with a message; one without runs never.
    Repeat(Vec<TaskPlan>),
    Every { minutes: i64 },
    Cron(Cron),
}

/// How many of a task's cycles are searched: a date that only February 29
/// has waits up to eight years, and a cycle of every 12 months from February
/// reaches a 31st never.
///
/// Bounded for the same reason as [`SEARCH_DAYS`].
const SEARCH_CYCLES: i64 = 1000;

/// A task ready to be read: its cycle, its moment in it and the calendar of
/// its working days.
#[derive(Debug, Clone)]
struct TaskPlan {
    id: String,
    every: i64,
    unit: RepeatUnit,
    /// Its first cycle's day; none for a cycle of one unit, which any unit
    /// starts.
    start: Option<Date>,
    time: Time,
    days: u8,
    day: MonthDay,
    month: i8,
    calendar: Option<super::holidays::Calendar>,
}

/// Why a schedule cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PlanError {
    #[error("unknown time zone {0:?}")]
    TimeZone(String),
    #[error("cron expression {expression:?}: {reason}")]
    Cron { expression: String, reason: String },
    #[error("no public holidays are known for {0}")]
    Holidays(String),
}

/// The time zone `name`, an IANA name such as `Asia/Shanghai`.
pub(crate) fn zone(name: &str) -> Result<TimeZone, PlanError> {
    TimeZone::get(name).map_err(|_| PlanError::TimeZone(name.to_owned()))
}

fn day_bit(day: jiff::civil::Weekday) -> u8 {
    1 << day.to_monday_zero_offset()
}

fn weekday_bit(day: Weekday) -> u8 {
    1 << (Weekday::ALL.iter().position(|each| *each == day).expect("every day is listed") as u8)
}

impl Plan {
    pub(crate) fn new(timing: &Timing, zone_name: &str, since: Timestamp) -> Result<Self, PlanError> {
        let zone = zone(zone_name)?;
        let kind = match timing {
            Timing::Once { at } => Kind::Once(at.to_jiff()),
            Timing::Repeat { repeat, tasks } => Kind::Repeat(
                tasks
                    .iter()
                    .filter(|task| !demi_shared_types::is_blank(&task.prompt))
                    .map(|task| TaskPlan::new(repeat, task))
                    .collect::<Result<_, _>>()?,
            ),
            Timing::Every { minutes } => Kind::Every {
                minutes: i64::from(*minutes),
            },
            Timing::Cron { expression } => Kind::Cron(Cron::parse(expression)?),
        };
        Ok(Self { kind, zone, since })
    }

    /// An interval of `minutes` from `since`, as a change watch checks.
    pub(crate) fn every(minutes: u32, since: Timestamp) -> Self {
        Self {
            kind: Kind::Every {
                minutes: i64::from(minutes),
            },
            zone: TimeZone::UTC,
            since,
        }
    }

    /// The first time after `after`, and not before the schedule was set;
    /// none when no time is left, as for a moment that passed.
    pub(crate) fn next_after(&self, after: Timestamp) -> Option<Timestamp> {
        match &self.kind {
            Kind::Once(at) => (*at > after && *at >= self.since).then_some(*at),
            Kind::Every { minutes } => {
                let step = minutes * 60_000;
                let from = self.since.as_millisecond();
                let past = after.as_millisecond() - from;
                let count = if past < 0 { 1 } else { past / step + 1 };
                Timestamp::from_millisecond(from + count * step).ok()
            }
            Kind::Repeat(tasks) => {
                let after = after.max(self.since);
                tasks.iter().filter_map(|task| self.task_after(task, after)).min()
            }
            Kind::Cron(cron) => {
                let after = after.max(self.since);
                self.search(after, |date| cron.times_on(date))
            }
        }
    }

    /// What is due after `after` and at or before `now`, in time order: a
    /// repeat's tasks, each once at its latest time in that span, which a
    /// backend that was down missed; another schedule's latest time, once,
    /// with no task.
    pub(crate) fn due(&self, after: Timestamp, now: Timestamp) -> Vec<(Timestamp, Option<&str>)> {
        let Kind::Repeat(tasks) = &self.kind else {
            return self.latest_due(after, now).map(|at| (at, None)).into_iter().collect();
        };
        let mut due: Vec<(Timestamp, Option<&str>)> = tasks
            .iter()
            .filter_map(|task| {
                let mut latest = None;
                let mut next = self.task_after(task, after.max(self.since));
                while let Some(at) = next.filter(|at| *at <= now) {
                    latest = Some(at);
                    next = self.task_after(task, at);
                }
                latest.map(|at| (at, Some(task.id.as_str())))
            })
            .collect();
        due.sort_by_key(|(at, _)| *at);
        due
    }

    /// The first moment of `task` after `after`, searching its cycles from
    /// the one that holds `after`'s local date.
    fn task_after(&self, task: &TaskPlan, after: Timestamp) -> Option<Timestamp> {
        let from = after.to_zoned(self.zone.clone()).date();
        let anchor = task.start.unwrap_or(from);
        let current = task.unit_index(anchor, from);
        // The cycle holding `from`, or the first one when it is later.
        let first = if current < 0 { 0 } else { current - current % task.every };
        (0..SEARCH_CYCLES)
            .map(|cycle| first + cycle * task.every)
            .flat_map(|index| task.dates_in(anchor, index))
            .filter(|date| task.start.is_none_or(|start| *date >= start))
            .filter_map(|date| self.instant(date.to_datetime(task.time)))
            .find(|at| *at > after)
    }

    /// The latest time at or before `now` after `after`, which a backend
    /// that was down missed: one run stands for all of them.
    fn latest_due(&self, after: Timestamp, now: Timestamp) -> Option<Timestamp> {
        let mut due = None;
        let mut next = self.next_after(after);
        if let Kind::Every { minutes } = &self.kind {
            // The interval's latest time is counted, not walked.
            let first = next?;
            if first > now {
                return None;
            }
            let step = minutes * 60_000;
            let behind = (now.as_millisecond() - first.as_millisecond()) / step;
            return Timestamp::from_millisecond(first.as_millisecond() + behind * step).ok();
        }
        while let Some(at) = next.filter(|at| *at <= now) {
            due = Some(at);
            next = self.next_after(at);
        }
        due
    }

    /// The first time after `after` among the clock times `times` gives for
    /// each local date from `after`'s on.
    fn search(&self, after: Timestamp, times: impl Fn(Date) -> Vec<Time>) -> Option<Timestamp> {
        let start = after.to_zoned(self.zone.clone()).date();
        for offset in 0..SEARCH_DAYS {
            let date = start.checked_add(offset.days()).ok()?;
            for time in times(date) {
                let at = self.instant(date.to_datetime(time))?;
                if at > after {
                    return Some(at);
                }
            }
        }
        None
    }

    /// A local date and time as an instant: a time the clocks skip is the first
    /// minute after the gap, and a repeated one its first occurrence, as cron
    /// does.
    fn instant(&self, local: DateTime) -> Option<Timestamp> {
        let ambiguous = self.zone.to_ambiguous_timestamp(local);
        match ambiguous.offset() {
            AmbiguousOffset::Gap { after, .. } => {
                // Read with the offset after the gap, the time falls before
                // it; the gap ends at the next transition.
                let early = after.to_timestamp(local).ok()?;
                self.zone.following(early).next().map(|transition| transition.timestamp())
            }
            AmbiguousOffset::Unambiguous { .. } | AmbiguousOffset::Fold { .. } => ambiguous.earlier().ok(),
        }
    }
}

/// The start of the local day that holds `now`, from which a daily limit
/// counts, and the next one, when it starts again.
pub(crate) fn day_bounds(now: Timestamp, zone: &TimeZone) -> (Timestamp, Timestamp) {
    let date = now.to_zoned(zone.clone()).date();
    let start = |date: Date| {
        zone.to_ambiguous_zoned(date.to_datetime(Time::midnight()))
            .compatible()
            .map(|zoned| zoned.timestamp())
            .unwrap_or(now)
    };
    let tomorrow = date.tomorrow().unwrap_or(date);
    (start(date), start(tomorrow))
}

impl TaskPlan {
    fn new(repeat: &Repeat, task: &Task) -> Result<Self, PlanError> {
        let working = matches!(task.day, MonthDay::WorkingDay { .. })
            && matches!(repeat.unit, RepeatUnit::Months | RepeatUnit::Years);
        let calendar = match (&task.holidays, working) {
            (Some(place), true) => Some(super::holidays::calendar(place).ok_or_else(|| {
                PlanError::Holidays(match &place.region {
                    Some(region) => format!("{} {region}", place.country),
                    None => place.country.clone(),
                })
            })?),
            _ => None,
        };
        Ok(Self {
            id: task.id.clone(),
            every: i64::from(repeat.every),
            unit: repeat.unit,
            start: repeat
                .start
                .as_deref()
                .map(|day| day.parse().expect("the contract checked the first cycle's day")),
            time: clock(&task.time).expect("the contract checked the clock time"),
            days: task.days.iter().map(|day| weekday_bit(*day)).fold(0, |all, bit| all | bit),
            day: task.day,
            month: i8::try_from(task.month).expect("the contract checked the month"),
            calendar,
        })
    }

    /// The index of the unit holding `date`, counted from the unit of
    /// `anchor`, negative before it.
    fn unit_index(&self, anchor: Date, date: Date) -> i64 {
        let months = |day: Date| i64::from(day.year()) * 12 + i64::from(day.month());
        match self.unit {
            RepeatUnit::Days => days_between(anchor, date),
            RepeatUnit::Weeks => days_between(monday(anchor), monday(date)) / 7,
            RepeatUnit::Months => months(date) - months(anchor),
            RepeatUnit::Years => i64::from(date.year()) - i64::from(anchor.year()),
        }
    }

    /// The task's dates in the unit `index` units after `anchor`'s, in order.
    fn dates_in(&self, anchor: Date, index: i64) -> Vec<Date> {
        let span = |count: i64| i32::try_from(count).ok();
        match self.unit {
            RepeatUnit::Days => span(index)
                .and_then(|days| anchor.checked_add(days.days()).ok())
                .into_iter()
                .collect(),
            RepeatUnit::Weeks => {
                let Some(week) = span(index * 7).and_then(|days| monday(anchor).checked_add(days.days()).ok()) else {
                    return Vec::new();
                };
                (0..7)
                    .filter_map(|offset| week.checked_add(offset.days()).ok())
                    .filter(|date| self.days & day_bit(date.weekday()) != 0)
                    .collect()
            }
            RepeatUnit::Months => {
                let Some(first) = span(index)
                    .and_then(|months| anchor.first_of_month().checked_add(months.months()).ok())
                else {
                    return Vec::new();
                };
                self.day_of(first).into_iter().collect()
            }
            RepeatUnit::Years => {
                let Some(first) = span(index)
                    .and_then(|years| i16::try_from(i32::from(anchor.year()) + years).ok())
                    .and_then(|year| Date::new(year, self.month, 1).ok())
                else {
                    return Vec::new();
                };
                self.day_of(first).into_iter().collect()
            }
        }
    }

    /// The task's day in the month that starts on `first`; none in a month
    /// that lacks its date.
    fn day_of(&self, first: Date) -> Option<Date> {
        let last = first.last_of_month();
        match self.day {
            MonthDay::Date { date } => first.with().day(i8::try_from(date).ok()?).build().ok(),
            MonthDay::LastDay {} => Some(last),
            MonthDay::Weekday { week, weekday } => {
                let wanted = jiff_weekday(weekday);
                let nth = match week {
                    WeekOfMonth::First => 1,
                    WeekOfMonth::Second => 2,
                    WeekOfMonth::Third => 3,
                    WeekOfMonth::Fourth => 4,
                    WeekOfMonth::Last => -1,
                };
                first.nth_weekday_of_month(nth, wanted).ok()
            }
            MonthDay::WorkingDay { which } => {
                let weekdays = super::holidays::Calendar::weekdays();
                let calendar = self.calendar.as_ref().unwrap_or(&weekdays);
                let mut days = first.series(1.day()).take_while(|day| *day <= last);
                match which {
                    FirstOrLast::First => days.find(|day| calendar.works_on(*day)),
                    FirstOrLast::Last => days.filter(|day| calendar.works_on(*day)).last(),
                }
            }
        }
    }
}

fn days_between(from: Date, to: Date) -> i64 {
    i64::from(to.since(from).map_or(0, |span| span.get_days()))
}

fn monday(date: Date) -> Date {
    date.checked_sub(i32::from(date.weekday().to_monday_zero_offset()).days()).unwrap_or(date)
}

fn jiff_weekday(day: Weekday) -> jiff::civil::Weekday {
    match day {
        Weekday::Mon => jiff::civil::Weekday::Monday,
        Weekday::Tue => jiff::civil::Weekday::Tuesday,
        Weekday::Wed => jiff::civil::Weekday::Wednesday,
        Weekday::Thu => jiff::civil::Weekday::Thursday,
        Weekday::Fri => jiff::civil::Weekday::Friday,
        Weekday::Sat => jiff::civil::Weekday::Saturday,
        Weekday::Sun => jiff::civil::Weekday::Sunday,
    }
}

/// `09:00` as a clock time.
fn clock(text: &str) -> Option<Time> {
    let (hour, minute) = text.split_once(':')?;
    Time::new(hour.parse().ok()?, minute.parse().ok()?, 0, 0).ok()
}

/// A cron expression's five fields, each the set of values it matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cron {
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    weekdays: u64,
    /// Whether the day of the month and of the week were both restricted,
    /// in which case either matches, as cron reads them.
    either_day: bool,
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

impl Cron {
    pub(crate) fn parse(expression: &str) -> Result<Self, PlanError> {
        let fail = |reason: String| PlanError::Cron {
            expression: expression.to_owned(),
            reason,
        };
        let fields: Vec<&str> = expression.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields.as_slice() else {
            return Err(fail(format!(
                "it has {} fields; cron reads five: minute, hour, day of month, month, day of week",
                fields.len()
            )));
        };
        let minutes = field(minute, 0, 59, &[]).map_err(|reason| fail(format!("minute: {reason}")))?;
        let hours = field(hour, 0, 23, &[]).map_err(|reason| fail(format!("hour: {reason}")))?;
        let days = field(day, 1, 31, &[]).map_err(|reason| fail(format!("day of month: {reason}")))?;
        let months = field(month, 1, 12, &MONTHS).map_err(|reason| fail(format!("month: {reason}")))?;
        let mut weekdays =
            field(weekday, 0, 7, &DAYS).map_err(|reason| fail(format!("day of week: {reason}")))?;
        // 7 is Sunday too.
        if weekdays & (1 << 7) != 0 {
            weekdays |= 1;
        }
        let restricted = |text: &str| !text.starts_with('*');
        Ok(Self {
            minutes,
            hours,
            days,
            months,
            weekdays,
            either_day: restricted(day) && restricted(weekday),
        })
    }

    /// The clock times the expression matches on `date`, in order.
    fn times_on(&self, date: Date) -> Vec<Time> {
        let month = self.months & (1 << date.month()) != 0;
        let day_of_month = self.days & (1 << date.day()) != 0;
        let day_of_week = self.weekdays & (1 << date.weekday().to_sunday_zero_offset()) != 0;
        let day = if self.either_day {
            day_of_month || day_of_week
        } else {
            day_of_month && day_of_week
        };
        if !month || !day {
            return Vec::new();
        }
        let mut times = Vec::new();
        for hour in 0..24 {
            if self.hours & (1 << hour) == 0 {
                continue;
            }
            for minute in 0..60 {
                if self.minutes & (1 << minute) != 0 {
                    times.push(Time::new(hour, minute, 0, 0).expect("an hour and a minute in range"));
                }
            }
        }
        times
    }
}

/// One field's set of values between `low` and `high`: `*`, `a`, `a-b`,
/// each with `/step`, joined by commas; `names` stand for `low` on.
fn field(text: &str, low: u32, high: u32, names: &[&str]) -> Result<u64, String> {
    let value = |part: &str| -> Result<u32, String> {
        let lower = part.to_ascii_lowercase();
        if let Some(index) = names.iter().position(|name| *name == lower) {
            // Month names start at 1, day names at 0.
            return Ok(index as u32 + if low == 1 { 1 } else { 0 });
        }
        let number: u32 = part.parse().map_err(|_| format!("{part:?} is not a number"))?;
        if number < low || number > high {
            return Err(format!("{number} is outside {low}-{high}"));
        }
        Ok(number)
    };
    let mut set = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => {
                let step: u32 = step.parse().map_err(|_| format!("step {step:?} is not a number"))?;
                if step == 0 {
                    return Err("a step of 0".into());
                }
                (range, step)
            }
            None => (part, 1),
        };
        let (from, to) = if range == "*" {
            (low, high)
        } else if let Some((from, to)) = range.split_once('-') {
            (value(from)?, value(to)?)
        } else {
            let single = value(range)?;
            // `a/n` runs from a to the end.
            (single, if part.contains('/') { high } else { single })
        };
        if from > to {
            return Err(format!("{from}-{to} runs backwards"));
        }
        let mut each = from;
        while each <= to {
            set |= 1 << each;
            each += step;
        }
    }
    Ok(set)
}
