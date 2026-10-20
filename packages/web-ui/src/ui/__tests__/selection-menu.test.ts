import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { compile } from '@tailwindcss/node'
import { chromium, type Browser, type Page } from 'playwright'
import { vueSingleFileComponents, withoutViteGlobs } from '../../__tests__/vue-components'

// A right-click on selected text: it keeps the selection, its menu is the
// selection's alone wherever over the selected lines it lands, a gap between
// two list items included, the selection's toolbar steps aside while the menu
// is open and comes back after it, a selection that went leaves the message's
// menu, and the toolbar appears once a drag ends, in its place. Selections, hit
// tests and layout are the browser's, so this runs the product's reply, user
// message, ContextMenu and SelectionToolbar in a real Chromium with the
// stylesheet Tailwind builds; happy-dom has none of them. About 4 s: the page's
// and the stylesheet's builds and the browser's start, shared, and a page per
// test.

declare global {
  interface Window {
    /** Mounts the page's menu and toolbar, a user's message, a reply with a two-item list and a message field. */
    showTranscript(): void
    /** The toolbar as each frame drew it: null when it showed nothing, else where it stood. */
    toolbarFrames: ({ top: number; left: number } | null)[]
  }
}

const PAGE_ENTRY = `
import { createApp, defineComponent, h } from 'vue'
import ContextMenu from '../ContextMenu.vue'
import Popover from '../Popover.vue'
import { appOverlayStore } from '../../overlay/appOverlay'
import SelectionToolbar from '../SelectionToolbar.vue'
import AssistantTextBlock from '../../agent/blocks/AssistantTextBlock.vue'
import UserBlock from '../../agent/blocks/UserBlock.vue'
import { provideSendTargets } from '../../agent/send-to-conversation'
import { provideReplySelectionTools } from '../../agent/reply-selection'
import { provideBlobUrl } from '../../agent/media-source'
import { provideMediaViewer } from '../../files/media-viewer'

const REPLY = [
  'The upload fails for two reasons:',
  '',
  '- 客户端在服务器回应之前就放弃了等待，因为超时只有三十秒。',
  '- 每个分块有四兆字节，在慢速网络上传输时间太长。',
  '',
  'Smaller chunks fix both.',
].join('\\n')

window.toolbarFrames = []
const sample = () => {
  const toolbar = document.querySelector('[data-selection-toolbar]')
  const box = toolbar && getComputedStyle(toolbar).visibility !== 'hidden' ? toolbar.getBoundingClientRect() : null
  window.toolbarFrames.push(box && { top: Math.round(box.top), left: Math.round(box.left) })
  requestAnimationFrame(sample)
}
requestAnimationFrame(sample)

window.showTranscript = () => {
  const root = document.createElement('div')
  root.style.cssText = 'width:640px;padding:80px 0 0'
  document.body.append(root)
  createApp(defineComponent({
    setup() {
      provideBlobUrl((source) => 'blob:' + source)
      provideMediaViewer()
      provideSendTargets({ recent: () => [], search: { search: async () => [] }, describe: () => null, shown: () => 'c1', send: () => {} })
      provideReplySelectionTools({
        selection: () => [
          { label: 'Comment', inPlace: true, run: () => {} },
          { label: 'Ask', inPlace: true, run: () => {} },
        ],
        picture: () => [],
      })
      return () => [
        h(UserBlock, { content: [{ type: 'text', text: 'Why do my uploads keep failing on the train?' }] }),
        h(AssistantTextBlock, { blockId: 'text-1', content: REPLY, createdAt: '2026-10-18T10:00:00Z', showFooter: true, fork: async () => {}, share: { start: () => {} } }),
        // The message field, which has the focus as the page opens, as the composer does.
        h('textarea', { 'aria-label': 'Message', autofocus: true }),
        // A closed menu by the field, as the composer's are, which keeps the page's focus tracking (focusReturn.ts) listening.
        h(Popover, { isOpen: false, overlayStore: appOverlayStore }),
        h(ContextMenu),
        h(SelectionToolbar),
      ]
    },
  })).mount(root)
}
`

describe.skipIf(!existsSync(chromium.executablePath()))('in Chromium', () => {
  let browser: Browser
  let script: string
  let css: string
  beforeAll(async () => {
    const entry = fileURLToPath(new URL('./selection-menu-entry.ts', import.meta.url))
    const build = await Bun.build({
      entrypoints: [entry],
      files: { [entry]: PAGE_ENTRY },
      plugins: [vueSingleFileComponents, withoutViteGlobs],
      target: 'browser',
      conditions: ['development'],
      define: { 'process.env.NODE_ENV': '"development"' },
    })
    if (!build.success)
      throw new AggregateError(build.logs, 'The page did not build')
    script = await build.outputs[0]!.text()
    // The product's stylesheet with every class the page's code names, as Tailwind's scan of the sources finds them.
    const styles = fileURLToPath(new URL('../../styles/', import.meta.url))
    const compiler = await compile(`@import 'tailwindcss';\n@import './base.css';`, { base: styles, onDependency: () => {} })
    css = compiler.build([...new Set(script.split(/[\s'"`\\]+/))])
    browser = await chromium.launch()
  })
  afterAll(async () => {
    await browser?.close()
  })

  async function transcript(): Promise<Page> {
    const opened = await browser.newPage({ viewport: { width: 800, height: 700 }, reducedMotion: 'reduce' })
    await opened.route('https://demi.test/', (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html><body></body>' }))
    await opened.goto('https://demi.test/')
    await opened.addStyleTag({ content: css })
    await opened.addScriptTag({ content: script, type: 'module' })
    await opened.waitForFunction(() => typeof window.showTranscript === 'function')
    await opened.evaluate(() => window.showTranscript())
    await opened.locator('li').nth(1).waitFor()
    await opened.getByRole('textbox', { name: 'Message' }).focus()
    return opened
  }

  /** The middle of a text's first and last characters, where a drag over it starts and ends. */
  async function ends(opened: Page, selector: string, from: string, to: string) {
    return opened.evaluate(({ selector, from, to }) => {
      const at = (text: string, last: boolean) => {
        for (const el of document.querySelectorAll(selector)) {
          const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
          for (let node = walker.nextNode() as Text | null; node; node = walker.nextNode() as Text | null) {
            const index = node.data.indexOf(text)
            if (index < 0)
              continue
            const range = document.createRange()
            const offset = last ? index + text.length - 1 : index
            range.setStart(node, offset)
            range.setEnd(node, offset + 1)
            const box = range.getBoundingClientRect()
            return { x: box.left + box.width / 2, y: box.top + box.height / 2 }
          }
        }
        throw new Error('No text ' + text)
      }
      return { start: at(from, false), end: at(to, true) }
    }, { selector, from, to })
  }

  /** Selects from `from` to `to` by a mouse drag, as the user does. */
  async function drag(opened: Page, selector: string, from: string, to: string): Promise<void> {
    const { start, end } = await ends(opened, selector, from, to)
    await opened.mouse.move(start.x - 3, start.y)
    await opened.mouse.down()
    await opened.mouse.move((start.x + end.x) / 2, (start.y + end.y) / 2, { steps: 4 })
    await opened.mouse.move(end.x + 3, end.y, { steps: 4 })
    await opened.mouse.up()
  }

  const selected = (opened: Page) => opened.evaluate(() => window.getSelection()?.toString() ?? '')
  const menuRows = (opened: Page) => opened.locator('[role="menuitem"]').allTextContents().then((rows) => rows.map((row) => row.trim()))
  const toolbar = (opened: Page) => opened.locator('[data-selection-toolbar]')

  test('a right-click inside a selection across two list items keeps it and opens its menu alone, the toolbar hidden until Escape', async () => {
    const opened = await transcript()
    await drag(opened, 'li', '服务器回应', '传输时间')
    const words = await selected(opened)
    expect(words).toContain('服务器回应')
    expect(words).toContain('传输时间')
    await toolbar(opened).waitFor()
    const { start } = await ends(opened, 'li', '放弃了', '放弃了')
    await opened.mouse.click(start.x, start.y, { button: 'right' })
    await opened.locator('[role="menu"]').waitFor()
    expect(await selected(opened)).toBe(words)
    expect(await menuRows(opened)).toEqual(['Copy', 'Comment', 'Ask', 'Send to Conversation'])
    expect(await toolbar(opened).count()).toBe(0)
    await opened.keyboard.press('Escape')
    await opened.locator('[role="menu"]').waitFor({ state: 'detached' })
    expect(await selected(opened)).toBe(words)
    await toolbar(opened).waitFor()
    await opened.close()
  })

  test('a right-click in the gap between two selected list items opens the selection menu', async () => {
    const opened = await transcript()
    await drag(opened, 'li', '服务器回应', '传输时间')
    const words = await selected(opened)
    // Halfway between the first item's last line and the second's first, where no text is.
    const gap = await opened.evaluate(() => {
      const [first, second] = [...document.querySelectorAll('li')].map((li) => li.getBoundingClientRect())
      return { x: first!.left + 40, y: (first!.bottom + second!.top) / 2 }
    })
    const under = await opened.evaluate(({ x, y }) => {
      const range = window.getSelection()!.getRangeAt(0)
      return [...range.getClientRects()].some((rect) => x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom)
    }, gap)
    // The point is between the selection's lines, not on any of them.
    expect(under).toBe(false)
    await opened.mouse.click(gap.x, gap.y, { button: 'right' })
    await opened.locator('[role="menu"]').waitFor()
    expect(await selected(opened)).toBe(words)
    expect(await menuRows(opened)).toEqual(['Copy', 'Comment', 'Ask', 'Send to Conversation'])
    await opened.close()
  })

  test('off a selection, or once it went, the message has its own menu', async () => {
    const opened = await transcript()
    await drag(opened, 'li', '服务器回应', '传输时间')
    await toolbar(opened).waitFor()
    // The reply's last paragraph, which the selection leaves out.
    const { start: off } = await ends(opened, 'p', 'Smaller', 'Smaller')
    await opened.mouse.click(off.x, off.y, { button: 'right' })
    await opened.locator('[role="menu"]').waitFor()
    expect(await menuRows(opened)).toEqual(['Copy Message', 'Fork', 'Share'])
    await opened.keyboard.press('Escape')
    await opened.locator('[role="menu"]').waitFor({ state: 'detached' })
    // A click ends the selection: where it was, the message's menu opens.
    await opened.mouse.click(off.x, off.y)
    expect(await selected(opened)).toBe('')
    const { start: inside } = await ends(opened, 'li', '放弃了', '放弃了')
    await opened.mouse.click(inside.x, inside.y, { button: 'right' })
    await opened.locator('[role="menu"]').waitFor()
    expect(await menuRows(opened)).toEqual(['Copy Message', 'Fork', 'Share'])
    await opened.close()
  })

  test('blank space beside a selected paragraph, outside its bounds, is not on the selection', async () => {
    const opened = await transcript()
    await drag(opened, '.select-text', 'Why', 'train?')
    expect(await selected(opened)).toContain('Why do my uploads keep failing on the train?')
    const places = await opened.evaluate(() => {
      const bubble = document.querySelector('.select-text')!.closest('.rounded-xl')!.getBoundingClientRect()
      return {
        // In the bubble, under its text.
        padding: { x: bubble.left + bubble.width / 2, y: bubble.bottom - 3 },
        // Right of the bubble, still in the message's row.
        right: { x: bubble.right + 12, y: bubble.top + bubble.height / 2 },
        // The page above the transcript.
        above: { x: bubble.left + bubble.width / 2, y: 30 },
      }
    })
    for (const where of [places.padding, places.right]) {
      await opened.mouse.click(where.x, where.y, { button: 'right' })
      await opened.locator('[role="menu"]').waitFor()
      const rows = await menuRows(opened)
      expect(rows).toContain('Copy Message')
      expect(rows).not.toContain('Copy')
      await opened.keyboard.press('Escape')
      await opened.locator('[role="menu"]').waitFor({ state: 'detached' })
    }
    await opened.mouse.click(places.above.x, places.above.y, { button: 'right' })
    // Nothing declares a menu above the transcript: none opens, the frame after the click included.
    await opened.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
    expect(await opened.locator('[role="menu"]').count()).toBe(0)
    await opened.close()
  })

  test('a user message\'s selection menu holds one Send to Conversation and no message entries', async () => {
    const opened = await transcript()
    await drag(opened, '.select-text', 'uploads keep', 'failing')
    const words = await selected(opened)
    expect(words).toContain('uploads keep')
    const { start } = await ends(opened, '.select-text', 'keep', 'keep')
    await opened.mouse.click(start.x, start.y, { button: 'right' })
    await opened.locator('[role="menu"]').waitFor()
    expect(await selected(opened)).toBe(words)
    expect(await menuRows(opened)).toEqual(['Copy', 'Send to Conversation'])
    await opened.close()
  })

  test('a drag-selection\'s toolbar appears once, after the release, where it stays', async () => {
    const opened = await transcript()
    const { start, end } = await ends(opened, 'li', '服务器回应', '传输时间')
    await opened.mouse.move(start.x - 3, start.y)
    await opened.mouse.down()
    await opened.mouse.move(end.x + 3, end.y, { steps: 8 })
    const released = await opened.evaluate(() => window.toolbarFrames.length)
    await opened.mouse.up()
    await toolbar(opened).waitFor()
    // A few frames after it stands, to see it neither move nor blink.
    await opened.evaluate(() => new Promise((resolve) => {
      let left = 4
      const next = () => (--left === 0 ? resolve(null) : requestAnimationFrame(next))
      requestAnimationFrame(next)
    }))
    const frames = await opened.evaluate(() => window.toolbarFrames)
    // Nothing drew it while the drag held.
    expect(frames.slice(0, released).every((frame) => frame === null)).toBe(true)
    const after = frames.slice(released)
    const first = after.findIndex((frame) => frame !== null)
    const shown = after.slice(first)
    // Once drawn it stays, in the one place it first stood.
    expect(shown.every((frame) => frame !== null && frame.top === shown[0]!.top && frame.left === shown[0]!.left)).toBe(true)
    await opened.close()
  })
})
