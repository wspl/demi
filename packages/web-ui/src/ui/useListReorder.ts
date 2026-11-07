import { onBeforeUnmount, ref, type Ref } from 'vue'
import { DRAG_START_PX, dragShift, dragTarget, moved } from './drag-order'

/**
 * Drag to reorder the rows of a list, as a macOS list's rows are, down a
 * vertical list (`axis` y) or along a row of buttons, as a taskbar's (x).
 *
 * - A row pressed on its handle follows the pointer once it moves a few pixels.
 * - The other rows step aside to show where it will land, and letting go puts it
 *   there. The click that letting go makes is not the row's.
 * - Escape, or the window losing focus, puts it back.
 * - The handle also moves its row one place with the Up and Down arrow keys, or
 *   Left and Right along a row.
 *
 * The rows are the elements of `container` marked `data-reorder-id`, in order.
 * `commit` receives the ids in their new order, and the host's list shows it with
 * its next render.
 */
export function useListReorder(
  container: Readonly<Ref<HTMLElement | undefined | null>>,
  ids: () => readonly string[],
  commit: (ids: string[]) => void,
  axis: 'x' | 'y' = 'y',
) {
  const across = axis === 'x'
  /** The row being dragged, once it moved far enough to drag. */
  const dragging = ref<string | null>(null)
  let held: {
    pointerId: number
    at: number
    from: number
    to: number
    rows: HTMLElement[]
    centers: number[]
    step: number
  } | null = null

  function rows(): HTMLElement[] {
    return Array.from(container.value?.querySelectorAll<HTMLElement>('[data-reorder-id]') ?? [])
  }

  function settle() {
    for (const row of held?.rows ?? []) {
      row.style.transform = ''
      row.style.zIndex = ''
      row.style.position = ''
    }
    held = null
    dragging.value = null
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', finish)
    window.removeEventListener('pointercancel', settle)
    window.removeEventListener('keydown', escape, true)
    window.removeEventListener('blur', settle)
  }

  function move(event: PointerEvent) {
    if (!held || event.pointerId !== held.pointerId) {
      return
    }
    const offset = (across ? event.clientX : event.clientY) - held.at
    if (dragging.value === null) {
      if (Math.abs(offset) < DRAG_START_PX) {
        return
      }
      dragging.value = held.rows[held.from]?.dataset['reorderId'] ?? null
    }
    event.preventDefault()
    held.to = dragTarget(held.centers, held.from, held.centers[held.from]! + offset)
    held.rows.forEach((row, index) => {
      if (index === held!.from) {
        row.style.position = 'relative'
        row.style.zIndex = '1'
        row.style.transform = `${across ? 'translateX' : 'translateY'}(${offset}px)`
      } else {
        row.style.transform = `${across ? 'translateX' : 'translateY'}(${dragShift(index, held!.from, held!.to) * held!.step}px)`
      }
    })
  }

  function finish(event: PointerEvent) {
    if (!held || event.pointerId !== held.pointerId) {
      return
    }
    const { from, to } = held
    const dragged = dragging.value !== null
    settle()
    if (dragged) {
      swallowClick()
    }
    if (dragged && from !== to) {
      commit(moved(ids(), from, to))
    }
  }

  /** Takes the click that letting go of a dragged row makes, before the row sees it. */
  function swallowClick() {
    const swallow = (event: MouseEvent) => {
      event.stopPropagation()
      event.preventDefault()
    }
    window.addEventListener('click', swallow, { capture: true, once: true })
    // A release off the row makes no click; the next one is the user's.
    setTimeout(() => window.removeEventListener('click', swallow, { capture: true }))
  }

  function escape(event: KeyboardEvent) {
    if (event.key !== 'Escape') {
      return
    }
    event.preventDefault()
    event.stopPropagation()
    settle()
  }

  /** The handle of row `id` was pressed. */
  function start(event: PointerEvent, id: string) {
    if (event.button !== 0) {
      return
    }
    settle()
    const listed = rows()
    const from = listed.findIndex((row) => row.dataset['reorderId'] === id)
    if (from < 0 || listed.length < 2) {
      return
    }
    const rects = listed.map((row) => row.getBoundingClientRect())
    held = {
      pointerId: event.pointerId,
      at: across ? event.clientX : event.clientY,
      from,
      to: from,
      rows: listed,
      centers: rects.map((rect) => (across ? rect.left + rect.width / 2 : rect.top + rect.height / 2)),
      // The others step aside by the dragged row's own length and the gap
      // between rows, whatever their own lengths.
      step: across
        ? rects[from]!.width + Math.max(0, rects[1]!.left - rects[0]!.right)
        : rects[from]!.height + Math.max(0, rects[1]!.top - rects[0]!.bottom),
    }
    event.preventDefault()
    window.addEventListener('pointermove', move, { passive: false })
    window.addEventListener('pointerup', finish)
    window.addEventListener('pointercancel', settle)
    window.addEventListener('keydown', escape, true)
    window.addEventListener('blur', settle)
  }

  /** A key on the handle of row `id`: Up and Down, or Left and Right along a row, move it one place. */
  function keydown(event: KeyboardEvent, id: string) {
    const [back, on] = across ? ['ArrowLeft', 'ArrowRight'] : ['ArrowUp', 'ArrowDown']
    const step = event.key === back ? -1 : event.key === on ? 1 : 0
    if (step === 0) {
      return
    }
    event.preventDefault()
    const order = ids()
    const from = order.indexOf(id)
    const to = from + step
    if (from < 0 || to < 0 || to >= order.length) {
      return
    }
    commit(moved(order, from, to))
  }

  onBeforeUnmount(settle)
  return { dragging, start, keydown }
}
