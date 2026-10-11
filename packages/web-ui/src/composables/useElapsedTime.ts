import { computed, onScopeDispose, ref, watch } from 'vue'

/**
 * The page's one clock for live intervals: every running row reads the same
 * `now`, which one timer moves each second while any row runs, so the rows
 * tick together and a page of running commands keeps one timer, not one each.
 */
const now = ref(Date.now())
let timer: ReturnType<typeof setInterval> | undefined
let readers = 0

/** A row starts reading the clock: the first one starts its timer. */
function holdClock(): void {
  now.value = Date.now()
  readers += 1
  if (timer === undefined) {
    timer = setInterval(() => {
      now.value = Date.now()
    }, 1000)
  }
}

/** A row stops reading the clock: the last one stops its timer. */
function releaseClock(): void {
  readers -= 1
  if (readers === 0 && timer !== undefined) {
    clearInterval(timer)
    timer = undefined
  }
}

/** Measures a live interval or a persisted interval; each stopped scope releases its hold on the shared clock. */
export function useElapsedTime(
  startedAt: () => number,
  running: () => boolean,
  endedAt: () => number | null = () => null,
) {
  let holding = false

  function release(): void {
    if (holding) {
      holding = false
      releaseClock()
    }
  }

  watch(
    () => running() && endedAt() === null,
    (live) => {
      if (live && !holding) {
        holding = true
        holdClock()
      } else if (!live) {
        release()
      }
    },
    { immediate: true },
  )
  onScopeDispose(release)

  return computed(() => {
    const start = startedAt()
    const end = endedAt() ?? (running() ? now.value : null)
    if (!Number.isFinite(start) || end === null || !Number.isFinite(end)) {
      return null
    }
    return Math.max(0, end - start)
  })
}
