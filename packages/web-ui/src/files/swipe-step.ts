import { DRAG_THRESHOLD_PX } from './image-zoom'

/**
 * How a swipe steps the media viewer's picture on a phone: the decisions the
 * gesture's owner, `ImagePreview`, asks of it. A swipe left shows the next
 * picture and one right the previous, as in Photos; the picture follows the
 * finger and either settles on the next one or springs back; at the first or
 * the last it stops.
 */

/** Which neighbours the picture shown has in its transcript. */
export interface StepReach {
  previous: boolean
  next: boolean
}

/** A swipe of this share of the frame's width steps, however slowly it went. */
const STEP_SHARE = 0.3

/** A flick at least this fast, in px per millisecond, steps after a shorter swipe. */
const FLICK_SPEED = 0.5

/** A flick steps only after moving this far, so a twitch does not. */
const FLICK_MIN_PX = 24

/** How much of the finger's travel a picture follows past the first or last one: it gives, but only a little. */
const RESISTANCE = 0.25

/**
 * Whether a press starts a swipe: a finger or a pen, never the mouse, whose
 * drag moves a picture, on a picture at its fitted size. A zoomed picture is
 * moved by the drag instead.
 */
export function startsSwipe(pointerType: string, fitted: boolean): boolean {
  return pointerType !== 'mouse' && fitted
}

/**
 * The way a drag of (`dx`, `dy`) from the press goes, decided once it has
 * left the click's reach and kept to its end: a swipe is horizontal, and a
 * mostly vertical drag is none.
 */
export type SwipeAxis = 'click' | 'horizontal' | 'vertical'

export function swipeAxis(dx: number, dy: number): SwipeAxis {
  if (Math.hypot(dx, dy) < DRAG_THRESHOLD_PX)
    return 'click'
  return Math.abs(dx) > Math.abs(dy) ? 'horizontal' : 'vertical'
}

/** The step a drag of `dx` goes toward: left is the next picture, right the previous one. */
function stepToward(dx: number): -1 | 1 {
  return dx < 0 ? 1 : -1
}

/** Whether there is a picture in the direction `step`. */
function exists(step: -1 | 1, reach: StepReach): boolean {
  return step === 1 ? reach.next : reach.previous
}

/** How far the picture stands from the middle while the finger is `dx` from where it pressed. */
export function swipeOffset(dx: number, reach: StepReach): number {
  return exists(stepToward(dx), reach) ? dx : dx * RESISTANCE
}

/**
 * The step a finger lifted `dx` from where it pressed, `elapsedMs` after,
 * makes in a frame `width` wide: -1 the previous picture, 1 the next, 0 none,
 * where the picture springs back.
 */
export function swipeStep(dx: number, width: number, elapsedMs: number, reach: StepReach): -1 | 0 | 1 {
  const step = stepToward(dx)
  if (!exists(step, reach))
    return 0
  const distance = Math.abs(dx)
  const flick = distance >= FLICK_MIN_PX && distance / Math.max(elapsedMs, 1) >= FLICK_SPEED
  return distance >= width * STEP_SHARE || flick ? step : 0
}
