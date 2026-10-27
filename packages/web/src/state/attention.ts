import { ref, type Ref } from 'vue'
import { FRAME_INPUT, USER_ACTION_EVENTS } from '@demicodes/web-ui/ui/user-input'

/**
 * Whether the user attends the page: from their first action on it while it
 * is visible and focused, a click or a tap, a key or a wheel, until it is
 * hidden or loses the focus. The conversation the page shows is read only
 * while it is attended, so what arrived while the user was away keeps its
 * marks until they act. Coming back to the page is no action, nor is the
 * pointer's move over it on the way.
 *
 * A focus lost from within a frame of the page tells the page's window
 * nothing, so whoever reads it asks `attending`, which checks the page is
 * still in front.
 */
export const attended: Ref<boolean> = ref(false)

/** A notification was clicked while the page was not in front, which its worker brings it to next. */
let attendWhenInFront = false

/** Whether the page is visible and its window, or a frame within it, has the focus. */
function inFront(): boolean {
  return document.visibilityState === 'visible' && document.hasFocus()
}

/** Whether the user attends the page now; a page no longer in front stops being attended. */
export function attending(): boolean {
  if (attended.value && !inFront()) {
    attended.value = false
  }
  return attended.value
}

/** The user acted on the page, such as by changing the conversation it shows: it is attended while in front. */
export function attend(): void {
  if (inFront()) {
    attended.value = true
  }
}

/** The user clicked a notification, which acts on another surface: the page is attended once it comes in front. */
export function attendNotification(): void {
  if (inFront()) {
    attended.value = true
  } else {
    attendWhenInFront = true
  }
}

/** Follows the user's actions on the page and its leaving, until `signal` aborts. */
export function followAttention(signal: AbortSignal): void {
  const act = (): void => {
    if (inFront()) {
      attended.value = true
    }
  }
  const front = (): void => {
    if (attendWhenInFront && inFront()) {
      attendWhenInFront = false
      attended.value = true
    }
  }
  for (const action of USER_ACTION_EVENTS) {
    window.addEventListener(action, act, { capture: true, passive: true, signal })
  }
  window.addEventListener(FRAME_INPUT, (event) => {
    if ((USER_ACTION_EVENTS as readonly string[]).includes((event as CustomEvent<string>).detail)) {
      act()
    }
  }, { signal })
  window.addEventListener('focus', front, { signal })
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState !== 'visible') {
      attended.value = false
    }
    front()
  }, { signal })
  // The window blurs when the focus moves into a frame of the page too,
  // which leaves the page in front; the document knows once the move ends.
  window.addEventListener('blur', () => {
    setTimeout(() => {
      if (!inFront()) {
        attended.value = false
      }
    })
  }, { signal })
}
