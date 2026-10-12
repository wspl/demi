import { computed, onScopeDispose, watch } from 'vue'
import { clockNow, holdClock } from './useClock'

/** The time from `startedAt` to `endedAt`, none until both are known; a clock set back reads 0. */
export function elapsedBetween(startedAt: number, endedAt: number | null): number | null {
  if (!Number.isFinite(startedAt) || endedAt === null || !Number.isFinite(endedAt)) {
    return null
  }
  return Math.max(0, endedAt - startedAt)
}

/** Measures a live interval or a persisted interval; a live one holds the shared clock until it ends or its scope stops. */
export function useElapsedTime(
  startedAt: () => number,
  running: () => boolean,
  endedAt: () => number | null = () => null,
) {
  let release: (() => void) | null = null

  function stop(): void {
    release?.()
    release = null
  }

  watch(
    () => running() && endedAt() === null,
    (live) => {
      if (live) {
        release ??= holdClock()
      } else {
        stop()
      }
    },
    { immediate: true },
  )
  onScopeDispose(stop)

  return computed(() => elapsedBetween(startedAt(), endedAt() ?? (running() ? clockNow.value : null)))
}
