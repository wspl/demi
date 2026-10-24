import { ref } from 'vue'
import { createFingerDrag } from '../ui/finger-drag'

/** How far a swipe lays the Delete bare, in px. */
export const REVEAL_PX = 88

/**
 * A row a finger swipes to the left to lay its Delete bare, as iOS lists do. The
 * row follows the finger and settles by the drawers' rule (`createFingerDrag`);
 * a tap on a row that shows its Delete puts it away.
 */
export function useSwipeDelete() {
  const offset = ref(0)
  const revealed = ref(false)
  const dragging = ref(false)
  const drag = createFingerDrag({
    way: -1,
    size: () => REVEAL_PX,
    hold: () => { dragging.value = true },
    follow: (moved) => { offset.value = -Math.min(moved, REVEAL_PX * 1.5) },
    settle: (goes) => {
      dragging.value = false
      revealed.value = goes
      offset.value = goes ? -REVEAL_PX : 0
    },
  })
  function tap(): void {
    if (revealed.value) {
      revealed.value = false
      offset.value = 0
    }
  }
  return { offset, dragging, start: (event: TouchEvent) => drag.start(event), tap }
}
