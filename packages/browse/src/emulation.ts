// What `emulate` sets (browse.md § Conditions), as the DevTools protocol
// applies it to a page as it is, without reloading it: the screen's size,
// pixel ratio and touch, a phone's user agent, the theme, the locale and the
// time zone. The daemon's own session with each page holds these overrides,
// which the browser drops when that session ends; a daemon that attaches to
// a page applies them again.
import { devices, type CDPSession } from 'playwright'
import type { Emulation } from './state'

/** The viewport and pixel ratio of a browser no `emulate` changed. */
const DEFAULT_VIEWPORT = { width: 1440, height: 900 }
const DEFAULT_SCALE = 2

/** The screen `emulation` gives the page. */
export interface Screen {
  width: number
  height: number
  scale: number
  /** Whether the page is laid out as on a phone, with a fixed layout viewport. */
  mobile: boolean
  touch: boolean
  /** A phone's user agent; null keeps the browser's own. */
  userAgent: string | null
}

/** The screen `emulation` asks for, a named device's first; a device Playwright does not know fails. */
export function screenOf(emulation: Emulation): Screen {
  const device = emulation.device === undefined ? undefined : devices[emulation.device]
  if (emulation.device !== undefined && !device) {
    throw new Error(`Playwright knows no device ${emulation.device}`)
  }
  return {
    width: emulation.width ?? device?.viewport.width ?? DEFAULT_VIEWPORT.width,
    height: emulation.height ?? device?.viewport.height ?? DEFAULT_VIEWPORT.height,
    scale: emulation.scale ?? device?.deviceScaleFactor ?? DEFAULT_SCALE,
    mobile: device?.isMobile ?? false,
    touch: device?.hasTouch ?? false,
    userAgent: device?.userAgent ?? null,
  }
}

/**
 * Applies `emulation` to the page of `session`; `nativeUserAgent` is the
 * browser's own, which a page that no longer emulates a phone gets back.
 */
export async function applyEmulation(session: CDPSession, emulation: Emulation, nativeUserAgent: string): Promise<void> {
  const screen = screenOf(emulation)
  await session.send('Emulation.setDeviceMetricsOverride', {
    width: screen.width,
    height: screen.height,
    deviceScaleFactor: screen.scale,
    mobile: screen.mobile,
  })
  await session.send('Emulation.setTouchEmulationEnabled', { enabled: screen.touch, maxTouchPoints: screen.touch ? 5 : 1 })
  await session.send('Emulation.setUserAgentOverride', {
    userAgent: screen.userAgent ?? nativeUserAgent,
    acceptLanguage: emulation.locale,
  })
  // An empty value takes each override back.
  await session.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: emulation.theme ?? '' }] })
  await session.send('Emulation.setLocaleOverride', emulation.locale === undefined ? {} : { locale: emulation.locale })
  await session.send('Emulation.setTimezoneOverride', { timezoneId: emulation.timezone ?? '' })
}
