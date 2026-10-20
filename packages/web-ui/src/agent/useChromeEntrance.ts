import { onScopeDispose, ref, watch } from 'vue'
import { CHROME_ENTER_MS } from '../ui/chrome-enter'
import { isOpening, type SessionLoad } from './session-status'

/**
 * Which rows of the transcript are arriving right now, by the keys that name
 * them across their forms (`row-key.ts`).
 *
 * - Rows present when the list opens or when a step of the conversation's
 *   opening lands are history and do not move.
 * - A row that joins afterwards enters once, then is settled, so a virtual row
 *   remounted by scrolling stays still.
 * - A row that changes form stays still too, as does a row that takes another's
 *   place under its key (`succeedKeys`).
 * - A new conversation in the same list starts over from its own history.
 */
export function useChromeEntrance(
  keys: () => readonly string[],
  scope: () => string,
  load: () => SessionLoad,
): { isEntering: (key: string) => boolean } {
  let settled = new Set(keys())
  const entering = ref(new Set<string>())
  const timers = new Set<ReturnType<typeof setTimeout>>()

  function settle(id: string): void {
    settled.add(id)
    if (entering.value.delete(id)) {
      entering.value = new Set(entering.value)
    }
  }

  function enter(id: string): void {
    settled.add(id)
    entering.value = new Set(entering.value).add(id)
    const timer = setTimeout(() => {
      timers.delete(timer)
      settle(id)
    }, CHROME_ENTER_MS)
    timers.add(timer)
  }

  function clearTimers(): void {
    for (const timer of timers) {
      clearTimeout(timer)
    }
    timers.clear()
  }

  watch(
    () => [scope(), load(), keys()] as const,
    ([currentScope, currentLoad, current], [previousScope, previousLoad]) => {
      if (
        currentScope !== previousScope
        || currentLoad === 'loading'
        || isOpening(previousLoad)
        || currentLoad === 'failed'
      ) {
        clearTimers()
        settled = new Set(current)
        entering.value = new Set()
        return
      }
      for (const id of current) {
        if (!settled.has(id)) {
          enter(id)
        }
      }
    },
  )

  onScopeDispose(clearTimers)

  return {
    isEntering: (key) => entering.value.has(key),
  }
}
