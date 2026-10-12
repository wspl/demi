import { computed, toValue, type MaybeRefOrGetter } from 'vue'
import { createSharedComposable, useNow } from '@vueuse/core'
import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'

dayjs.extend(relativeTime)

// VueUse releases the shared clock when its last consuming scope is disposed.
const useClock = createSharedComposable(() => useNow({ interval: 1000 }))

/** A moment that has passed, in words against the shared clock, which ticks each second. */
export function useRelativeTime(timestamp: MaybeRefOrGetter<string>) {
  const now = useClock()
  return computed(() => formatPast(toValue(timestamp), now.value))
}

/**
 * A moment that has passed, in words against `now`: "3 minutes ago". One
 * `now` has not reached yet, as a reply the backend stamped within the
 * clock's last second or by a clock running slightly ahead, reads "a few
 * seconds ago" rather than "in a few seconds".
 */
export function formatPast(timestamp: string, now: Date): string {
  const moment = dayjs(timestamp)
  return (moment.isAfter(now) ? dayjs(now) : moment).from(now)
}

/**
 * A moment to come, in words against the shared clock: "in 8 minutes". One
 * the clock has passed reads "in a few seconds", since what it names is
 * about to happen.
 */
export function useTimeUntil(timestamp: MaybeRefOrGetter<string>) {
  const now = useClock()
  return computed(() => {
    const moment = dayjs(toValue(timestamp))
    return moment.isAfter(now.value) ? moment.from(now.value) : 'in a few seconds'
  })
}

/** A day in the interface's English: "October 7, 2026". */
export function formatDay(timestamp: string): string {
  return dayjs(timestamp).format('MMMM D, YYYY')
}

const MINUTE_MS = 60_000
const HOUR_MS = 60 * MINUTE_MS
const DAY_MS = 24 * HOUR_MS
const WEEK_MS = 7 * DAY_MS

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
    const hours = Math.floor(left / HOUR_MS)
    const minutes = Math.floor((left % HOUR_MS) / MINUTE_MS)
    return minutes ? `Resets in ${hours}h ${minutes}m` : `Resets in ${hours}h`
  }
  // A vendor states a reset to the second or less, as 3:59:59.6 PM for
  // 4:00 PM: a moment is named by its nearest minute.
  const moment = dayjs(timestamp).add(30, 'second').startOf('minute')
  if (left < WEEK_MS) {
    // 7 days ahead would name today's weekday, so the weekday stops at 6.
    return `Resets ${moment.format('ddd h:mm A')}`
  }
  const thisYear = moment.year() === dayjs(now).year()
  return `Resets ${moment.format(thisYear ? 'MMM D' : 'MMM D, YYYY')}`
}

/** When a usage window resets ([`formatReset`]), against the shared clock, which ticks each second. */
export function useResetTime(timestamp: MaybeRefOrGetter<string>) {
  const now = useClock()
  return computed(() => formatReset(toValue(timestamp), now.value))
}

/**
 * A timestamp as a moment to the minute, in the interface's English whatever
 * the browser's locale: "Thursday, October 9 at 2:00 PM", with its year when
 * that is not this year's.
 */
export function formatMoment(timestamp: string): string {
  const moment = dayjs(timestamp)
  const thisYear = moment.year() === dayjs().year()
  return moment.format(thisYear ? 'dddd, MMMM D [at] h:mm A' : 'dddd, MMMM D, YYYY [at] h:mm A')
}
