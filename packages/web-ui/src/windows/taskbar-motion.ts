import { WINDOW_MOTION_MS } from './stage'

/**
 * A taskbar button's width changes with motion: one that comes grows from
 * nothing, one that goes shrinks to nothing, and one whose name or count
 * changes goes to its new width, so the buttons beside it slide rather than
 * jump. None moves where the user asks for reduced motion.
 */

const EASING = 'cubic-bezier(0.2, 0, 0, 1)'

/**
 * How long a menu that resting on a taskbar button shows stays after the
 * pointer leaves the button or the menu, in ms, so the pointer can cross the
 * gap between them.
 */
export const TASKBAR_LEAVE_MS = 200

function still(): boolean {
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches
}

/** Animates `element` between two widths, clipping what it holds meanwhile, then calls `done`. */
function widen(element: HTMLElement, from: Keyframe, to: Keyframe, done?: () => void): void {
  if (still()) {
    done?.()
    return
  }
  const overflow = element.style.overflow
  element.style.overflow = 'hidden'
  const motion = element.animate([from, to], { duration: WINDOW_MOTION_MS, easing: EASING })
  motion.onfinish = motion.oncancel = () => {
    element.style.overflow = overflow
    done?.()
  }
}

/** A button that comes (TransitionGroup's `enter`). */
export function grow(element: Element, done: () => void): void {
  const box = element as HTMLElement
  const width = box.getBoundingClientRect().width
  widen(box, { width: '0px', minWidth: '0px', paddingInline: '0px', opacity: 0 }, { width: `${width}px`, minWidth: '0px', opacity: 1 }, done)
}

/** A button that goes (TransitionGroup's `leave`). */
export function shrink(element: Element, done: () => void): void {
  const box = element as HTMLElement
  const width = box.getBoundingClientRect().width
  widen(box, { width: `${width}px`, minWidth: '0px', opacity: 1 }, { width: '0px', minWidth: '0px', paddingInline: '0px', opacity: 0 }, done)
}

/** A button whose width went from `from` px to its present one. */
export function resized(element: HTMLElement, from: number): void {
  const to = element.getBoundingClientRect().width
  if (Math.abs(to - from) >= 1) {
    widen(element, { width: `${from}px`, minWidth: '0px' }, { width: `${to}px`, minWidth: '0px' })
  }
}
