import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { compile } from '@tailwindcss/node'
import { chromium, type Browser, type Page } from 'playwright'
import { vueSingleFileComponents, withoutViteGlobs } from '../../__tests__/vue-components'

// A right-click on selected text:
//
// - It keeps the selection.
// - Its menu is the selection's alone wherever over the selected lines it lands, a
//   gap between two list items included.
// - The selection's toolbar steps aside while the menu is open and comes back
//   after it.
// - A selection that went leaves the message's menu.
// - The toolbar appears once a drag ends, in its place.
// - A triple-click on a reply's last paragraph, its last list item, or a paragraph
//   before a side question's card shows Comment and Ask, though the browser ends
//   the selection at the timestamp or the card under it; a selection that takes
//   the card's words shows none.
// - The toolbar stands over the middle of the selected lines, whatever their
//   first word is drawn in.
// - A selection that takes a character of the timestamp is no longer the
//   reply's text; one that ends after a picture covers the picture.
// - A comment quotes the selection as Copy copies it: whitespace selected is kept,
//   and two paragraphs have a blank line between them.
//
// Selections, hit tests and layout are the browser's, so this runs the product's
// reply, user message, comments layer, ContextMenu and SelectionToolbar in a real
// Chromium with the stylesheet Tailwind builds; happy-dom has none of them.
//
// Cost: about 7 s: the page's and the stylesheet's builds and the browser's start,
// shared, and a page per test.

declare global {
  interface Window {
    /**
     * Mounts the page's menu and toolbar, a user's message, replies with the
     * comments layer over them, one with a side question's card, and a message
     * field.
     */
    showTranscript(): void
    /** The quotes of the comments saved, in order. */
    commented: string[]
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
import ReplyComments from '../../agent/comments/ReplyComments.vue'
import { EMPTY_TRANSCRIPT } from '@demicodes/conversation-client'
import { ref } from 'vue'

const REPLY = [
  'The upload fails for two reasons:',
  '',
  '- 客户端在服务器回应之前就放弃了等待，因为超时只有三十秒。',
  '- 每个分块有四兆字节，在慢速网络上传输时间太长。',
  '',
  'Smaller chunks fix both.',
].join('\\n')

// A reply that ends with a list.
const STEPS = [
  'Two steps bring the build back:',
  '',
  '- Clear the cache of the old toolchain.',
  '- Run the whole test suite once more.',
].join('\\n')

// Paragraphs each wrapping onto a second line, whose first word is bold, code or a link.
const LEADS = [
  '**Restore** the cache before the first step, then run the tests again so that every package builds from the same lockfile.',
  '',
  '\`cargo test\` runs the whole workspace and stops at the first package whose build fails, which names the missing dependency.',
  '',
  '[The guide](https://example.com/guide) explains how the cache key is made from the lockfile and the toolchain version together.',
].join('\\n')

// A paragraph a side question asks about, its card set after it.
const ASKED = 'The cache key names the lockfile.'
const CARDED = [ASKED, '', 'Nothing else changes between the runs.'].join('\\n')

// A reply with a picture between two paragraphs, and a code block.
const PICTURED = [
  'See the chart:',
  '',
  '![Timeouts by hour](https://demi.test/chart.svg)',
  '',
  'Most uploads fail after 30 s.',
  '',
  '\`\`\`js',
  'const user = {',
  "  name: 'alex',",
  '}',
  '\`\`\`',
].join('\\n')

window.toolbarFrames = []
window.commented = []
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
      const rows = ref(null)
      const comments = ref()
      const waiting = ref([])
      // The transcript's own wiring (AgentMessageList): the comments layer's Comment and Ask.
      provideReplySelectionTools({
        selection: (reply, range, streaming) => comments.value?.selectionTools(reply, range, streaming) ?? [],
        picture: () => [],
      })
      const question = {
        id: 'q1', blockId: 'text-4', start: 0, end: ASKED.length, quote: ASKED, question: 'Which lockfile?', title: 'Cache keys',
        folded: false, askedAt: '2026-10-18T10:05:00Z', history: EMPTY_TRANSCRIPT, failures: {}, working: false,
      }
      const reply = (blockId, content) => h(AssistantTextBlock, { blockId, content, createdAt: '2026-10-18T10:00:00Z', showFooter: true, fork: async () => {}, share: { start: () => {} } })
      return () => [
        h(UserBlock, { content: [{ type: 'text', text: 'Why do my uploads keep failing on the train?' }] }),
        h('div', { ref: rows, style: 'position: relative' }, [
          reply('text-1', REPLY),
          reply('text-2', STEPS),
          reply('text-3', LEADS),
          reply('text-4', CARDED),
          reply('text-5', PICTURED),
          h(ReplyComments, {
            ref: comments,
            root: rows.value,
            sent: [],
            commenting: {
              get waiting() {
                return waiting.value
              },
              add: (comment) => {
                window.commented.push(comment.quote)
                waiting.value = [...waiting.value, comment]
              },
              edit: () => {},
              remove: () => {},
            },
            asking: { questions: [question], ask: () => {}, followUp: () => {}, retry: () => {}, fold: () => {}, read: () => {} },
          }),
        ]),
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
    const opened = await browser.newPage({ viewport: { width: 800, height: 2400 }, reducedMotion: 'reduce' })
    await opened.route('https://demi.test/', (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html><body></body>' }))
    // A picture, which the reply draws wider than the line before it.
    await opened.route('https://demi.test/chart.svg', (route) => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="80"><rect width="480" height="80" fill="#8ab"/></svg>' }))
    await opened.goto('https://demi.test/')
    await opened.addStyleTag({ content: css })
    await opened.addScriptTag({ content: script, type: 'module' })
    await opened.waitForFunction(() => typeof window.showTranscript === 'function')
    await opened.evaluate(() => window.showTranscript())
    await opened.locator('li').nth(1).waitFor()
    await opened.locator('[data-ask-card]').waitFor()
    await opened.locator('[data-reply-text="text-5"] img').waitFor()
    await opened.waitForFunction(() => document.querySelector('[data-reply-text="text-5"] img')!.getBoundingClientRect().width > 150)
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

  /** Two frames after the selection's last change, by which the toolbar shows or stays away. */
  const settled = (opened: Page) => opened.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  const toolRows = (opened: Page) => toolbar(opened).getByRole('button').allTextContents().then((rows) => rows.map((row) => row.trim()))

  /** Triple-clicks the word `word` of the block `selector` holds, which selects the block, as the user does. */
  async function tripleClick(opened: Page, selector: string, word: string): Promise<void> {
    const { start } = await ends(opened, selector, word, word)
    await opened.mouse.click(start.x, start.y, { clickCount: 3 })
  }

  test('a triple-click on a reply\'s last paragraph, its last list item or a paragraph before a side question\'s card shows Comment and Ask, which comment on its words alone', async () => {
    const cases = [
      // The browser ends these at the start of the timestamp under the reply.
      { selector: 'p', word: 'Smaller', quote: 'Smaller chunks fix both.' },
      { selector: 'li', word: 'suite', quote: 'Run the whole test suite once more.' },
      // This one at the start of the card's title.
      { selector: '[data-reply-text="text-4"] p', word: 'lockfile.', quote: 'The cache key names the lockfile.' },
    ]
    const opened = await transcript()
    for (const [index, { selector, word, quote }] of cases.entries()) {
      await tripleClick(opened, selector, word)
      expect((await selected(opened)).trim()).toBe(quote)
      await toolbar(opened).waitFor()
      expect(await toolRows(opened)).toEqual(['Comment', 'Ask'])
      await toolbar(opened).getByRole('button', { name: 'Comment' }).click()
      // The box opens under the words a frame later, its field focused.
      await opened.locator('[data-comment-field] textarea:focus').waitFor()
      await opened.keyboard.type('Why?')
      await opened.keyboard.press('Enter')
      await opened.waitForFunction((count) => window.commented.length === count, index + 1)
      expect((await opened.evaluate(() => window.commented)).at(-1)).toBe(quote)
    }
    await opened.close()
  })

  test('a selection that takes words of a side question\'s card shows no toolbar', async () => {
    const opened = await transcript()
    await drag(opened, '[data-reply-text="text-4"]', 'cache key', 'Cache')
    expect(await selected(opened)).toContain('Cache')
    await settled(opened)
    expect(await toolbar(opened).count()).toBe(0)
    await opened.close()
  })

  test('the toolbar stands over the middle of the selected lines when their first word is bold, code or a link', async () => {
    const opened = await transcript()
    for (const [from, to] of [['Restore', 'lockfile.'], ['cargo', 'dependency.'], ['The', 'together.']] as const) {
      // From the last word back to the first: a press on a link would drag the link.
      const { start, end } = await ends(opened, '[data-reply-text="text-3"] p', from, to)
      await opened.mouse.move(end.x + 3, end.y)
      await opened.mouse.down()
      await opened.mouse.move(start.x - 3, start.y, { steps: 8 })
      await opened.mouse.up()
      await toolbar(opened).waitFor()
      await settled(opened)
      // The lines as the browser draws the selection: the leftmost and rightmost of its boxes.
      const lines = await opened.evaluate(() => {
        const rects = [...window.getSelection()!.getRangeAt(0).getClientRects()].filter((rect) => rect.width > 0)
        return { left: Math.min(...rects.map((rect) => rect.left)), right: Math.max(...rects.map((rect) => rect.right)), count: new Set(rects.map((rect) => Math.round(rect.bottom))).size }
      })
      // The paragraph wraps, so its first box is its first word's alone.
      expect(lines.count).toBeGreaterThan(1)
      const box = (await toolbar(opened).boundingBox())!
      expect(Math.abs(box.x + box.width / 2 - (lines.left + lines.right) / 2)).toBeLessThan(4)
    }
    await opened.close()
  })

  /**
   * Selects in `selector`'s element from the character `from` to `to` of its text,
   * counted over its text nodes as the page draws them, as a drag over them does.
   */
  async function selectChars(opened: Page, selector: string, from: number, to: number): Promise<void> {
    await opened.evaluate(({ selector, from, to }) => {
      const el = document.querySelector(selector)!
      const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
      const range = document.createRange()
      let count = 0
      for (let node = walker.nextNode() as Text | null; node; node = walker.nextNode() as Text | null) {
        if (from >= count && from <= count + node.length && count + node.length > from)
          range.setStart(node, from - count)
        if (to > count && to <= count + node.length)
          range.setEnd(node, to - count)
        count += node.length
      }
      ;(document.activeElement as HTMLElement | null)?.blur()
      getSelection()!.removeAllRanges()
      getSelection()!.addRange(range)
    }, { selector, from, to })
  }

  /** The character offset of `text` in `selector`'s element's text, after it with `after`. */
  const at = (opened: Page, selector: string, text: string, after = false) =>
    opened.evaluate(({ selector, text, after }) => {
      const whole = document.querySelector(selector)!.textContent!
      const index = whole.indexOf(text)
      if (index < 0)
        throw new Error('No text ' + text)
      return after ? index + text.length : index
    }, { selector, text, after })

  /** Comments "Why?" on the selection through the toolbar; what the comment quotes. */
  async function commentOnSelection(opened: Page): Promise<string> {
    await toolbar(opened).getByRole('button', { name: 'Comment' }).click()
    await opened.locator('[data-comment-field] textarea:focus').waitFor()
    const before = await opened.evaluate(() => window.commented.length)
    await opened.keyboard.type('Why?')
    await opened.keyboard.press('Enter')
    await opened.waitForFunction((count) => window.commented.length > count, before)
    return opened.evaluate(() => window.commented.at(-1)!)
  }

  test('a selection that takes a character of the reply\'s timestamp is not the reply\'s text: its menu has Copy alone', async () => {
    const opened = await transcript()
    const reply = '[data-reply-text="text-1"]'
    await opened.evaluate((reply) => {
      const words = [...document.querySelectorAll(`${reply} p`)].at(-1)!.firstChild as Text
      const time = document.querySelector(`${reply} time`)!.firstChild as Text
      const range = document.createRange()
      range.setStart(words, words.data.indexOf('both'))
      range.setEnd(time, 1)
      ;(document.activeElement as HTMLElement | null)?.blur()
      getSelection()!.removeAllRanges()
      getSelection()!.addRange(range)
    }, reply)
    const { start } = await ends(opened, `${reply} p`, 'both', 'both')
    await opened.mouse.click(start.x, start.y, { button: 'right' })
    await opened.locator('[role="menu"]').waitFor()
    expect(await menuRows(opened)).toEqual(['Copy'])
    await opened.close()
  })

  test('a selection that ends after a picture covers it: the toolbar stands over the words and the picture, and Comment quotes the words', async () => {
    const opened = await transcript()
    await opened.evaluate(() => {
      const image = document.querySelector('[data-reply-text="text-5"] img')!
      const words = document.querySelector('[data-reply-text="text-5"] p')!.firstChild as Text
      const range = document.createRange()
      range.setStart(words, 0)
      range.setEndAfter(image)
      ;(document.activeElement as HTMLElement | null)?.blur()
      getSelection()!.removeAllRanges()
      getSelection()!.addRange(range)
    })
    await toolbar(opened).waitFor()
    await settled(opened)
    expect(await toolRows(opened)).toEqual(['Comment', 'Ask'])
    const center = await opened.evaluate(() => {
      const line = document.createRange()
      line.selectNodeContents(document.querySelector('[data-reply-text="text-5"] p')!)
      const words = line.getBoundingClientRect()
      const image = document.querySelector('[data-reply-text="text-5"] img')!.getBoundingClientRect()
      // The words' line is narrower than the picture, so the two have another middle.
      if (words.right > image.right - 40)
        throw new Error('The words are as wide as the picture')
      return (Math.min(words.left, image.left) + Math.max(image.right, words.right)) / 2
    })
    const box = (await toolbar(opened).boundingBox())!
    expect(Math.abs(box.x + box.width / 2 - center)).toBeLessThan(4)
    expect(await commentOnSelection(opened)).toBe('See the chart:')
    await opened.close()
  })

  test('selected whitespace is kept: a code line\'s break and indent, and the space after a bold word', async () => {
    const opened = await transcript()
    const code = '[data-reply-text="text-5"] pre code'
    await opened.locator(code).getByText('name').waitFor()
    await selectChars(opened, code, await at(opened, code, '{', true), await at(opened, code, 'name', true))
    await toolbar(opened).waitFor()
    expect(await commentOnSelection(opened)).toBe('\n  name')
    const bold = '[data-reply-text="text-3"] p'
    await selectChars(opened, bold, 0, (await at(opened, bold, 'Restore', true)) + 1)
    await toolbar(opened).waitFor()
    expect(await commentOnSelection(opened)).toBe('Restore ')
    await opened.close()
  })

  test('a comment across two paragraphs quotes them as Copy copies them, a blank line between', async () => {
    const opened = await transcript()
    const body = '[data-reply-text="text-3"] .markdown-body'
    await selectChars(opened, body, await at(opened, body, 'same lockfile.'), await at(opened, body, 'cargo', true))
    await toolbar(opened).waitFor()
    expect(await commentOnSelection(opened)).toBe('same lockfile.\n\ncargo')
    await opened.close()
  })
})
