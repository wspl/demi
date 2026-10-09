// `demi.webkit()`: a page of Playwright's WebKit, the engine of Safari, for
// what a check must see in Safari's engine as well as in the slot's
// Chromium, such as the order of an input method's events (`demi.ime`). It
// is signed in as the slot's browser is, has its screen, theme, locale and
// time zone (`demi.emulate`), so `demi.shot` pictures it as it pictures
// `page`, its `goto` takes an address relative to the slot's web app as
// `page`'s does, and it lasts for the call: the call closes its browser
// when it ends.
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
  context.setDefaultTimeout(ACTION_MS)
  return context.newPage()
}
