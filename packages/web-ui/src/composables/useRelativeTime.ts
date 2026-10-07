import { computed, toValue, type MaybeRefOrGetter } from 'vue'
import { createSharedComposable, useNow } from '@vueuse/core'
import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'

dayjs.extend(relativeTime)

// VueUse releases the shared clock when its last consuming scope is disposed.
const useClock = createSharedComposable(() => useNow({ interval: 1000 }))

export function useRelativeTime(timestamp: MaybeRefOrGetter<string>) {
  const now = useClock()
  return computed(() => dayjs(toValue(timestamp)).from(now.value))
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
