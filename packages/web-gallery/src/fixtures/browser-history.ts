import { ref, shallowRef } from 'vue'
import type { HistoryDirection } from '@demicodes/web-ui/plugins/page'

/** A step of a specimen's history: what it shows, open or closed. */
interface Shown {
  open: boolean
}

/**
 * The web browser's history of one specimen's view, as the product keeps it
 * in the page's address (`docs/decisions/0014-the-work-panel-has-addresses.md`):
 * `step` adds what `current` reads as a step after the one shown, `replace`
 * takes the shown one's place, and Back and Forward `restore` the step they
 * reach, available while it shows the view open.
 */
export function galleryHistory<T extends Shown>(current: () => T, restore: (step: T) => void) {
  const steps = shallowRef<T[]>([current()])
  const at = ref(0)
  return {
    step(): void {
      steps.value = [...steps.value.slice(0, at.value + 1), current()]
      at.value = steps.value.length - 1
    },
    replace(): void {
      steps.value = steps.value.map((step, index) => (index === at.value ? current() : step))
    },
    canGo(direction: HistoryDirection): boolean {
      const next = steps.value[at.value + (direction === 'back' ? -1 : 1)]
      return current().open && next?.open === true
    },
    go(direction: HistoryDirection): void {
      const index = at.value + (direction === 'back' ? -1 : 1)
      const next = steps.value[index]
      if (next) {
        at.value = index
        restore(next)
      }
    },
    /** The history as a specimen starts it again: what it shows, alone. */
    reset(): void {
      steps.value = [current()]
      at.value = 0
    },
  }
}
