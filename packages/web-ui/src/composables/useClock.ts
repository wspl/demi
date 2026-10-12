import { getCurrentScope, onScopeDispose, readonly, ref, type Ref } from 'vue'

/**
 * The page's one clock: every live time, a running command's elapsed time or
 * a relative time such as "3 minutes ago", reads the same `now`, which one
 * timer moves each second while any reader holds it, so the rows tick
 * together and a page of live times keeps one timer, not one each.
 */
const now = ref(Date.now())
let timer: ReturnType<typeof setInterval> | undefined
let readers = 0

/**
 * Starts reading the clock: the first reader starts the timer. Answers the
 * release, which is safe to call again; the last release stops the timer.
 */
export function holdClock(): () => void {
  now.value = Date.now()
  readers += 1
  if (timer === undefined) {
    timer = setInterval(() => {
      now.value = Date.now()
    }, 1000)
  }
  let held = true
  return () => {
    if (!held) {
      return
    }
    held = false
    readers -= 1
    if (readers === 0 && timer !== undefined) {
      clearInterval(timer)
      timer = undefined
    }
  }
}

/** The clock's `now` in milliseconds as of its last tick; it moves only while some reader holds the clock. */
export const clockNow: Readonly<Ref<number>> = readonly(now)

/** The clock's `now` in milliseconds, held until the calling scope is disposed. */
export function useClock(): Readonly<Ref<number>> {
  if (getCurrentScope()) {
    onScopeDispose(holdClock())
  }
  return clockNow
}
