import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { compile } from '@tailwindcss/node'
import { chromium, type Browser, type Page } from 'playwright'
import { vueSingleFileComponents } from '../../__tests__/vue-components'

// The work panel's tab strip keeps every tab's width and reach when crowded:
//
// - The pinned tabs show their kinds' own titles, whatever they show.
// - Selecting a tab changes no tab's width. A long title, a selected one or the
//   open file's name never widens its tab past its share.
// - With more tabs than fit, a list of every tab selects the one picked.
//
// Widths are layout, so this runs the product's WorkPanel in a real Chromium
// with the stylesheet Tailwind builds from it; happy-dom lays nothing out.
// About 5 s: the page's and the stylesheet's builds and the browser's start,
// shared, and a page per test.

/** One tab of the strip as the page measures it. */
interface Box {
  name: string
  left: number
  right: number
  width: number
  selected: boolean
}

declare global {
  interface Window {
    /**
     * Mounts a work panel `width` px wide with a page tab of each title, the
     * `selected` one selected, and Changes and Files pinned.
     */
    showPanel(options: { width: number; titles: string[]; selected: string; file?: string }): void
    /** Every tab of the strip, the pinned ones first. */
    tabBoxes(): Box[]
    /** The strip's view of its own tabs. */
    stripView(): { left: number; right: number }
    /** What the panel has selected, as its host keeps it. */
    selection(): string
  }
}

const PAGE_ENTRY = `
import { createApp, defineComponent, h, ref } from 'vue'
import { z } from 'zod'
import { File, FileDiff, Globe } from '@lucide/vue'
import WorkPanel from '../WorkPanel.vue'

const Nothing = defineComponent({ setup: () => () => null })
const kinds = [
  {
    kind: 'change', schema: z.object({}), title: () => 'Changes', content: Nothing,
    mark: defineComponent({ setup: () => () => h(FileDiff, { size: 16 }) }),
    badge: defineComponent({ setup: () => () => h('span', { class: 'flex gap-0.5 text-[12px]' }, [h('span', '+4066'), h('span', '−2005')]) }),
    pinned: { data: () => ({}) },
  },
  {
    kind: 'file', schema: z.object({ path: z.string().optional() }), content: Nothing,
    title: (data) => data.path ? 'File: ' + data.path : 'Files',
    mark: defineComponent({ setup: () => () => h(File, { size: 16 }) }),
    pinned: { data: () => ({}) },
  },
  {
    kind: 'page', schema: z.object({ title: z.string() }), title: (data) => data.title, content: Nothing,
    mark: defineComponent({ setup: () => () => h(Globe, { size: 16 }) }),
  },
]

window.showPanel = ({ width, titles, selected, file }) => {
  const history = ref([selected])
  const root = document.createElement('div')
  root.style.cssText = 'width:' + width + 'px;height:400px;display:flex'
  document.body.append(root)
  createApp({
    render: () => h(WorkPanel, {
      class: 'w-full',
      panel: { history: history.value, tabs: titles.map((title) => ({ id: title, kind: 'page', data: { title } })) },
      pinned: file ? { file: { path: file } } : {},
      kinds,
      onSelect: (id) => { history.value = [...history.value, id] },
    }, { default: () => h('span') }),
  }).mount(root)
  window.selection = () => history.value.at(-1)
}

window.tabBoxes = () => [...document.querySelectorAll('[role="tablist"] [role="tab"]')].map((tab) => {
  const box = tab.getBoundingClientRect()
  return { name: tab.textContent.trim(), left: box.left, right: box.right, width: box.width, selected: tab.getAttribute('aria-selected') === 'true' }
})

window.stripView = () => {
  // The row the strip's own tabs scroll in, after the pinned ones.
  const view = [...document.querySelectorAll('[role="tablist"] [role="tab"]')].at(-1).parentElement.getBoundingClientRect()
  return { left: view.left, right: view.right }
}
`

const PANEL_PX = 596

describe.skipIf(!existsSync(chromium.executablePath()))('in Chromium', () => {
  let browser: Browser
  let script: string
  let css: string
  beforeAll(async () => {
    const entry = fileURLToPath(new URL('./work-panel-strip-entry.ts', import.meta.url))
    const build = await Bun.build({
      entrypoints: [entry],
      files: { [entry]: PAGE_ENTRY },
      plugins: [vueSingleFileComponents],
      target: 'browser',
      conditions: ['development'],
      define: { 'process.env.NODE_ENV': '"development"' },
    })
    if (!build.success)
      throw new AggregateError(build.logs, 'The page did not build')
    script = await build.outputs[0]!.text()
    // The product's stylesheet with every class the page's code names, as
    // Tailwind's scan of the sources finds them.
    const styles = fileURLToPath(new URL('../../styles/', import.meta.url))
    const compiler = await compile(`@import 'tailwindcss';\n@import './base.css';`, { base: styles, onDependency: () => {} })
    css = compiler.build([...new Set(script.split(/[\s'"`\\]+/))])
    browser = await chromium.launch()
  })
  afterAll(async () => {
    await browser?.close()
  })

  async function panel(options: { titles: string[]; selected: string; file?: string; width?: number }): Promise<Page> {
    const opened = await browser.newPage({ viewport: { width: 1000, height: 600 }, reducedMotion: 'reduce' })
    await opened.route('https://demi.test/', (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html><body></body>' }))
    await opened.goto('https://demi.test/')
    await opened.addStyleTag({ content: css })
    // A module, so the bundle's top-level functions, such as Vue's
    // addEventListener, stay off the window.
    await opened.addScriptTag({ content: script, type: 'module' })
    // An inline module runs once the page has parsed it, after the tag is in.
    await opened.waitForFunction(() => typeof window.showPanel === 'function')
    await opened.evaluate((shown) => window.showPanel(shown), { width: PANEL_PX, ...options })
    return opened
  }

  /**
   * The tabs once the strip has measured them and settled, across two frames.
   */
  async function settled(opened: Page): Promise<Box[]> {
    await opened.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
    return opened.evaluate(() => window.tabBoxes())
  }

  test('selecting a tab, pinned or not, changes no tab\'s width', async () => {
    const titles = ['127.0.0.1:5173', 'Build log', 'API reference', 'Pull request #42: Keep the session cookie on sign-out', 'Staging']
    const opened = await panel({ titles, selected: 'change', file: 'src/notes/2026-10-17-crowded-work-panel.md' })
    const first = await settled(opened)
    expect(first).toHaveLength(7)
    const widths = first.map((tab) => tab.width)
    for (const index of [1, 2, 4, 6, 0]) {
      await opened.locator('[role="tablist"] [role="tab"]').nth(index).dispatchEvent('pointerdown', { button: 0 })
      const now = await settled(opened)
      expect(now[index]!.selected).toBe(true)
      expect(now.map((tab) => tab.width)).toEqual(widths)
    }
    await opened.close()
  })

  test('a long title, selected or the open file\'s name, never widens its tab past the others\' share or the panel', async () => {
    const long = 'A page whose title runs on far longer than any tab could show in the panel, and then some more'
    const titles = ['Log', long, 'Pull request #42: Keep the session cookie on sign-out', 'Error log 2026']
    // Wide enough that the cut titles share more than their least width.
    const opened = await panel({ titles, selected: 'file', file: 'docs/2026-10-17-crowded-work-panel-regression-investigation-notes.md', width: 760 })
    const plain = await settled(opened)
    const view = await opened.evaluate(() => window.stripView())
    const [change, file, short, longTab, request, dashboard] = plain
    // The pinned tabs show their kinds' titles beside their icons, not the open
    // file's name, which is File's tooltip.
    expect(change!.name).toBe('Changes+4066−2005')
    expect(file!.name).toBe('Files')
    expect(file!.width).toBeGreaterThan(40)
    // The tabs whose titles are cut share the room equally, the long one no
    // wider than the rest.
    expect(longTab!.width).toBeCloseTo(request!.width, 0)
    expect(longTab!.width).toBeCloseTo(dashboard!.width, 0)
    // A short title takes what it needs and leaves the rest to them: no room
    // stays empty while a title is cut.
    expect(short!.width).toBeLessThan(request!.width)
    expect(dashboard!.right).toBeCloseTo(view.right, 0)
    // Every tab shows within the strip, none under the panel's own controls.
    for (const tab of plain.slice(2))
      expect(tab.right).toBeLessThanOrEqual(view.right + 0.5)
    // Selected, the long title keeps the width it had.
    await opened.locator('[role="tablist"] [role="tab"]').nth(3).dispatchEvent('pointerdown', { button: 0 })
    expect((await settled(opened))[3]!.width).toBe(longTab!.width)
    await opened.close()
  })

  test('with more tabs than fit, the list of every tab selects the one picked; with few there is no list', async () => {
    const few = await panel({ titles: ['Build log', 'Staging', 'API reference'], selected: 'Build log' })
    await settled(few)
    expect(await few.getByRole('button', { name: 'Show All Tabs' }).count()).toBe(0)
    await few.close()

    const titles = Array.from({ length: 25 }, (_, index) => `Page ${index + 1}`)
    const opened = await panel({ titles, selected: 'Page 25' })
    await settled(opened)
    const list = opened.getByRole('button', { name: 'Show All Tabs' })
    expect(await list.count()).toBe(1)
    await list.click()
    const rows = opened.getByRole('menuitem')
    expect(await rows.allTextContents()).toEqual(['Changes', 'Files', ...titles].map((title) => expect.stringContaining(title)))
    await rows.filter({ hasText: /^Page 3$/ }).click()
    expect(await opened.evaluate(() => window.selection())).toBe('Page 3')
    await opened.getByRole('menuitem').first().waitFor({ state: 'detached' })
    // The picked tab is selected and brought into the strip's view.
    const tabs = await settled(opened)
    const picked = tabs.find((tab) => tab.name === 'Page 3')!
    const view = await opened.evaluate(() => window.stripView())
    expect(picked.selected).toBe(true)
    expect(picked.left).toBeGreaterThanOrEqual(view.left - 0.5)
    expect(picked.right).toBeLessThanOrEqual(view.right + 0.5)
    // The titled pinned tabs leave the strip's tabs their least width, and the
    // strip scrolls the rest out of view.
    const [change, file, ...pages] = tabs
    expect([change!.name, file!.name]).toEqual(['Changes+4066−2005', 'Files'])
    for (const tab of pages)
      expect(tab.width).toBeGreaterThanOrEqual(87.5)
    expect(pages.some((tab) => tab.right > view.right + 0.5)).toBe(true)
    await opened.close()
  })
})
