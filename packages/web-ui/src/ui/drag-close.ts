/**
 * How a drag on a bottom sheet (`Sheet.vue`), the sidebar's drawer or the
 * full-screen work panel (`finger-drag.ts`) ends, as iOS's and Android's
 * end theirs: a flick toward closing closes it however short the pull, a
 * flick back keeps it, and
 * otherwise it closes once pulled past 30% of its size and springs back
 * short of that.
 */

/** The share of its size a slow pull must pass to close it. */
export const DRAG_CLOSE_SHARE = 0.3

/** A flick: the finger leaves at this speed or faster, in px per ms (500 px/s). */
export const DRAG_FLICK_SPEED = 0.5

/**
 * Whether a drag that let go `pulled` px from the resting place toward
 * closing, moving at `velocity` px/ms (positive toward closing), closes a
 * sheet or a drawer `size` px deep.
 */
export function dragCloses(pulled: number, size: number, velocity: number): boolean {
  if (pulled <= 0)
    return false
  if (velocity >= DRAG_FLICK_SPEED)
    return true
  if (velocity <= -DRAG_FLICK_SPEED)
    return false
  return pulled >= size * DRAG_CLOSE_SHARE
}

/**
 * The speed of a drag along one axis as the finger lifts at `liftAt`, from
 * its recent samples (`at`, the newest last), in px per ms. It is the speed
 * over the last 80 ms before the lift, the time the finger stood still
 * before it included: a flick held still before the lift has stopped.
 */
export function dragVelocity(samples: readonly { at: number; t: number }[], liftAt: number): number {
  const last = samples.at(-1)
  const recent = samples.findIndex((sample) => liftAt - sample.t <= 80)
  if (!last || recent === -1 || samples.length < 2)
    return 0
  // With one sample in the window, the one before it says where the finger came from.
  const first = samples[Math.min(recent, samples.length - 2)]!
  if (liftAt <= first.t)
    return 0
  return (last.at - first.at) / (liftAt - first.t)
}
