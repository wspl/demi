// `demi.ime` in the slot's kind of browser, Chrome with its own menus,
// started in a temporary slot: about a second, the browser's start.
// Playwright's own headless browser has no menus, so it would not show
// what a composing key that reaches them does.
import { afterAll, expect, test } from 'bun:test'
import { existsSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'
import { Browser } from '../browser'
import { slotPorts, type Slot } from '../slot'
import type { Tool } from '../tool'
import { ime } from './ime'

const root = mkdtempSync(join(tmpdir(), 'browse-ime-'))
const slot: Slot = { root, number: 0, ports: slotPorts(0), folder: join(root, '.cache/browse') }
mkdirSync(slot.folder, { recursive: true })
const browser = new Browser(slot, () => undefined)

afterAll(async () => {
  await browser.close()
  rmSync(root, { recursive: true, force: true })
})

test.skipIf(!existsSync(chromium.executablePath()))(
  'a composition the page leaves unhandled, ended either way, keeps the page where it was',
  async () => {
    const tool: Tool = {
      slot,
      browser,
      network: () => Promise.reject(new Error('ime needs no network')),
      print: () => undefined,
      env: {},
      wrote: () => undefined,
      release: () => undefined,
      endServer: () => undefined,
    }
    const page = await browser.page()
    await page.setContent('<textarea></textarea>')
    const field = page.locator('textarea')
    // The keys reach the browser's menus once the page lets them through; the second already went astray.
    for (const commit of ['escape', 'escape', 'enter', 'escape'] as const) {
      await ime(tool, '你好', { into: field, commit })
    }
    expect([page.url(), await field.inputValue()]).toEqual(['about:blank', '你好'])
  },
)
