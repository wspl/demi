import { onBeforeUnmount, ref, type Ref } from 'vue'
import { DRAG_START_PX, dragShift, dragTarget, moved } from './drag-order'

/**
 * Drag to reorder the rows of a vertical list, as a macOS list's rows are.
 *
 * - A row pressed on its handle follows the pointer once it moves a few pixels.
 * - The other rows step aside to show where it will land, and letting go puts it
 *   there.
 * - Escape, or the window losing focus, puts it back.
 * - The handle also moves its row one place with the Up and Down arrow keys.
 *
 * The rows are the elements of `container` marked `data-reorder-id`, in order.
 * `commit` receives the ids in their new order, and the host's list shows it with
 * its next render.
 */
export function useListReorder(
  container: Readonly<Ref<HTMLElement | undefined>>,
  ids: () => readonly string[],
  commit: (ids: string[]) => void,
) {
  /** The row being dragged, once it moved far enough to drag. */
  const dragging = ref<string | null>(null)
  let held: {
    pointerId: number
    y: number
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
    const offset = event.clientY - held.y
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
        row.style.transform = `translateY(${offset}px)`
      } else {
        row.style.transform = `translateY(${dragShift(index, held!.from, held!.to) * held!.step}px)`
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
    if (dragged && from !== to) {
      commit(moved(ids(), from, to))
    }
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
      y: event.clientY,
      from,
      to: from,
      rows: listed,
      centers: rects.map((rect) => rect.top + rect.height / 2),
      // The others step aside by the dragged row's own height and the gap
      // between rows, whatever their own heights.
      step: rects[from]!.height + (rects.length > 1 ? Math.max(0, rects[1]!.top - rects[0]!.bottom) : 0),
    }
    event.preventDefault()
    window.addEventListener('pointermove', move, { passive: false })
    window.addEventListener('pointerup', finish)
    window.addEventListener('pointercancel', settle)
    window.addEventListener('keydown', escape, true)
    window.addEventListener('blur', settle)
  }

  /** A key on the handle of row `id`: Up and Down move it one place. */
  function keydown(event: KeyboardEvent, id: string) {
    const step = event.key === 'ArrowUp' ? -1 : event.key === 'ArrowDown' ? 1 : 0
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
