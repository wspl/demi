import { pageLanguage } from '../i18n/text'
import { t } from '../locales/text'

// The interface's dates, times, ages and spans, in the page's language
// (`localization.md` § Where the text lives): Intl says each date and time
// as the language does ("6:12 PM", "下午6:12"), and the catalogue words the
// compact forms Intl has no words for ("2h", "2 小时").

const MINUTE_MS = 60_000
const HOUR_MS = 60 * MINUTE_MS
const DAY_MS = 24 * HOUR_MS
const WEEK_MS = 7 * DAY_MS

/** The moment `timestamp` in Intl's words of `options`. */
function intl(timestamp: Date, options: Intl.DateTimeFormatOptions): string {
  return new Intl.DateTimeFormat(pageLanguage(), options).format(timestamp)
}

/** A time on the clock: "6:12 PM". */
function clock(moment: Date): string {
  return intl(moment, { hour: 'numeric', minute: '2-digit' })
}

/** A day of `now`'s year without its year, "Sep 2", or of another with it, "Sep 2, 2025". */
function day(moment: Date, now: Date): string {
  const year = moment.getFullYear() === now.getFullYear() ? undefined : 'numeric'
  return intl(moment, { month: 'short', day: 'numeric', year })
}

/** A weekday in short: "Sat". */
function weekday(moment: Date): string {
  return intl(moment, { weekday: 'short' })
}

/** Whether `a` and `b` fall on one day of the local calendar. */
function sameDay(a: Date, b: Date): boolean {
  return a.toDateString() === b.toDateString()
}

/** `moment` moved to the start of its minute, or of the next one with `up` when it is inside one. */
function toMinute(moment: Date, up: boolean): Date {
  const start = Math.floor(moment.getTime() / MINUTE_MS) * MINUTE_MS
  return new Date(up && start < moment.getTime() ? start + MINUTE_MS : start)
}

/**
 * A distance in time as Intl says it for the page's language, rounded to
 * its largest unit as a person would: "3 minutes ago", "in 2 hours".
 */
function relative(ms: number): string {
  const format = new Intl.RelativeTimeFormat(pageLanguage(), { numeric: 'auto' })
  const sign = Math.sign(ms) || 1
  const seconds = Math.abs(ms) / 1000
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ['minute', 60],
    ['hour', 60 * 60],
    ['day', 24 * 60 * 60],
    ['month', 30 * 24 * 60 * 60],
    ['year', 365 * 24 * 60 * 60],
  ]
  // Each unit holds until the next one would round to one: 45 minutes is
  // "1 hour", 22 hours "1 day", 26 days "1 month" and 11 months "1 year".
  const next: Record<string, number> = { minute: 45 * 60, hour: 22 * 60 * 60, day: 26 * 24 * 60 * 60, month: 320 * 24 * 60 * 60 }
  for (const [unit, size] of units) {
    if (unit === 'year' || seconds < next[unit]!) {
      return format.format(sign * Math.max(1, Math.round(seconds / size)), unit)
    }
  }
  throw new Error('unreachable: the year unit always answers')
}

/**
 * A moment that has passed, in words against `now`: "3 minutes ago". One
 * `now` has not reached yet, as a reply the backend stamped within the
 * clock's last second or by a clock running slightly ahead, reads "a few
 * seconds ago" rather than "in a few seconds".
 */
export function formatPast(timestamp: string, now: Date | number): string {
  const past = Math.max(0, new Date(now).getTime() - new Date(timestamp).getTime())
  return past < 45_000 ? t('dates.fewSecondsAgo') : relative(-past)
}

/**
 * A moment to come, in words against `now`: "in 8 minutes". One `now` has
 * passed reads "in a few seconds", since what it names is about to happen.
 */
export function formatUntil(timestamp: string, now: Date | number): string {
  const left = new Date(timestamp).getTime() - new Date(now).getTime()
  return left < 45_000 ? t('dates.inFewSeconds') : relative(left)
}

/**
 * How long ago a moment was, as a list of commits says it beside each: "now"
 * within a minute, then "5m", "2h", "3d", "4w", then the day it was, "Sep
 * 2", with the year when it is not `now`'s.
 */
export function formatAge(timestamp: string, now: Date | number): string {
  const moment = new Date(timestamp)
  const at = new Date(now)
  const past = Math.max(0, at.getTime() - moment.getTime())
  if (past < MINUTE_MS)
    return t('dates.age.now')
  if (past < HOUR_MS)
    return t('dates.age.minutes', { count: Math.floor(past / MINUTE_MS) })
  if (past < DAY_MS)
    return t('dates.age.hours', { count: Math.floor(past / HOUR_MS) })
  if (past < WEEK_MS)
    return t('dates.age.days', { count: Math.floor(past / DAY_MS) })
  if (past < 5 * WEEK_MS)
    return t('dates.age.weeks', { count: Math.floor(past / WEEK_MS) })
  return day(moment, at)
}

/**
 * A calendar date with no time of day, `YYYY-MM-DD` such as a model's
 * release, as the day it names wherever the reader is: "Apr 21, 2026".
 */
export function formatDate(date: string): string {
  return new Intl.DateTimeFormat(pageLanguage(), { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' })
    .format(new Date(`${date}T00:00:00Z`))
}

/** A day in full: "October 7, 2026". */
export function formatDay(timestamp: string): string {
  return intl(new Date(timestamp), { month: 'long', day: 'numeric', year: 'numeric' })
}

/** The Modified column and a file's card, one shape for every row: "Sep 2, 2026 6:12 PM"; empty for an unknown time. */
export function formatModified(iso: string | undefined): string {
  if (!iso) {
    return ''
  }
  const time = new Date(iso)
  if (Number.isNaN(time.getTime())) {
    return ''
  }
  return `${intl(time, { month: 'short', day: 'numeric', year: 'numeric' })} ${clock(time)}`
}

/**
 * A timestamp as a moment to the minute: "Thursday, October 9 at 2:00 PM",
 * with its year when that is not `now`'s year.
 */
export function formatMoment(timestamp: string, now: Date = new Date()): string {
  const moment = new Date(timestamp)
  const year = moment.getFullYear() === now.getFullYear() ? undefined : 'numeric'
  return intl(moment, { weekday: 'long', month: 'long', day: 'numeric', year, hour: 'numeric', minute: '2-digit' })
}

/**
 * A span of time in the interface's duration words, counted as a timer
 * counts: "42s", "2m 14s" (the seconds left out on the minute), "1h 5m"
 * (the minutes left out on the hour, the seconds never shown). Every unit is
 * floored, so a span reads as what has fully passed. The thinking label, the
 * command time tag, the agent status bar and the reset times all say their
 * spans with it.
 */
export function formatSpan(ms: number): string {
  const seconds = Math.floor(Math.max(0, ms) / 1000)
  if (seconds < 60) {
    return t('dates.span.seconds', { seconds })
  }
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) {
    return seconds % 60
      ? t('dates.span.minutesSeconds', { minutes, seconds: seconds % 60 })
      : t('dates.span.minutes', { minutes })
  }
  const hours = Math.floor(minutes / 60)
  return minutes % 60
    ? t('dates.span.hoursMinutes', { hours, minutes: minutes % 60 })
    : t('dates.span.hours', { hours })
}

/**
 * A moment to come, `timestamp`, after `verb`: when a failed request can
 * succeed again, as a failure's record says it and as the control that
 * resumes the turn then names it (`turns.md` § Recovering an unfinished
 * turn), or when a credential kept for a while is removed (`credentials.md`).
 * "Resets at 9:00 AM" or "Removed today at 6:00 PM" on `now`'s day, "Resume
 * Sat at 9:00 AM" within a week either way, then "Resets Oct 24 at 9:00 AM",
 * with the year when it is not this one. The moment is rounded up to the
 * minute, so it never names a time before the moment: 7:03:55 PM is 7:04 PM.
 * `resetting` says it inside a sentence, as the line that says an agent moved
 * to another account does.
 */
export function formatVerbAt(verb: 'Resets' | 'Resume' | 'resetting' | 'Removed', timestamp: string, now: Date): string {
  const moment = toMinute(new Date(timestamp), true)
  const at = clock(moment)
  if (sameDay(moment, now)) {
    return t(`dates.when.${verb}.today`, { clock: at })
  }
  if (Math.abs(moment.getTime() - now.getTime()) < WEEK_MS - DAY_MS) {
    return t(`dates.when.${verb}.weekday`, { weekday: weekday(moment), clock: at })
  }
  return t(`dates.when.${verb}.day`, { day: day(moment, now), clock: at })
}

/**
 * When a usage window resets, as specifically as the user would say it,
 * counted from `now` in the user's time zone (`usage-and-quota.md` § What a
 * snapshot says): "Resets now" once the clock has passed it, "Resets in <1m"
 * within a minute, "Resets in 42m" within the hour, "Resets in 3h 42m"
 * within a day, the minutes left out on the hour, "Resets Sat 4:00 PM" from
 * 24 hours to under 7 days, then the day alone, "Resets Oct 24", with the
 * year when it is not this year's: "Resets Jan 3, 2027". The row is chosen
 * by the exact time left and the countdown counts it down like a timer, so
 * 59m 59s left reads "in 59m"; a moment named on the clock is its nearest
 * minute.
 */
export function formatReset(timestamp: string, now: Date): string {
  const left = new Date(timestamp).getTime() - now.getTime()
  if (left <= 0) {
    return t('dates.reset.now')
  }
  if (left < MINUTE_MS) {
    return t('dates.reset.underMinute')
  }
  if (left < HOUR_MS) {
    return t('dates.reset.in', { span: t('dates.span.minutes', { minutes: Math.floor(left / MINUTE_MS) }) })
  }
  if (left < DAY_MS) {
    return t('dates.reset.in', { span: formatSpan(left) })
  }
  // A vendor states a reset to the second or less, as 3:59:59.6 PM for
  // 4:00 PM: a moment is named by its nearest minute.
  const moment = toMinute(new Date(new Date(timestamp).getTime() + 30_000), false)
  if (left < WEEK_MS) {
    // 7 days ahead would name today's weekday, so the weekday stops at 6.
    return t('dates.reset.weekday', { weekday: weekday(moment), clock: clock(moment) })
  }
  return t('dates.reset.day', { day: day(moment, now) })
}
