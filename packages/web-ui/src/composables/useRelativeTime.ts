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

/** Time left before a future timestamp, ticking on the shared clock; negative once it passes. */
export function useTimeRemaining(timestamp: MaybeRefOrGetter<string>) {
  const now = useClock()
  return computed(() => dayjs(toValue(timestamp)).diff(now.value))
}

/** Countdown label for an expose: minutes, then seconds once under a minute; never "60 s". */
export function formatTimeRemaining(remainingMs: number): string {
  if (remainingMs <= 0) {
    return 'Expired'
  }
  const minutes = Math.floor(remainingMs / 60_000)
  return minutes >= 1
    ? `${minutes} min left`
    : `${Math.floor(remainingMs / 1000)} s left`
}
