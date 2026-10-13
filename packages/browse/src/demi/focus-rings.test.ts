// `demi.focusRings` over a page with one focus indicator of each kind the
// check finds and two it must not: in the slot's kind of browser, started in
// a temporary slot, about a second.
import { afterAll, expect, test } from 'bun:test'
import { existsSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'
import { Browser } from '../browser'
import { slotPorts, type Slot } from '../slot'
import type { Tool } from '../tool'
import { focusRings } from './focus-rings'

const root = mkdtempSync(join(tmpdir(), 'browse-rings-'))
const slot: Slot = { root, number: 0, ports: slotPorts(0), folder: join(root, '.cache/browse') }
mkdirSync(slot.folder, { recursive: true })
const browser = new Browser(slot, () => undefined)

afterAll(async () => {
  await browser.close()
  rmSync(root, { recursive: true, force: true })
})

const PAGE = `
<style>
  body { margin: 40px; }
  button { outline: none; margin: 0 24px 24px 0; }
  .stack button { margin: 0; }
  button:focus-visible { outline: 2px solid blue; outline-offset: 1px; }
  .clip { overflow: hidden; width: 200px; height: 80px; background: #eee; margin-bottom: 24px; }
  .padded { padding: 4px; }
  .inset:focus-visible { outline-offset: -2px; }
  .ring:focus-visible { outline: none; box-shadow: 0 0 0 2px blue; }
  .stack { position: relative; height: 40px; }
  #under { width: 50px; }
  .stack .cover { position: absolute; z-index: 5; left: 50px; top: 0; width: 140px; height: 40px; background: #ccc; }
</style>
<div class="clip"><button id="flush">Flush</button></div>
<div class="clip padded"><button id="padded">Padded</button></div>
<div class="clip"><button id="inset" class="inset">Inset</button></div>
<div class="clip"><button id="shadow" class="ring">Shadow</button></div>
<div class="stack"><button id="under">Under</button><div class="cover"></div></div>
<button id="free">Free</button>
<button id="skipped" tabindex="-1">Skipped by Tab</button>`

test.skipIf(!existsSync(chromium.executablePath()))(
  'focus rings: an outline or a ring an ancestor clips or an element covers is reported with its side and its pixels, the rest is not',
  async () => {
    const lines: string[] = []
    const tool: Tool = {
      slot,
      browser,
      network: () => Promise.reject(new Error('focusRings needs no network')),
      print: (line) => lines.push(line),
      env: {},
      wrote: () => undefined,
      release: () => undefined,
      endServer: () => undefined,
    }
    const page = await browser.page()
    await page.setContent(PAGE)
    const report = await focusRings(tool, { shots: false })
    const found = report.offenders.map((offender) => [
      offender.element.split(' ')[1], offender.reached, offender.findings.map((finding) => [finding.kind, finding.by, finding.cuts]),
    ])
    expect(found).toEqual([
      ['"Flush"', 'Tab', [['clipped', 'div.clip', { top: 3, right: 0, bottom: 0, left: 3 }]]],
      ['"Shadow"', 'Tab', [['clipped', 'div.clip', { top: 2, right: 0, bottom: 0, left: 2 }]]],
      ['"Under"', 'Tab', [['covered', 'div.cover', { top: 0, right: 2, bottom: 0, left: 0 }]]],
    ])
    expect(report.checked).toBe(7)
  },
)

const WITH_DIALOG = `
<style>
  button { outline: none; }
  button:focus-visible { outline: 2px solid blue; outline-offset: 1px; }
  .clip { overflow: hidden; width: 120px; height: 40px; }
  .dialog { position: fixed; left: 200px; top: 100px; width: 200px; background: white; }
  .selected { outline: 2px solid blue; }
</style>
<div class="clip"><button id="behind">Behind</button></div>
<div class="dialog" role="dialog"><div class="clip"><button id="inside">Inside</button></div><button id="free">Free</button></div>
<div class="clip"><span class="selected" id="chosen">Chosen</span></div>`

test.skipIf(!existsSync(chromium.executablePath()))(
  'focus rings: an open dialog is the only part checked, and an indicator a state draws is read where it stands',
  async () => {
    const tool: Tool = {
      slot,
      browser,
      network: () => Promise.reject(new Error('focusRings needs no network')),
      print: () => undefined,
      env: {},
      wrote: () => undefined,
      release: () => undefined,
      endServer: () => undefined,
    }
    const page = await browser.page()
    await page.setContent(WITH_DIALOG)
    const inDialog = await focusRings(tool, { shots: false })
    const chosen = await focusRings.of(tool, page.locator('#chosen'), { shots: false })
    expect([
      inDialog.checked, inDialog.offenders.map((offender) => offender.element.split(' ')[1]),
      chosen.offenders.map((offender) => [offender.reached, offender.findings.map((finding) => [finding.by, finding.cuts])]),
    ]).toEqual([
      2, ['"Inside"'],
      [['shown', [['div.clip', { top: 2, right: 0, bottom: 0, left: 2 }]]]],
    ])
  },
)

// The markers are web-ui's `ResizeHandle`: the root, and the line inside it that the keyboard's focus lights.
const WITH_HANDLES = `
<style>
  [data-resize-handle] { outline: none; position: relative; height: 8px; width: 200px; margin: 24px 0; }
  [data-resize-line] { position: absolute; left: 0; right: 0; top: 3px; height: 2px; }
  [data-resize-handle]:focus-visible [data-resize-line]:not(.unlit) { background: blue; }
  .clip { overflow: hidden; width: 200px; }
</style>
<div role="separator" tabindex="0" data-resize-handle aria-label="Open"><span data-resize-line></span></div>
<div role="separator" tabindex="0" data-resize-handle aria-label="Unlit"><span data-resize-line class="unlit"></span></div>
<div contenteditable aria-label="Field" style="outline: none">text</div>
<div data-region tabindex="-1" style="outline: none">Pane</div>`

test.skipIf(!existsSync(chromium.executablePath()))(
  'focus rings: a resize handle\'s lit line is its indicator, an unlit one is listed',
  async () => {
    const tool: Tool = {
      slot,
      browser,
      network: () => Promise.reject(new Error('focusRings needs no network')),
      print: () => undefined,
      env: {},
      wrote: () => undefined,
      release: () => undefined,
      endServer: () => undefined,
    }
    const page = await browser.page()
    await page.setContent(WITH_HANDLES)
    const report = await focusRings(tool, { shots: false })
    expect([report.checked, report.offenders.length, report.withoutIndicator]).toEqual([1, 0, ['div Unlit']])
  },
)

const WITH_LINKS = `
<style>
  a { outline: none; text-decoration: underline; text-decoration-color: gray; margin-right: 24px; }
  a.shown:focus-visible { text-decoration-color: black; }
</style>
<a href="#" class="shown">Looks</a>
<a href="#" class="plain">Plain</a>`

test.skipIf(!existsSync(chromium.executablePath()))(
  'focus rings: a link in text has the look the keyboard gives it as its indicator; one that does not change is listed',
  async () => {
    const tool: Tool = {
      slot,
      browser,
      network: () => Promise.reject(new Error('focusRings needs no network')),
      print: () => undefined,
      env: {},
      wrote: () => undefined,
      release: () => undefined,
      endServer: () => undefined,
    }
    const page = await browser.page()
    await page.setContent(WITH_LINKS)
    const report = await focusRings(tool, { shots: false })
    expect([report.checked, report.offenders.length, report.withoutIndicator]).toEqual([1, 0, ['a Plain']])
  },
)

const WITH_LISTS = `
<style>
  [role=listbox] { outline: none; margin-bottom: 24px; }
  .lit { outline: 2px solid blue; outline-offset: 1px; }
</style>
<div role="listbox" tabindex="0" aria-label="Lit" aria-activedescendant="lit-1">
  <div role="option" id="lit-0">First</div><div role="option" id="lit-1" class="lit">Second</div>
</div>
<div role="listbox" tabindex="0" aria-label="Unlit" aria-activedescendant="unlit-0">
  <div role="option" id="unlit-0">First</div>
</div>
<div role="listbox" tabindex="0" aria-label="Idle">
  <div role="option">First</div>
</div>`

test.skipIf(!existsSync(chromium.executablePath()))(
  'focus rings: a list that keeps the focus shows its ring on its active row; one with no lit active row is listed',
  async () => {
    const tool: Tool = {
      slot,
      browser,
      network: () => Promise.reject(new Error('focusRings needs no network')),
      print: () => undefined,
      env: {},
      wrote: () => undefined,
      release: () => undefined,
      endServer: () => undefined,
    }
    const page = await browser.page()
    await page.setContent(WITH_LISTS)
    const report = await focusRings(tool, { shots: false })
    expect([report.checked, report.offenders.length, report.withoutIndicator]).toEqual([1, 0, ['div Unlit', 'div Idle']])
  },
)
