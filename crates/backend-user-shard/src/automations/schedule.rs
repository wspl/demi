//! When an automation fires by itself.
//!
//! This module finds the next time of a schedule after a moment, in its time
//! zone, and the latest one a backend that was down missed, which fires once.
//!
//! Daylight saving time follows cron:
//!
//! - A clock time that a change to summer time skips fires at the first minute
//!   after the gap.
//! - One that the change back repeats fires the first time.
//!
//! No crate the workspace uses reads cron expressions over `jiff`'s time zones,
//! so the five fields are read here.

use demi_web_api_protocol::automations::{Schedule, Weekday};
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
    Daily { time: Time, days: u8 },
    Every { minutes: i64 },
    Cron(Cron),
}

/// Why a schedule cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PlanError {
    #[error("unknown time zone {0:?}")]
    TimeZone(String),
    #[error("cron expression {expression:?}: {reason}")]
    Cron { expression: String, reason: String },
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
            Schedule::Daily { time, days } => Kind::Daily {
                time: clock(time).expect("the contract checked the clock time"),
                days: days.iter().map(|day| weekday_bit(*day)).fold(0, |all, bit| all | bit),
            },
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
            Kind::Daily { time, days } => {
                let after = after.max(self.since);
                self.search(after, |date| {
                    if days & day_bit(date.weekday()) == 0 {
                        return Vec::new();
                    }
                    vec![*time]
                })
            }
            Kind::Cron(cron) => {
                let after = after.max(self.since);
                self.search(after, |date| cron.times_on(date))
            }
        }
    }

    /// The latest time at or before `now` after `after`, which a backend
    /// that was down missed: one run stands for all of them.
    pub(crate) fn latest_due(&self, after: Timestamp, now: Timestamp) -> Option<Timestamp> {
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

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn daily(time: &str, days: &[Weekday]) -> Schedule {
        Schedule::Daily {
            time: time.into(),
            days: days.to_vec(),
        }
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
            (daily("09:00", &WEEKDAYS), "Asia/Shanghai", "2026-10-16T02:00:00Z", Some("2026-10-19T01:00:00Z")),
            (daily("09:00", &WEEKDAYS), "Asia/Shanghai", "2026-10-15T00:59:00Z", Some("2026-10-15T01:00:00Z")),
            // 02:30 does not exist on 8 March in New York: the first minute
            // after the gap, 03:00 EDT; a cron time in it too, once.
            (daily("02:30", &Weekday::ALL), "America/New_York", "2026-03-07T12:00:00Z", Some("2026-03-08T07:00:00Z")),
            (cron("15,45 2 * * *"), "America/New_York", "2026-03-08T06:59:00Z", Some("2026-03-08T07:00:00Z")),
            (cron("15,45 2 * * *"), "America/New_York", "2026-03-08T07:00:00Z", Some("2026-03-09T06:15:00Z")),
            // 01:30 happens twice on 1 November: the first, then 2 November.
            (daily("01:30", &Weekday::ALL), "America/New_York", "2026-10-31T12:00:00Z", Some("2026-11-01T05:30:00Z")),
            (daily("01:30", &Weekday::ALL), "America/New_York", "2026-11-01T05:30:00Z", Some("2026-11-02T06:30:00Z")),
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
        let plan = Plan::new(&daily("09:00", &WEEKDAYS), "UTC", since).unwrap();
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
        let late = Plan::new(&daily("09:00", &WEEKDAYS), "UTC", now).unwrap();
        assert_eq!(late.latest_due(last, now), None);
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
