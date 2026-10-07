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

/** A timestamp against now, in words: "in 2 days", "3 hours ago". A label for one render, not a ticking one. */
export function formatRelativeTime(timestamp: string): string {
  return dayjs(timestamp).fromNow()
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
