import { dragCloses, dragVelocity } from './drag-close'

/**
 * A finger dragging a side pane sideways on a phone, the one owner of how
 * such a drag goes: the sidebar's drawer swiped closed, drawn in from the
 * screen's left edge, and the work panel swiped back from that edge
 * (SidebarLayout).
 *
 * The finger's first clear move decides what the touch is: one mostly the
 * drag's way takes the pane, which then follows the finger from where it
 * first touched; any other leaves the touch to scroll what is under it. At
 * the lift the pane settles by the sheets' and drawers' rule (`dragCloses`):
 * past 30% of its size, or with a flick still moving, it goes on the way it
 * was dragged; a shorter pull, or one held still before the lift, springs
 * back. A second finger or the system taking the touch springs it back too:
 * only a lift lets it go. A lift that ends a drag clicks nothing.
 *
 * Its moves are heard on what the finger touched, where a touch's events go
 * however far it moves and even once that element has left the page, and
 * only while it lasts: a listener that may stop a move from scrolling makes
 * every scroll wait for it.
 */
export interface FingerDrag {
  /** A finger down on the pane, or at the edge it is drawn from: the drag follows it until its lift. */
  start(event: TouchEvent): void
  /** The touch is lost, as to a second finger: a pane the drag holds springs back. */
  cancel(): void
}

export interface FingerDragOptions {
  /** The way the drag moves the pane: right (1) or left (-1). */
  way: 1 | -1
  /** The pane's size along the drag, in px, which its share is measured against. */
  size(): number
  /** The drag takes the pane: it follows the finger, with no motion of its own, from here on. */
  hold(): void
  /** The finger has moved the pane `moved` px its way from where it first touched. */
  follow(moved: number): void
  /** The finger let go of a pane it held: it goes on its way (true) or springs back (false). */
  settle(goes: boolean): void
}

/** A finger that moves this far settles whether it drags the pane or scrolls what is under it. */
const SLOP_PX = 8

/** Keeps the pane's drag; `dispose` releases the listeners of a touch still under way. */
export function createFingerDrag(options: FingerDragOptions): FingerDrag & { dispose(): void } {
  /** The touch under way: where it first touched, whether it holds the pane, and its recent samples of how far it moved it. */
  let touch: { startX: number; startY: number; holding: boolean; samples: { at: number; t: number }[] } | null = null
  /** The listeners of the touch under way, on what it touched, from its first touch to its lift. */
  let listening: AbortController | null = null

  function release(): void {
    listening?.abort()
    listening = null
  }

  /** Ends the touch: a pane it held springs back. */
  function cancel(): void {
    const held = touch?.holding === true
    touch = null
    release()
    if (held)
      options.settle(false)
  }

  function move(event: TouchEvent): void {
    const finger = event.touches[0]
    if (!touch || !finger)
      return
    // `touches` lists every finger on the screen: one put down anywhere else takes the gesture too.
    if (event.touches.length > 1) {
      cancel()
      return
    }
    const dx = finger.clientX - touch.startX
    const dy = finger.clientY - touch.startY
    const along = dx * options.way
    if (!touch.holding) {
      if (Math.hypot(dx, dy) < SLOP_PX)
        return
      // A move mostly the drag's way takes the pane; any other scrolls what is under the finger.
      if (!(along > 0 && Math.abs(dx) > Math.abs(dy))) {
        touch = null
        release()
        return
      }
      touch.holding = true
      options.hold()
    }
    if (event.cancelable)
      event.preventDefault()
    // The pane follows the finger from where it first touched, the slop included.
    const moved = Math.max(0, along)
    touch.samples = [...touch.samples.filter((sample) => event.timeStamp - sample.t < 120), { at: moved, t: event.timeStamp }]
    options.follow(moved)
  }

  function end(event: TouchEvent): void {
    const ended = touch
    // A finger still on the screen, wherever it went down, means the lift was not the gesture's end.
    if (!ended?.holding || event.touches.length > 0) {
      cancel()
      return
    }
    touch = null
    release()
    // The lift of a drag is no tap: nothing under the finger is clicked, by the browser or by the phone's page (`phone-page.ts`).
    if (event.cancelable)
      event.preventDefault()
    const moved = ended.samples.at(-1)?.at ?? 0
    options.settle(dragCloses(moved, options.size(), dragVelocity(ended.samples, event.timeStamp)))
  }

  function start(event: TouchEvent): void {
    const finger = event.touches[0]
    const target = event.target
    // A second finger takes the gesture from the first: the pane goes back to its place.
    if (event.touches.length !== 1 || !finger || !(target instanceof Element)) {
      cancel()
      return
    }
    release()
    // The first touch is the pull's first sample, so a flick of a single move has a speed.
    touch = { startX: finger.clientX, startY: finger.clientY, holding: false, samples: [{ at: 0, t: event.timeStamp }] }
    listening = new AbortController()
    const { signal } = listening
    // An Element's own event map names no touch events: these are, as their names say.
    target.addEventListener('touchmove', (next) => {
      if (next instanceof TouchEvent)
        move(next)
    }, { passive: false, signal })
    target.addEventListener('touchend', (next) => {
      if (next instanceof TouchEvent)
        end(next)
    }, { signal })
    target.addEventListener('touchcancel', cancel, { signal })
  }

  return { start, cancel, dispose: release }
}
