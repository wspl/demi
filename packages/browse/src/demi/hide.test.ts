// `demi.hide` and `demi.show`, and the focus every other page keeps, run as the
// tool's server runs a call, against a page the test serves, in a temporary
// slot: about 2 s, the browser's start.
//
// Playwright's session with each page emulates focus, which keeps the page
// visible whatever tab is in front of it. Only the real browser shows whether a
// page the tool hides sees itself hidden.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { existsSync, mkdirSync, mkdtempSync, rmSync, watch } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'
import { Browser } from '../browser'
import { runCall } from '../call'
import { Network } from '../network'
import { slotPaths, slotPorts, type Slot } from '../slot'

const root = mkdtempSync(join(tmpdir(), 'browse-hide-'))
const slot: Slot = { root, number: 0, ports: slotPorts(0), folder: join(root, '.cache/browse') }
mkdirSync(slot.folder, { recursive: true })
const browser = new Browser(slot, () => undefined)
// The page here is served by the test: the network stands in front of nothing.
const network = Network.listen(0, 1)
const available = existsSync(chromium.executablePath())

/** A page that records each `visibilitychange` it gets, with the state it then says. */
const PAGE = `<!doctype html><a download="note.txt" href="data:text/plain,a note">Download</a>
<script>window.changes = []; document.addEventListener('visibilitychange', () => changes.push(document.visibilityState))</script>`

beforeAll(async () => {
  if (!available) {
    return
  }
  const context = (await browser.page()).context()
  await context.route('http://browse.test/**', (route) => route.fulfill({ contentType: 'text/html', body: PAGE }))
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

const STATE = `() => [document.visibilityState, changes.join(' ')]`

test.skipIf(!available)('a hidden page says hidden, then visible once shown, with an event each time', async () => {
  expect(await call(`await page.goto('http://browse.test/')
await demi.hide()
const hidden = await page.evaluate(${STATE})
await demi.show()
return [hidden, await page.evaluate(${STATE})]`))
    .toEqual([0, "[ [ 'hidden', 'hidden' ], [ 'visible', 'hidden visible' ] ]"])
})

test.skipIf(!available)('a page the call leaves hidden is visible in the next call', async () => {
  expect((await call(`await page.goto('http://browse.test/')
await demi.hide()`))[0]).toBe(0)
  expect(await call(`return page.evaluate(${STATE})`)).toEqual([0, "[ 'visible', 'hidden visible' ]"])
})

test.skipIf(!available)('a page behind another the script opens stays visible and focused', async () => {
  expect(await call(`const behind = await context.newPage()
await behind.goto('http://browse.test/')
const front = await context.newPage()
await front.bringToFront()
const state = await behind.evaluate(() => [document.visibilityState, document.hasFocus()])
await front.close()
await behind.close()
return state`)).toEqual([0, "[ 'visible', true ]"])
})

test.skipIf(!available)('a download lands in the slot\'s downloads folder', async () => {
  const folder = slotPaths(slot).downloads
  const landed = new Promise<void>((resolve) => {
    const watcher = watch(folder, () => {
      if (existsSync(join(folder, 'note.txt'))) {
        watcher.close()
        resolve()
      }
    })
  })
  expect((await call(`await page.goto('http://browse.test/')
await page.getByRole('link', { name: 'Download' }).click()`))[0]).toBe(0)
  await landed
})
