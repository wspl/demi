import { PHONE_QUERY } from './phone'

/**
 * What the phone layout's page needs (`phone.ts`), set up once by the app
 * (`installPhonePage`), in effect while the page is a phone's width:
 *
 * - The page never zooms. A pinch zooms nothing but what zooms itself, such
 *   as a picture (`ImagePreview`): `touch-action` keeps it from the page
 *   everywhere but iOS, which ignores it for a pinch and is stopped through
 *   its own gesture events.
 * - With the on-screen keyboard open, the app is exactly what the keyboard
 *   leaves visible, so the composer sits right above it. Chrome's
 *   `interactive-widget=resizes-content` does that itself; iOS keeps the page
 *   its size and moves the visible part over it, which the app follows
 *   (`--app-height`, `--app-top`, and `data-keyboard` on the root, which
 *   drops the bottom safe area the keyboard covers).
 * - A long press opens what a right-click opens where the page marks it
 *   (`data-touch-menu`), as iOS gives no `contextmenu` for one; Android
 *   gives its own, and then none is added.
 */

/** A press held this long, without moving, is a long press. */
const LONG_PRESS_MS = 550
/** A finger that moves farther than this scrolls or drags, and is no long press. */
const LONG_PRESS_SLOP_PX = 10
/** The visible part this much shorter than the page means the keyboard is open, not a toolbar that moved. */
const KEYBOARD_MIN_PX = 80

export function installPhonePage(): () => void {
  const phone = window.matchMedia(PHONE_QUERY)
  const root = document.documentElement
  const listeners = new AbortController()
  const { signal } = listeners

  // iOS's pinch, which `touch-action` does not reach.
  const stopGesture = (event: Event): void => {
    if (phone.matches)
      event.preventDefault()
  }
  document.addEventListener('gesturestart', stopGesture, { passive: false, signal })
  document.addEventListener('gesturechange', stopGesture, { passive: false, signal })

  const viewport = window.visualViewport
  const fit = (): void => {
    if (!viewport)
      return
    const keyboard = phone.matches && root.clientHeight - viewport.height > KEYBOARD_MIN_PX
    if (keyboard) {
      root.style.setProperty('--app-height', `${viewport.height}px`)
      root.style.setProperty('--app-top', `${viewport.offsetTop}px`)
      root.dataset.keyboard = ''
    } else {
      root.style.removeProperty('--app-height')
      root.style.removeProperty('--app-top')
      delete root.dataset.keyboard
    }
  }
  viewport?.addEventListener('resize', fit, { signal })
  viewport?.addEventListener('scroll', fit, { signal })

  let press: { timer: number; x: number; y: number; target: Element; opened: boolean } | null = null
  const endPress = (): void => {
    if (press)
      window.clearTimeout(press.timer)
    press = null
  }
  document.addEventListener('touchstart', (event) => {
    endPress()
    const finger = event.touches[0]
    const target = event.target instanceof Element ? event.target : null
    if (!phone.matches || event.touches.length !== 1 || !finger || !target?.closest('[data-touch-menu]'))
      return
    const x = finger.clientX
    const y = finger.clientY
    press = {
      x,
      y,
      target,
      opened: false,
      timer: window.setTimeout(() => {
        if (!press)
          return
        press.opened = true
        target.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 2 }))
      }, LONG_PRESS_MS),
    }
  }, { passive: true, signal })
  document.addEventListener('touchmove', (event) => {
    const finger = event.touches[0]
    if (press && !press.opened && finger && Math.hypot(finger.clientX - press.x, finger.clientY - press.y) > LONG_PRESS_SLOP_PX)
      endPress()
  }, { passive: true, signal })
  document.addEventListener('touchend', (event) => {
    // The press opened a menu: its release is no tap on what lies under it.
    if (press?.opened && event.cancelable)
      event.preventDefault()
    endPress()
  }, { passive: false, signal })
  document.addEventListener('touchcancel', endPress, { signal })
  // Android's own long press opens the menu: none is added.
  document.addEventListener('contextmenu', (event) => {
    if (press && !press.opened && event.isTrusted)
      endPress()
  }, { capture: true, signal })

  return () => {
    listeners.abort()
    endPress()
  }
}
