import { formatSpan } from '@demicodes/utils'
import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'

dayjs.extend(relativeTime)

// The interface's dates and times, whatever the browser's locale: English,
// with a 12-hour clock everywhere ("6:12 PM", never "18:12"). The pieces
// below are the ones every wording shares.
const CLOCK = 'h:mm A'
const MONTH_DAY = 'MMM D'
const MONTH_DAY_YEAR = 'MMM D, YYYY'

const MINUTE_MS = 60_000
const HOUR_MS = 60 * MINUTE_MS
const DAY_MS = 24 * HOUR_MS
const WEEK_MS = 7 * DAY_MS

/** `inYear` for a moment in `now`'s year, `otherYear` for one in any other: a year is named only when it is not this one. */
function byYear(moment: dayjs.Dayjs, now: Date, inYear: string, otherYear: string): string {
  return moment.year() === dayjs(now).year() ? inYear : otherYear
}

/**
 * A moment that has passed, in words against `now`: "3 minutes ago". One
 * `now` has not reached yet, as a reply the backend stamped within the
 * clock's last second or by a clock running slightly ahead, reads "a few
 * seconds ago" rather than "in a few seconds".
 */
export function formatPast(timestamp: string, now: Date | number): string {
  const moment = dayjs(timestamp)
  return (moment.isAfter(now) ? dayjs(now) : moment).from(now)
}

/**
 * A moment to come, in words against `now`: "in 8 minutes". One `now` has
 * passed reads "in a few seconds", since what it names is about to happen.
 */
export function formatUntil(timestamp: string, now: Date | number): string {
  const moment = dayjs(timestamp)
  return moment.isAfter(now) ? moment.from(now) : 'in a few seconds'
}

/**
 * How long ago a moment was, as a list of commits says it beside each: "now"
 * within a minute, then "5m", "2h", "3d", "4w", then the day it was, "Sep
 * 2", with the year when it is not `now`'s.
 */
export function formatAge(timestamp: string, now: Date | number): string {
  const moment = dayjs(timestamp)
  const past = Math.max(0, dayjs(now).diff(moment))
  if (past < MINUTE_MS)
    return 'now'
  if (past < HOUR_MS)
    return `${Math.floor(past / MINUTE_MS)}m`
  if (past < DAY_MS)
    return `${Math.floor(past / HOUR_MS)}h`
  if (past < WEEK_MS)
    return `${Math.floor(past / DAY_MS)}d`
  if (past < 5 * WEEK_MS)
    return `${Math.floor(past / WEEK_MS)}w`
  return moment.format(byYear(moment, now instanceof Date ? now : new Date(now), MONTH_DAY, MONTH_DAY_YEAR))
}

/** A day in the interface's English: "October 7, 2026". */
export function formatDay(timestamp: string): string {
  return dayjs(timestamp).format('MMMM D, YYYY')
}

/** The Modified column and a file's card, one shape for every row: "Sep 2, 2026 6:12 PM"; empty for an unknown time. */
export function formatModified(iso: string | undefined): string {
  if (!iso) {
    return ''
  }
  const time = dayjs(iso)
  return time.isValid() ? time.format(`${MONTH_DAY_YEAR} ${CLOCK}`) : ''
}

/**
 * A timestamp as a moment to the minute: "Thursday, October 9 at 2:00 PM",
 * with its year when that is not `now`'s year.
 */
export function formatMoment(timestamp: string, now: Date = new Date()): string {
  const moment = dayjs(timestamp)
  return moment.format(byYear(moment, now, `dddd, MMMM D [at] ${CLOCK}`, `dddd, MMMM D, YYYY [at] ${CLOCK}`))
}

/**
 * When a failed request can succeed again, `timestamp`, after `verb`, as a
 * failure's record says it and as the control that resumes the turn then
 * names it (`product.md` § Recovering an unfinished turn): "Resets at
 * 9:00 AM" on `now`'s day, "Resume Sat at 9:00 AM" within a week either way,
 * then "Resets Oct 24 at 9:00 AM", with the year when it is not this one. The
 * moment is rounded up to the minute, so neither ever names a time before
 * the reset: 7:03:55 PM is 7:04 PM.
 */
export function formatRetryAt(verb: 'Resets' | 'Resume', timestamp: string, now: Date): string {
  const exact = dayjs(timestamp)
  const minute = exact.startOf('minute')
  const moment = minute.isSame(exact) ? minute : minute.add(1, 'minute')
  const clock = moment.format(CLOCK)
  if (moment.isSame(now, 'day')) {
    return `${verb} at ${clock}`
  }
  if (Math.abs(moment.diff(now)) < WEEK_MS - DAY_MS) {
    return `${verb} ${moment.format('ddd')} at ${clock}`
  }
  return `${verb} ${moment.format(byYear(moment, now, MONTH_DAY, MONTH_DAY_YEAR))} at ${clock}`
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
  const left = dayjs(timestamp).diff(now)
  if (left <= 0) {
    return 'Resets now'
  }
  if (left < MINUTE_MS) {
    return 'Resets in <1m'
  }
  if (left < HOUR_MS) {
    return `Resets in ${Math.floor(left / MINUTE_MS)}m`
  }
  if (left < DAY_MS) {
    return `Resets in ${formatSpan(left)}`
  }
  // A vendor states a reset to the second or less, as 3:59:59.6 PM for
  // 4:00 PM: a moment is named by its nearest minute.
  const moment = dayjs(timestamp).add(30, 'second').startOf('minute')
  if (left < WEEK_MS) {
    // 7 days ahead would name today's weekday, so the weekday stops at 6.
    return `Resets ${moment.format(`ddd ${CLOCK}`)}`
  }
  return `Resets ${moment.format(byYear(moment, now, MONTH_DAY, MONTH_DAY_YEAR))}`
}
