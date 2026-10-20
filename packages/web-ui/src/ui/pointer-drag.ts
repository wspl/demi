/** What a drag reports to the control that started it. */
export interface PointerDragHandlers {
  /** The cursor the whole page wears while the drag lasts, such as `col-resize`. */
  cursor: string
  /** How far the pointer has moved from the press, in px; at most once a frame. */
  move: (dx: number, dy: number) => void
  /**
   * The drag ended. `cancelled` is true when Escape asked to put things back
   * where they were at the press; otherwise the last move has been reported.
   */
  end: (cancelled: boolean) => void
}

/** The class the document wears while a drag lasts (`styles/base.css`). */
const DRAGGING_CLASS = 'is-dragging'

/**
 * Follows the pointer from a press on `event.currentTarget` until it is released,
 * as a divider, a window's title bar or its edge is dragged.
 *
 * - The element captures the pointer, so the drag survives leaving it, the page's
 *   window and iframes.
 * - While it lasts, the document wears the drag's cursor everywhere and selects no
 *   text.
 * - Moves are coalesced to one report a frame, and the last one is reported on
 *   release, so what follows ends exactly under the pointer.
 * - Release, cancel, lost capture and the window losing focus all end the drag the
 *   same way. Escape ends it as cancelled, and no other Escape handler acts on
 *   that key.
 * - Every listener, the frame and the capture are released when it ends.
 *
 * Returns what ends the drag early, as a release does: the control calls it when
 * it unmounts mid-drag.
 */
export function startPointerDrag(event: PointerEvent, handlers: PointerDragHandlers): () => void {
  const target = event.currentTarget
  if (!(target instanceof HTMLElement))
    throw new Error('A pointer drag starts from a listener on an HTML element')
  const element: HTMLElement = target
  const { pointerId } = event
  const startX = event.clientX
  const startY = event.clientY
  let pending: { dx: number; dy: number } | null = null
  let frame: number | null = null
  let ended = false

  function flush(): void {
    frame = null
    if (pending) {
      const { dx, dy } = pending
      pending = null
      handlers.move(dx, dy)
    }
  }

  function onMove(move: PointerEvent): void {
    if (move.pointerId !== pointerId)
      return
    pending = { dx: move.clientX - startX, dy: move.clientY - startY }
    frame ??= requestAnimationFrame(flush)
  }

  function onEnd(end: PointerEvent): void {
    if (end.pointerId === pointerId)
      finish(false)
  }

  function onBlur(): void {
    finish(false)
  }

  function onKeydown(key: KeyboardEvent): void {
    if (key.key !== 'Escape')
      return
    key.preventDefault()
    key.stopPropagation()
    finish(true)
  }

  function finish(cancelled: boolean): void {
    if (ended)
      return
    ended = true
    if (frame !== null)
      cancelAnimationFrame(frame)
    frame = null
    element.removeEventListener('pointermove', onMove)
    element.removeEventListener('pointerup', onEnd)
    element.removeEventListener('pointercancel', onEnd)
    element.removeEventListener('lostpointercapture', onEnd)
    window.removeEventListener('blur', onBlur)
    window.removeEventListener('keydown', onKeydown, true)
    const root = document.documentElement
    root.classList.remove(DRAGGING_CLASS)
    root.style.removeProperty('--drag-cursor')
    if (element.hasPointerCapture(pointerId))
      element.releasePointerCapture(pointerId)
    if (cancelled)
      pending = null
    else
      flush()
    handlers.end(cancelled)
  }

  element.setPointerCapture(pointerId)
  element.addEventListener('pointermove', onMove)
  element.addEventListener('pointerup', onEnd)
  element.addEventListener('pointercancel', onEnd)
  element.addEventListener('lostpointercapture', onEnd)
  window.addEventListener('blur', onBlur)
  window.addEventListener('keydown', onKeydown, true)
  const root = document.documentElement
  root.style.setProperty('--drag-cursor', handlers.cursor)
  root.classList.add(DRAGGING_CLASS)
  return () => finish(false)
}
