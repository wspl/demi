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
const HOUR_MINUTES = 60
const DAY_MINUTES = 24 * HOUR_MINUTES
const WEEK_MINUTES = 7 * DAY_MINUTES

/**
 * When a usage window resets, as specifically as the user would say it,
 * counted from `now` in the user's time zone (`usage-and-quota.md` § What a
 * snapshot says): "Resets in 12 min" within the hour, "Resets in 3 hr
 * 38 min" within a day, the minutes left out on the hour, "Resets Sat
 * 4:00 PM" within a week, then "Resets Oct 15 at 4:00 PM", with the year
 * when it is not this year's. Minutes count up, so the last one reads
 * "in 1 min", as does a reset the clock has just passed.
 */
export function formatReset(timestamp: string, now: Date): string {
  const moment = dayjs(timestamp)
  const minutes = Math.max(1, Math.ceil(moment.diff(now) / MINUTE_MS))
  if (minutes < HOUR_MINUTES) {
    return `Resets in ${minutes} min`
  }
  if (minutes < DAY_MINUTES) {
    const hours = Math.floor(minutes / HOUR_MINUTES)
    const rest = minutes % HOUR_MINUTES
    return rest ? `Resets in ${hours} hr ${rest} min` : `Resets in ${hours} hr`
  }
  if (minutes < WEEK_MINUTES) {
    return `Resets ${moment.format('ddd h:mm A')}`
  }
  const thisYear = moment.year() === dayjs(now).year()
  return `Resets ${moment.format(thisYear ? 'MMM D [at] h:mm A' : 'MMM D, YYYY [at] h:mm A')}`
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
