import { onBeforeUnmount, ref, watch } from 'vue'
import { CHROME_ENTER_MS } from '../ui/chrome-enter'
import type { MessageListBlock } from './pending-steers'

/**
 * Which transcript blocks are arriving right now. Blocks present when the
 * list opens are history and do not move; a block that joins the transcript
 * afterwards enters once, then is settled, so a virtual row remounted by
 * scrolling stays still. A block handed off through the activity slot is
 * settled before it becomes a row: the slot already carried it into place.
 * A new conversation in the same list starts over from its own history.
 */
export function useChromeEntrance(
  blocks: () => readonly MessageListBlock[],
  heldId: () => string | null,
  scope: () => string,
): { isEntering: (id: string) => boolean } {
  let settled = new Set(blocks().map((block) => block.id))
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

  watch(heldId, (id) => {
    if (id !== null) {
      settled.add(id)
    }
  }, { flush: 'sync' })

  watch(scope, () => {
    clearTimers()
    settled = new Set(blocks().map((block) => block.id))
    entering.value = new Set()
  })

  watch(blocks, (current) => {
    for (const block of current) {
      if (!settled.has(block.id)) {
        enter(block.id)
      }
    }
  })

  onBeforeUnmount(clearTimers)

  return {
    isEntering: (id) => entering.value.has(id),
  }
}
