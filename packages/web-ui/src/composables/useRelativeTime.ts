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

