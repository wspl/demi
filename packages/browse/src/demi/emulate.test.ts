// Playwright's own emulation calls in a script, `setViewportSize` and
// `emulateMedia`, on the slot's page and on a page the script opens, run as
// the tool's server runs a call against pages the test serves, in a
// temporary slot: about 2 s, the browser's start. Playwright sets them
// through a session of its own, which the tool's session, holding the
// slot's emulation, overrode again at the next load; only the real browser
// shows which of the two a page gets.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { existsSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'
import { Browser } from '../browser'
import { runCall } from '../call'
import { Network } from '../network'
import { local, slotPorts, type Slot } from '../slot'

const root = mkdtempSync(join(tmpdir(), 'browse-emulate-'))
const slot: Slot = { root, number: 0, ports: slotPorts(0), folder: join(root, '.cache/browse') }
mkdirSync(slot.folder, { recursive: true })
const browser = new Browser(slot, () => undefined)
// The pages here are served by the test: the network stands in front of nothing.
const network = Network.listen(0, 1)
const gallery = local(slot.ports.gallery)
const available = existsSync(chromium.executablePath())

/**
 * The gallery as the tool sees it: a page whose theme its state module's
 * `galleryState.mode` sets, light until something changes it.
 */
const GALLERY_PAGE = '<!doctype html><html data-theme="light"><script type="module" src="/src/gallery-state.ts"></script></html>'
const GALLERY_STATE = `export const galleryState = {
  set mode(mode) { document.documentElement.setAttribute('data-theme', mode) },
  get mode() { return document.documentElement.getAttribute('data-theme') },
}`

beforeAll(async () => {
  if (!available) {
    return
  }
  const context = (await browser.page()).context()
  await context.route('http://browse.test/**', (route) => route.fulfill({ contentType: 'text/html', body: '<p>A page</p>' }))
  await context.route(`${gallery}/**`, (route) => new URL(route.request().url()).pathname === '/src/gallery-state.ts'
    ? route.fulfill({ contentType: 'text/javascript', body: GALLERY_STATE })
    : route.fulfill({ contentType: 'text/html', body: GALLERY_PAGE }))
})

afterAll(async () => {
  await browser.close()
  await (await network).shutdown()
  rmSync(root, { recursive: true, force: true })
})

/** Runs `script` as a call; answers its exit code and the first line it printed, its result. */
async function call(script: string): Promise<[number, string | undefined]> {
  const lines: string[] = []
  const outcome = await runCall(
    { slot, browser, network: () => network, keep: {} },
    { script, file: null, headed: false, env: {}, limitMs: 30_000 },
    (line) => lines.push(line),
  )
  return [outcome.exit, lines[0]]
}

test.skipIf(!available)('a viewport the script sets holds at once and across a reload', async () => {
  expect(await call(`await page.goto('http://browse.test/')
await page.setViewportSize({ width: 440, height: 800 })
const now = await page.evaluate(() => innerWidth)
await page.reload()
return [now, await page.evaluate(() => innerWidth), page.viewportSize()]`))
    .toEqual([0, '[ 440, 440, { width: 440, height: 800 } ]'])
})

test.skipIf(!available)('a page the script opens keeps the viewport it sets across a reload', async () => {
  expect(await call(`const opened = await context.newPage()
await opened.goto('http://browse.test/')
await opened.setViewportSize({ width: 600, height: 800 })
await opened.reload()
const width = await opened.evaluate(() => innerWidth)
await opened.close()
return width`)).toEqual([0, '600'])
})

test.skipIf(!available)('a colour scheme set through emulateMedia shows in the gallery\'s theme', async () => {
  expect(await call(`await page.goto('${gallery}/overview')
await page.emulateMedia({ colorScheme: 'dark' })
const theme = () => page.evaluate(() => [document.documentElement.dataset.theme, matchMedia('(prefers-color-scheme: dark)').matches])
const now = await theme()
await page.reload()
return [now, (await theme())[1]]`)).toEqual([0, "[ [ 'dark', true ], true ]"])
})
