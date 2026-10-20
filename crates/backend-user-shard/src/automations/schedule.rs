//! When an automation fires by itself.
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

use demi_web_api_protocol::automations::{FirstOrLast, MonthDay, Repeat, RepeatUnit, Schedule, Task, WeekOfMonth, Weekday};
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{Timestamp, ToSpan};

/// How many days ahead a schedule is searched: a cron expression whose
/// only date is February 29 waits up to eight years for a Monday one.
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
    pub(crate) fn new(schedule: &Schedule, zone_name: &str, since: Timestamp) -> Result<Self, PlanError> {
        let zone = zone(zone_name)?;
        let kind = match schedule {
            Schedule::Once { at } => Kind::Once(at.to_jiff()),
            Schedule::Repeat { repeat, tasks } => Kind::Repeat(
                tasks
                    .iter()
                    .filter(|task| !demi_shared_types::is_blank(&task.prompt))
                    .map(|task| TaskPlan::new(repeat, task))
                    .collect::<Result<_, _>>()?,
            ),
            Schedule::Every { minutes } => Kind::Every {
                minutes: i64::from(*minutes),
            },
            Schedule::Cron { expression } => Kind::Cron(Cron::parse(expression)?),
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

#[cfg(test)]
mod tests {
    use super::*;
    use demi_web_api_protocol::automations::HolidayPlace;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn task(id: &str, time: &str) -> Task {
        Task {
            id: id.into(),
            time: time.into(),
            days: vec![Weekday::Mon],
            day: MonthDay::Date { date: 1 },
            month: 1,
            holidays: None,
            prompt: format!("the message of {id}"),
        }
    }

    fn repeat(every: u32, unit: RepeatUnit, start: Option<&str>, tasks: Vec<Task>) -> Schedule {
        Schedule::Repeat {
            repeat: Repeat {
                every,
                unit,
                start: start.map(Into::into),
            },
            tasks,
        }
    }

    /// Every week on `days` at `time`, one task: what a daily schedule was.
    fn weekly(time: &str, days: &[Weekday]) -> Schedule {
        repeat(1, RepeatUnit::Weeks, None, vec![Task { days: days.to_vec(), ..task("t", time) }])
    }

    /// Each moment `plan` gives after `after`, `count` of them, with the
    /// tasks due at each.
    fn moments(plan: &Plan, after: &str, count: usize) -> Vec<(Timestamp, Vec<String>)> {
        let mut after = at(after);
        let mut found = Vec::new();
        while found.len() < count
            && let Some(next) = plan.next_after(after)
        {
            let before = next.checked_sub(1.millisecond()).unwrap();
            let tasks = plan.due(before, next).into_iter().filter_map(|(_, task)| task.map(str::to_owned)).collect();
            found.push((next, tasks));
            after = next;
        }
        found
    }

    fn expect(list: &[(&str, &[&str])]) -> Vec<(Timestamp, Vec<String>)> {
        list.iter()
            .map(|(when, tasks)| (at(when), tasks.iter().map(|task| (*task).to_owned()).collect()))
            .collect()
    }

    fn cron(expression: &str) -> Schedule {
        Schedule::Cron {
            expression: expression.into(),
        }
    }

    const WEEKDAYS: [Weekday; 5] = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri];

    /// Each schedule's next time after a moment, in its zone: weekdays skip
    /// the weekend, a time the change to summer time skips fires at the
    /// first minute after the gap, a repeated one fires once, and cron reads its
    /// fields, names and either-day rule as cron does.
    #[test]
    fn a_schedule_fires_next_at_its_local_time() {
        let since = at("2026-01-01T00:00:00Z");
        let cases: [(Schedule, &str, &str, Option<&str>); 14] = [
            // Friday 10:00 in Shanghai: the next weekday 09:00 is Monday.
            (weekly("09:00", &WEEKDAYS), "Asia/Shanghai", "2026-10-16T02:00:00Z", Some("2026-10-19T01:00:00Z")),
            (weekly("09:00", &WEEKDAYS), "Asia/Shanghai", "2026-10-15T00:59:00Z", Some("2026-10-15T01:00:00Z")),
            // 02:30 does not exist on 8 March in New York: the first minute
            // after the gap, 03:00 EDT; a cron time in it too, once.
            (weekly("02:30", &Weekday::ALL), "America/New_York", "2026-03-07T12:00:00Z", Some("2026-03-08T07:00:00Z")),
            (cron("15,45 2 * * *"), "America/New_York", "2026-03-08T06:59:00Z", Some("2026-03-08T07:00:00Z")),
            (cron("15,45 2 * * *"), "America/New_York", "2026-03-08T07:00:00Z", Some("2026-03-09T06:15:00Z")),
            // 01:30 happens twice on 1 November: the first, then 2 November.
            (weekly("01:30", &Weekday::ALL), "America/New_York", "2026-10-31T12:00:00Z", Some("2026-11-01T05:30:00Z")),
            (weekly("01:30", &Weekday::ALL), "America/New_York", "2026-11-01T05:30:00Z", Some("2026-11-02T06:30:00Z")),
            (Schedule::Every { minutes: 120 }, "UTC", "2026-01-01T03:00:00Z", Some("2026-01-01T04:00:00Z")),
            (Schedule::Once { at: demi_shared_types::Timestamp::truncate(at("2026-05-01T08:00:00Z")) }, "UTC", "2026-04-01T00:00:00Z", Some("2026-05-01T08:00:00Z")),
            (Schedule::Once { at: demi_shared_types::Timestamp::truncate(at("2026-05-01T08:00:00Z")) }, "UTC", "2026-05-01T08:00:00Z", None),
            (cron("0 9 * * 1-5"), "UTC", "2026-10-17T10:00:00Z", Some("2026-10-19T09:00:00Z")),
            (cron("*/15 * * * *"), "UTC", "2026-10-17T10:07:00Z", Some("2026-10-17T10:15:00Z")),
            // Day of month or day of week: the 1st, or a Sunday.
            (cron("0 0 1 * sun"), "UTC", "2026-10-02T00:00:00Z", Some("2026-10-04T00:00:00Z")),
            (cron("30 6 29 feb *"), "UTC", "2026-03-01T00:00:00Z", Some("2028-02-29T06:30:00Z")),
        ];
        for (schedule, zone, after, expected) in cases {
            let plan = Plan::new(&schedule, zone, since).unwrap();
            assert_eq!(
                plan.next_after(at(after)),
                expected.map(at),
                "{schedule:?} in {zone} after {after}"
            );
        }
    }

    /// A backend down over three of a weekday schedule's times fires once,
    /// for the latest, and then waits for the next; an interval's latest
    /// time is counted.
    #[test]
    fn missed_times_fire_once_for_the_latest() {
        let since = at("2026-01-01T00:00:00Z");
        let plan = Plan::new(&weekly("09:00", &WEEKDAYS), "UTC", since).unwrap();
        // Last fired Monday 09:00; back on Thursday 10:00.
        let last = at("2026-10-12T09:00:00Z");
        let now = at("2026-10-15T10:00:00Z");
        let due = plan.latest_due(last, now);
        assert_eq!(due, Some(at("2026-10-15T09:00:00Z")));
        assert_eq!(plan.next_after(due.unwrap()), Some(at("2026-10-16T09:00:00Z")));
        assert_eq!(plan.latest_due(due.unwrap(), now), None);

        let every = Plan::new(&Schedule::Every { minutes: 10 }, "UTC", since).unwrap();
        assert_eq!(
            every.latest_due(at("2026-01-01T00:10:00Z"), at("2026-01-01T05:07:00Z")),
            Some(at("2026-01-01T05:00:00Z"))
        );
        // A schedule set after its times fires none of them.
        let late = Plan::new(&weekly("09:00", &WEEKDAYS), "UTC", now).unwrap();
        assert_eq!(late.latest_due(last, now), None);
    }

    /// A week with two tasks runs each on its day, in the automation's zone,
    /// and a task without a message runs never.
    #[test]
    fn a_weeks_tasks_run_each_at_its_moment() {
        let schedule = repeat(1, RepeatUnit::Weeks, None, vec![
            task("payments", "10:00"),
            Task { days: vec![Weekday::Wed], ..task("todos", "20:00") },
            Task { days: vec![Weekday::Fri], prompt: String::new(), ..task("unwritten", "09:00") },
        ]);
        let plan = Plan::new(&schedule, "Asia/Shanghai", at("2026-01-01T00:00:00Z")).unwrap();
        // From Tuesday noon in Shanghai: Wednesday 20:00, then Monday 10:00.
        assert_eq!(
            moments(&plan, "2026-10-20T04:00:00Z", 3),
            expect(&[
                ("2026-10-21T12:00:00Z", &["todos"]),
                ("2026-10-26T02:00:00Z", &["payments"]),
                ("2026-10-28T12:00:00Z", &["todos"]),
            ])
        );
    }

    /// Two tasks at the same moment are due together.
    #[test]
    fn tasks_at_one_moment_are_due_together() {
        let schedule = repeat(1, RepeatUnit::Days, None, vec![task("a", "09:00"), task("b", "09:00")]);
        let plan = Plan::new(&schedule, "UTC", at("2026-01-01T00:00:00Z")).unwrap();
        assert_eq!(moments(&plan, "2026-10-20T10:00:00Z", 1), expect(&[("2026-10-21T09:00:00Z", &["a", "b"])]));
    }

    /// Every two weeks from Monday 5 October runs in the weeks of 5 and 19
    /// October and 2 November, never in the week between.
    #[test]
    fn every_two_weeks_runs_in_the_first_week_of_each() {
        let schedule = repeat(2, RepeatUnit::Weeks, Some("2026-10-05"), vec![Task {
            days: vec![Weekday::Mon, Weekday::Thu],
            ..task("t", "09:00")
        }]);
        let plan = Plan::new(&schedule, "UTC", at("2026-01-01T00:00:00Z")).unwrap();
        assert_eq!(
            moments(&plan, "2026-10-09T10:00:00Z", 3),
            expect(&[
                ("2026-10-19T09:00:00Z", &["t"]),
                ("2026-10-22T09:00:00Z", &["t"]),
                ("2026-11-02T09:00:00Z", &["t"]),
            ])
        );
        // Before its first cycle nothing runs.
        assert_eq!(moments(&plan, "2026-09-01T00:00:00Z", 1), expect(&[("2026-10-05T09:00:00Z", &["t"])]));
    }

    /// Nothing runs before the first cycle's day, even in its unit: every 2
    /// months from 20 January with a task on the 1st first runs on 1 March;
    /// every 2 weeks from a Wednesday with a Monday task, on the Monday of
    /// the next cycle.
    #[test]
    fn nothing_runs_before_the_first_cycles_day() {
        let since = at("2026-01-01T00:00:00Z");
        let months = repeat(2, RepeatUnit::Months, Some("2027-01-20"), vec![task("t", "09:00")]);
        let plan = Plan::new(&months, "UTC", since).unwrap();
        assert_eq!(plan.next_after(at("2027-01-01T00:00:00Z")), Some(at("2027-03-01T09:00:00Z")));
        let weeks = repeat(2, RepeatUnit::Weeks, Some("2026-10-07"), vec![Task {
            days: vec![Weekday::Mon, Weekday::Fri],
            ..task("t", "09:00")
        }]);
        let plan = Plan::new(&weeks, "UTC", since).unwrap();
        assert_eq!(
            moments(&plan, "2026-10-01T00:00:00Z", 2),
            expect(&[("2026-10-09T09:00:00Z", &["t"]), ("2026-10-19T09:00:00Z", &["t"])])
        );
    }

    /// A backend down over two tasks' times runs each once, at its own
    /// latest time: Monday's and Wednesday's both, though Wednesday's is
    /// later.
    #[test]
    fn missed_times_fire_once_for_each_tasks_latest() {
        let schedule = repeat(1, RepeatUnit::Weeks, None, vec![
            task("a", "09:00"),
            Task { days: vec![Weekday::Mon, Weekday::Wed], ..task("b", "18:00") },
        ]);
        let plan = Plan::new(&schedule, "UTC", at("2026-01-01T00:00:00Z")).unwrap();
        // Last fired the Friday before; back on Thursday 10:00.
        let due = plan.due(at("2026-10-09T18:00:00Z"), at("2026-10-15T10:00:00Z"));
        assert_eq!(due, [(at("2026-10-12T09:00:00Z"), Some("a")), (at("2026-10-14T18:00:00Z"), Some("b"))]);
    }

    /// Every 3 months from January runs in January, April, July and
    /// October.
    #[test]
    fn every_three_months_runs_each_quarter() {
        let schedule = repeat(3, RepeatUnit::Months, Some("2026-01-01"), vec![task("t", "09:00")]);
        let plan = Plan::new(&schedule, "UTC", at("2026-01-01T00:00:00Z")).unwrap();
        assert_eq!(
            moments(&plan, "2026-01-01T09:00:00Z", 3),
            expect(&[
                ("2026-04-01T09:00:00Z", &["t"]),
                ("2026-07-01T09:00:00Z", &["t"]),
                ("2026-10-01T09:00:00Z", &["t"]),
            ])
        );
    }

    /// The last day runs every month, February's 28th included; the 31st is
    /// skipped where a month lacks it; a year's February 29 waits for a
    /// leap year; a weekday's last of a month is its last.
    #[test]
    fn a_months_day_is_read_as_the_month_has_it() {
        let since = at("2026-01-01T00:00:00Z");
        let cases: [(MonthDay, RepeatUnit, u8, &str, &[&str]); 4] = [
            (MonthDay::LastDay {}, RepeatUnit::Months, 1, "2026-02-01T00:00:00Z", &["2026-02-28T18:00:00Z", "2026-03-31T18:00:00Z", "2026-04-30T18:00:00Z"]),
            (MonthDay::Date { date: 31 }, RepeatUnit::Months, 1, "2026-03-31T18:00:00Z", &["2026-05-31T18:00:00Z", "2026-07-31T18:00:00Z"]),
            (MonthDay::Date { date: 29 }, RepeatUnit::Years, 2, "2026-01-01T00:00:00Z", &["2028-02-29T18:00:00Z", "2032-02-29T18:00:00Z"]),
            (MonthDay::Weekday { week: WeekOfMonth::Last, weekday: Weekday::Fri }, RepeatUnit::Months, 1, "2026-10-01T00:00:00Z", &["2026-10-30T18:00:00Z", "2026-11-27T18:00:00Z"]),
        ];
        for (day, unit, month, after, expected) in cases {
            let schedule = repeat(1, unit, None, vec![Task { day, month, ..task("t", "18:00") }]);
            let plan = Plan::new(&schedule, "UTC", since).unwrap();
            let found: Vec<Timestamp> = moments(&plan, after, expected.len()).into_iter().map(|(at, _)| at).collect();
            assert_eq!(found, expected.iter().map(|each| at(each)).collect::<Vec<_>>(), "{day:?} every {unit:?}");
        }
    }

    /// A working day skips the public holidays of the task's country and
    /// region: Scotland's Saint Andrew's Day, Monday 30 November 2026, moves
    /// its last working day to the Friday before, which the rest of the
    /// United Kingdom and Monday to Friday keep; China's National Day moves
    /// October's first working day to the 8th.
    #[test]
    fn a_working_day_skips_the_holidays_of_its_place() {
        let since = at("2026-01-01T00:00:00Z");
        let place = |country: &str, region: Option<&str>| {
            Some(HolidayPlace {
                country: country.into(),
                region: region.map(Into::into),
            })
        };
        let cases = [
            (FirstOrLast::Last, place("GB", Some("SCT")), "Europe/London", "2026-11-01T00:00:00Z", "2026-11-27T17:00:00Z"),
            (FirstOrLast::Last, place("GB", None), "Europe/London", "2026-11-01T00:00:00Z", "2026-11-30T17:00:00Z"),
            (FirstOrLast::Last, None, "Europe/London", "2026-11-01T00:00:00Z", "2026-11-30T17:00:00Z"),
            (FirstOrLast::First, place("CN", None), "Asia/Shanghai", "2026-09-30T00:00:00Z", "2026-10-08T09:00:00Z"),
        ];
        for (which, holidays, zone, after, expected) in cases {
            let schedule = repeat(1, RepeatUnit::Months, None, vec![Task {
                day: MonthDay::WorkingDay { which },
                holidays: holidays.clone(),
                ..task("t", "17:00")
            }]);
            let plan = Plan::new(&schedule, zone, since).unwrap();
            assert_eq!(plan.next_after(at(after)), Some(at(expected)), "{which:?} in {holidays:?}");
        }
        let unknown = repeat(1, RepeatUnit::Months, None, vec![Task {
            day: MonthDay::WorkingDay { which: FirstOrLast::Last },
            holidays: place("GB", Some("XX")),
            ..task("t", "17:00")
        }]);
        assert!(matches!(Plan::new(&unknown, "UTC", since), Err(PlanError::Holidays(_))));
    }

    #[test]
    fn a_cron_expression_cron_cannot_read_is_refused() {
        for expression in ["0 9 * *", "60 * * * *", "0 9 * * 8", "5-1 * * * *", "*/0 * * * *", "0 9 * foo *"] {
            assert!(Cron::parse(expression).is_err(), "{expression}");
        }
        assert!(zone("Mars/Olympus").is_err());
    }

    /// A day in Shanghai starts at its midnight, 16:00 the day before in
    /// UTC.
    #[test]
    fn a_daily_limit_counts_from_local_midnight() {
        let zone = zone("Asia/Shanghai").unwrap();
        let (start, next) = day_bounds(at("2026-10-17T01:00:00Z"), &zone);
        assert_eq!(start, at("2026-10-16T16:00:00Z"));
        assert_eq!(next, at("2026-10-17T16:00:00Z"));
    }
}
