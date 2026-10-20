import { PHONE_QUERY } from '../ui/phone'

/**
 * The browser's own bars take the color of the page's top, so the top of the
 * screen reads as one.
 *
 * - Safari on iOS paints the status bar's band with the page's `theme-color`, in
 *   a tab and in the app installed on the Home Screen, and desktop browsers tint
 *   their tab bar with it.
 * - On a phone the top of every screen is the conversation's bar, on the
 *   surface. Elsewhere it is the sidebar's, on the base surface.
 *
 * The page's HTML names the default tone's colors for the moment before the app
 * runs. From then on this keeps one `theme-color` tag on the surface the theme,
 * the tone and the width now give, as each changes: the theme and the tone are
 * announced on the root's `data-theme` and `data-tone`, and the width by the
 * phone switch.
 */
export function followThemeColor(): () => void {
  return followAppearance(() => {
    const color = pageTopColor()
    if (!color)
      return
    const [tag, ...others] = document.head.querySelectorAll<HTMLMetaElement>('meta[name="theme-color"]')
    // The HTML's tags choose by width and scheme; one tag, without a media query, now says the page's own.
    for (const other of others)
      other.remove()
    const shown = tag ?? document.head.appendChild(Object.assign(document.createElement('meta'), { name: 'theme-color' }))
    shown.removeAttribute('media')
    if (shown.content !== color)
      shown.content = color
  })
}

/**
 * The color of the page's top, which the browser's bars take: on a phone the
 * conversation's surface, elsewhere the sidebar's base surface; empty before
 * the stylesheet names it.
 */
export function pageTopColor(): string {
  const phone = window.matchMedia(PHONE_QUERY).matches
  return getComputedStyle(document.documentElement).getPropertyValue(phone ? '--surface' : '--surface-base').trim()
}

/**
 * Calls `apply` now and each time what the page's colors follow changes: the
 * theme and the tone, which the root's `data-theme` and `data-tone` announce,
 * and the phone switch. Returns the release of its listeners.
 */
export function followAppearance(apply: () => void): () => void {
  const root = document.documentElement
  const phone = window.matchMedia(PHONE_QUERY)
  const observer = new MutationObserver(apply)
  observer.observe(root, { attributes: true, attributeFilter: ['data-theme', 'data-tone'] })
  phone.addEventListener('change', apply)
  apply()
  return () => {
    observer.disconnect()
    phone.removeEventListener('change', apply)
  }
}
