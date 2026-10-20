import { computed, toValue, type MaybeRefOrGetter } from 'vue'
import { formatPast, formatReset, formatUntil } from '../ui/dates'
import { useClock } from './useClock'

/**
 * A moment that has passed, in words ([`formatPast`]) against the shared clock,
 * which ticks each second.
 */
export function useRelativeTime(timestamp: MaybeRefOrGetter<string>) {
  const now = useClock()
  return computed(() => formatPast(toValue(timestamp), now.value))
}

/** A moment to come, in words ([`formatUntil`]) against the shared clock. */
export function useTimeUntil(timestamp: MaybeRefOrGetter<string>) {
  const now = useClock()
  return computed(() => formatUntil(toValue(timestamp), now.value))
}

/**
 * When a usage window resets ([`formatReset`]), against the shared clock, which
 * ticks each second.
 */
export function useResetTime(timestamp: MaybeRefOrGetter<string>) {
  const now = useClock()
  return computed(() => formatReset(toValue(timestamp), new Date(now.value)))
}
