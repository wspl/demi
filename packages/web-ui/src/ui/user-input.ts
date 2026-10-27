/**
 * The user's input to the page, which tells that someone is at it: a key, a
 * click, a scroll or a pointer's move.
 *
 * A frame of the page's own, such as the text area of `FramedTextArea.vue`,
 * keeps its input to itself, so it tells the page's window of each with
 * `FRAME_INPUT`.
 */

/** The events of a window that are the user's input. */
export const USER_INPUT_EVENTS = ['keydown', 'pointerdown', 'pointermove', 'wheel', 'touchstart'] as const

/** What a frame of the page's own dispatches on the page's window at the user's input to it. */
export const FRAME_INPUT = 'demi-frame-input'

/** Tells `page` of each input to `frame`, a window of the page's own, until the answer is called. */
export function passInput(frame: Window, page: Window): () => void {
  const pass = (): void => {
    page.dispatchEvent(new Event(FRAME_INPUT))
  }
  for (const input of USER_INPUT_EVENTS) {
    frame.addEventListener(input, pass, { capture: true, passive: true })
  }
  return () => {
    for (const input of USER_INPUT_EVENTS) {
      frame.removeEventListener(input, pass, { capture: true })
    }
  }
}
