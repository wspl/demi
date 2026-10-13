/**
 * Moves that change no layout while they play (FLIP: first, last, invert,
 * play). The caller reads where an element stands, applies the new layout
 * at once, and the element glides from where it stood to where it now is,
 * through its transform alone; what it holds may unroll or roll up through
 * its clip. A long, virtualized page beside it lays out once, when the
 * layout changes, never per frame.
 */

/** Where `element` is drawn now, its transform included: the start of a glide. */
export function drawnTop(element: Element): number {
  return element.getBoundingClientRect().top
}

/**
 * Glides `element` from the top it was drawn at, `from`, to where its layout
 * now puts it; null when it did not move.
 */
export function glideFrom(element: Element, from: number, options: KeyframeAnimationOptions): Animation | null {
  const offset = from - element.getBoundingClientRect().top
  if (Math.abs(offset) < 0.5) {
    return null
  }
  return element.animate([{ transform: `translateY(${offset}px)` }, { transform: 'none' }], options)
}

/**
 * A clip that shows all of an element, or none of it from its bottom up,
 * with room around it for its outline, which is drawn outside its box.
 */
export const SHOWN_CLIP = 'inset(-2px -2px -2px -2px)'
export const ROLLED_UP_CLIP = 'inset(-2px -2px 100% -2px)'

/**
 * Unrolls `element` downward from `from`, a clip it was drawn with, or rolls
 * it up when `to` is `ROLLED_UP_CLIP`.
 */
export function unroll(element: Element, from: string, to: string, options: KeyframeAnimationOptions): Animation {
  return element.animate([{ clipPath: from }, { clipPath: to }], options)
}
