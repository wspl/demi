/**
 * A device drawn as a wireframe sketch around its screen (`mobile.md`
 * § The live view): the frame a view draws itself when the device's own is
 * not installed. The outline follows the screen at a bezel's distance with
 * the screen's corners, and each button is a short mark outside the edge it
 * stands on. Sizes are CSS pixels of the view.
 */

/** The bezel between the screen and the outline. */
export const OUTLINE_BEZEL = 10
/** How far a button's mark stands off the outline, and how thick it is. */
const MARK_GAP = 1
export const OUTLINE_MARK = 3
/** The room the outline takes around the screen, marks included. */
export const OUTLINE_ROOM = OUTLINE_BEZEL + MARK_GAP + OUTLINE_MARK

export interface OutlineBox {
  left: number
  top: number
  width: number
  height: number
}

/** A button on an edge of the device, as fractions of the edge's length from its top or its left. */
export interface OutlineButton {
  side: 'left' | 'right' | 'top' | 'bottom'
  start: number
  length: number
}

export interface OutlineShape {
  body: OutlineBox & { radius: number }
  marks: OutlineBox[]
}

/** The outline around `screen`, whose corners have `cornerRadius`, with `buttons` marked. */
export function deviceOutline(screen: OutlineBox, cornerRadius: number, buttons: readonly OutlineButton[]): OutlineShape {
  const body = {
    left: screen.left - OUTLINE_BEZEL,
    top: screen.top - OUTLINE_BEZEL,
    width: screen.width + 2 * OUTLINE_BEZEL,
    height: screen.height + 2 * OUTLINE_BEZEL,
    radius: cornerRadius > 0 ? cornerRadius + OUTLINE_BEZEL : 0,
  }
  const marks = buttons.map((button): OutlineBox => {
    const along = button.side === 'left' || button.side === 'right' ? body.height : body.width
    const start = button.start * along
    const length = Math.max(OUTLINE_MARK, button.length * along)
    switch (button.side) {
      case 'left':
        return { left: body.left - MARK_GAP - OUTLINE_MARK, top: body.top + start, width: OUTLINE_MARK, height: length }
      case 'right':
        return { left: body.left + body.width + MARK_GAP, top: body.top + start, width: OUTLINE_MARK, height: length }
      case 'top':
        return { left: body.left + start, top: body.top - MARK_GAP - OUTLINE_MARK, width: length, height: OUTLINE_MARK }
      case 'bottom':
        return { left: body.left + start, top: body.top + body.height + MARK_GAP, width: length, height: OUTLINE_MARK }
    }
  })
  return { body, marks }
}

/** How long an element's size must hold before it counts as laid out, so a drag asks once. */
const SETTLE_MS = 150

/**
 * Tells `settled` the element's size once a change of it has held for a
 * moment, not every pixel of a drag; answers what stops watching.
 */
export function observeSettledSize(element: Element, settled: (size: { width: number; height: number }) => void): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null
  const observer = new ResizeObserver(([entry]) => {
    if (!entry) {
      return
    }
    const size = { width: entry.contentRect.width, height: entry.contentRect.height }
    if (timer !== null) {
      clearTimeout(timer)
    }
    timer = setTimeout(() => {
      timer = null
      settled(size)
    }, SETTLE_MS)
  })
  observer.observe(element)
  return () => {
    observer.disconnect()
    if (timer !== null) {
      clearTimeout(timer)
    }
  }
}
