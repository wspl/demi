// `demi.webkit()`: a page of Playwright's WebKit build, for what a
// check must see in Safari's engine as well as in the slot's Chromium, such as the
// order of an input method's events (`demi.ime`).
//
// The page behaves like the slot's `page`:
//
// - It is signed in as the slot's browser is.
// - It has the slot's screen, theme, locale and time zone (`demi.emulate`), so
//   `demi.shot` pictures it as it pictures `page`.
// - Its `goto` takes an address relative to the slot's web app as `page`'s does.
//
// It lasts for the call: the call closes its browser when it ends. Its pages do
// not advertise push notifications, which this embedder cannot serve. Actual
// Safari is needed for a push check; service workers and offline storage remain.
import { existsSync } from 'node:fs'
import { webkit, type Page } from 'playwright'
import { install } from '../browser'
import { screenOf } from '../emulation'
import { readState } from '../state'
import { webBase, type Tool } from '../tool'

/** How long an action on the page waits for its element, as on the slot's page. */
const ACTION_MS = 10_000

export async function webkitPage(tool: Tool): Promise<Page> {
  if (!existsSync(webkit.executablePath())) {
    await install('webkit', tool.print)
  }
  await tool.network()
  const signedIn = await (await tool.browser.page()).context().storageState()
  const browser = await webkit.launch()
  tool.release(() => browser.close())
  const emulation = readState(tool.slot).emulation ?? {}
  const screen = screenOf(emulation)
  const context = await browser.newContext({
    baseURL: webBase(tool.slot),
    storageState: signedIn,
    viewport: { width: screen.width, height: screen.height },
    deviceScaleFactor: screen.scale,
    isMobile: screen.mobile,
    hasTouch: screen.touch,
    userAgent: screen.userAgent ?? undefined,
    colorScheme: emulation.theme ?? null,
    locale: emulation.locale,
    timezoneId: emulation.timezone,
  })
  // Playwright enables Push API in the renderer through the inspector, but not
  // its native IPC endpoint. getSubscription then kills the renderer. Its public
  // API has no per-feature WebKit setting, so hide the unsupported interfaces
  // before page code runs, including on reload and in child frames. Do not fake
  // a subscription result or disable the service worker to avoid that call.
  await context.addInitScript(() => {
    for (const name of ['PushManager', 'PushSubscription', 'PushSubscriptionOptions'])
      Reflect.deleteProperty(globalThis, name)
    if (typeof ServiceWorkerRegistration !== 'undefined')
      Reflect.deleteProperty(ServiceWorkerRegistration.prototype, 'pushManager')
  })
  context.setDefaultTimeout(ACTION_MS)
  return context.newPage()
}
