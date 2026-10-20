import { PHONE_QUERY, SCREEN_EDGE_PX } from './phone'

/**
 * What a phone's page needs, set up once by the app (`installPhonePage`).
 *
 * With the on-screen keyboard open, the app is exactly what the keyboard leaves
 * visible, so the composer sits right above it and the transcript's end stays in
 * view, at every width: an iPhone turned sideways is wider than the phone layout
 * (`phone.ts`) and has the same keyboard.
 *
 * - Chrome's `interactive-widget=resizes-content` does that itself. Safari on iOS
 *   ignores it, keeps the page its full height under the keyboard and scrolls the
 *   page to show the focused field.
 * - So the page's own scroll is held at 0, and the body, the app's frame
 *   (`base.css`), takes the visual viewport's height and top (`--app-height`,
 *   `--app-top`, and `data-keyboard` on the root, which drops the bottom safe area
 *   the keyboard covers). It follows the viewport as the keyboard opens, closes,
 *   changes its height or the phone turns.
 * - A page zoomed in, as a desktop's pinch zooms one, keeps its frame: what is
 *   visible is then a part of it.
 *
 * While the page is a phone's width, too:
 *
 * - The page never zooms. A pinch zooms nothing but what zooms itself, such as a
 *   picture (`ImagePreview`). `touch-action`, on every element since WebKit starts
 *   each scroller over (`base.css`), keeps it from the page everywhere but iOS,
 *   which ignores it for a pinch and is stopped through its own gesture events.
 * - The screen's edges are the app's. Safari on iOS, and the installed app, start
 *   their back and forward swipe from a touch within a few px of the screen's left
 *   or right edge, sliding the previous page in under the app's own controls.
 *   A touch that starts within `SCREEN_EDGE_PX` of either edge has its default
 *   prevented, which keeps that swipe from starting (since iOS 13.4). The app's
 *   own gestures there, such as the drawer's opening drag (SidebarLayout), still
 *   see every move.
 * - That touch scrolls nothing, and a prevented touch is no click either. So a tap
 *   there, one that lifts without moving and opened no menu, is given its click
 *   here, and a text field under it the focus, as the browser would have.
 * - Android's own Back, which no page can stop, is the history's
 *   (`overlay/back-close.ts`). Chrome's overscroll navigation is held off by the
 *   page's `overscroll-behavior: none` (`base.css`).
 *
 * At every width, the viewport tag stays as the page shipped it. Its
 * `maximum-scale=1` is what keeps Safari on iOS from zooming in on a field it
 * focuses whose text is under 16 px, such as one an extension adds, and a page
 * Safari zoomed stays zoomed, since nothing a page does sets the scale back. So a
 * script that rewrites the tag, removes it or adds a viewport tag of its own after
 * it, as an extension may, finds the page's tag back as it was, and last.
 */

/** A finger that moves farther than this drags, and is no tap. */
const TAP_SLOP_PX = 10
/** The visible part this much shorter than the page means the keyboard is open, not a toolbar that moved. */
const KEYBOARD_MIN_PX = 80
/** A scale this close to 1 is a page not zoomed in; a pinch on a desktop's page scales it. */
const UNZOOMED = 0.01

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

  const keepViewport = keepViewportTag()

  const viewport = window.visualViewport
  const fit = (): void => {
    if (!viewport)
      return
    const keyboard = Math.abs(viewport.scale - 1) < UNZOOMED && root.clientHeight - viewport.height > KEYBOARD_MIN_PX
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
  // Safari scrolls the page to show a focused field, though nothing in it
  // overflows; the app stands at the visual viewport instead.
  window.addEventListener('scroll', () => {
    if (window.scrollX !== 0 || window.scrollY !== 0)
      window.scrollTo(0, 0)
  }, { passive: true, signal })

  /** A touch that started at a screen edge, whose default was prevented, while it may still be a tap. */
  let edgeTap: { x: number; y: number; target: Element } | null = null
  document.addEventListener('touchstart', (event) => {
    edgeTap = null
    // The finger that just came down, which with another already on the screen is not the first.
    const finger = event.changedTouches[0]
    if (!phone.matches || !finger || !event.cancelable)
      return
    if (finger.clientX > SCREEN_EDGE_PX && finger.clientX < window.innerWidth - SCREEN_EDGE_PX)
      return
    event.preventDefault()
    // A second finger makes it a pinch or a gesture of the page's, never a tap.
    if (event.touches.length === 1 && event.target instanceof Element)
      edgeTap = { x: finger.clientX, y: finger.clientY, target: event.target }
  }, { passive: false, signal })

  document.addEventListener('touchmove', (event) => {
    const finger = event.touches[0]
    if (edgeTap && (event.touches.length !== 1 || !finger || Math.hypot(finger.clientX - edgeTap.x, finger.clientY - edgeTap.y) > TAP_SLOP_PX))
      edgeTap = null
  }, { passive: true, signal })
  document.addEventListener('touchend', (event) => {
    const tap = edgeTap
    edgeTap = null
    // A release whose default is prevented, by a long press's menu (`context-menu.ts`) or by what was touched, makes no click.
    if (tap && !event.defaultPrevented && event.touches.length === 0)
      giveTap(tap)
  }, { passive: false, signal })
  document.addEventListener('touchcancel', () => {
    edgeTap = null
  }, { signal })

  return () => {
    listeners.abort()
    keepViewport?.disconnect()
  }
}

/** What takes the focus from a tap, as the browser gives it to a field. */
const TAP_FOCUSES = 'input:not([type=checkbox]):not([type=radio]):not([type=button]):not([type=submit]), textarea, [contenteditable]:not([contenteditable=false])'

/**
 * The click a tap at a screen edge would have made, which preventing its
 * touch took away: on what the finger touched, where it touched, with the
 * focus first in the text field it touched, as the browser orders them.
 */
function giveTap(tap: { x: number; y: number; target: Element }): void {
  if (!tap.target.isConnected)
    return
  const field = tap.target.closest<HTMLElement>(TAP_FOCUSES)
  if (field && document.activeElement !== field)
    field.focus()
  tap.target.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, composed: true, view: window, detail: 1, clientX: tap.x, clientY: tap.y }))
}

/**
 * Holds the viewport tag the page shipped with, read as the app starts, before any
 * extension's script runs. It puts its words back when they change, and the tag back
 * as the last viewport tag of the head, which is the one a browser follows, when it
 * leaves or another follows it.
 *
 * Null on a page without one.
 */
function keepViewportTag(): MutationObserver | null {
  const tag = document.querySelector<HTMLMetaElement>('meta[name="viewport"]')
  const head = document.head
  if (!tag || !head)
    return null
  const shipped = tag.content
  const restore = (): void => {
    if (tag.name !== 'viewport')
      tag.name = 'viewport'
    if (tag.content !== shipped)
      tag.content = shipped
    const tags = head.querySelectorAll('meta[name="viewport"]')
    if (tags[tags.length - 1] !== tag)
      head.append(tag)
  }
  // What it puts back changes the head once more, and then finds all as it was.
  const observer = new MutationObserver(restore)
  observer.observe(head, { childList: true, subtree: true, attributes: true, attributeFilter: ['content', 'name'] })
  return observer
}
