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
// - The selection's field steps aside while the menu is open and comes back
//   after it.
// - A selection that went leaves the message's menu.
// - The field appears once a drag ends, in its place.
// - A triple-click on a reply's last paragraph, its last list item, or a paragraph
//   before a side question's card shows Comment and Ask, though the browser ends
//   the selection at the timestamp or the card under it; a selection that takes
//   the card's words shows none.
// - The field stands over the middle of the selected lines, whatever their
//   first word is drawn in.
// - A selection that takes a character of the timestamp is no longer the
//   reply's text; one that ends after a picture covers the picture.
// - A comment quotes the selection as Copy copies it: whitespace selected is kept,
//   and two paragraphs have a blank line between them.
// - The comment field has the focus as it opens, so a key typed after the
//   selection starts in it, and the words selected stay for Copy and for the
//   menu. Return presses the chosen action and Command-Return sends; with no
//   words Return shakes it and Send is dimmed. A menu's Ask or a picture's Comment
//   opens the same field.
// - On a phone the field opens in the bottom sheet, which has no scrim, takes no
//   focus until tapped, follows the selection's handles, and closes on a swipe
//   down or a tap that clears the selection.
//
// Selections, hit tests and layout are the browser's, so this runs the product's
// reply, user message, comments layer, ContextMenu and comment field in a real
// Chromium with the stylesheet Tailwind builds; happy-dom has none of them.
//
// Cost: about 7 s: the page's and the stylesheet's builds and the browser's start,
// shared, and a page per test.

declare global {
  interface Window {
    /**
     * Mounts the page's menu and field, a user's message, replies with the
     * comments layer over them, one with a side question's card, and a message
     * field; at a phone's width the replies fill the page.
     */
    showTranscript(): void
    /** The quotes of the comments saved, in order. */
    commented: string[]
    /** The words of the comments sent alone now, and of the side questions asked, in order. */
    sentNow: string[]
    asked: string[]
    /** The field as each frame drew it: null when it showed nothing, else where it stood. */
    fieldFrames: ({ top: number; left: number } | null)[]
  }
}

const PAGE_ENTRY = `
import { createApp, defineComponent, h } from 'vue'
import ContextMenu from '../ContextMenu.vue'
import Popover from '../Popover.vue'
import { appOverlayStore } from '../../overlay/appOverlay'
import SelectionFieldHost from '../../agent/comments/SelectionFieldHost.vue'
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

window.fieldFrames = []
window.commented = []
window.sentNow = []
window.asked = []
const sample = () => {
  const layer = document.querySelector('[data-selection-field-layer]')
  const box = layer && getComputedStyle(layer).visibility !== 'hidden' ? layer.getBoundingClientRect() : null
  window.fieldFrames.push(box && { top: Math.round(box.top), left: Math.round(box.left) })
  requestAnimationFrame(sample)
}
requestAnimationFrame(sample)

window.showTranscript = () => {
  const root = document.createElement('div')
  // At a phone's width the replies scroll inside a page of the screen's size, as the app's do.
  document.body.style.height = '100dvh'
  root.style.cssText = window.innerWidth < 768 ? 'height:100dvh;overflow-y:auto;padding:80px 0 0' : 'width:640px;padding:80px 0 0;margin-left:160px'
  document.body.append(root)
  if (window.innerWidth < 768) {
    // The message field of the app's page, with a chip: taller than the sheet's own content, which the sheet must cover.
    const composer = document.createElement('div')
    composer.dataset.composerShell = ''
    composer.style.cssText = 'position:fixed;left:8px;right:8px;bottom:24px;height:200px;border-radius:24px;background:#eee'
    document.body.append(composer)
  }
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
        field: (reply, range, streaming) => comments.value?.selectionField(reply, range, streaming) ?? null,
        picture: (reply, image, streaming) => comments.value?.pictureTools(reply, image, streaming) ?? [],
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
              sendNow: (comment) => window.sentNow.push(comment.text),
            },
            asking: { questions: [question], ask: (asked) => window.asked.push(asked.question), followUp: () => {}, retry: () => {}, fold: () => {}, read: () => {} },
          }),
        ]),
        // The message field, which has the focus as the page opens, as the composer does.
        h('textarea', { 'aria-label': 'Message', autofocus: true }),
        // A closed menu by the field, as the composer's are, which keeps the page's focus tracking (focusReturn.ts) listening.
        h(Popover, { isOpen: false, overlayStore: appOverlayStore }),
        h(ContextMenu),
        h(SelectionFieldHost),
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

  async function transcript(phone = false): Promise<Page> {
    const opened = await browser.newPage({ viewport: phone ? { width: 390, height: 844 } : { width: 800, height: 2400 }, reducedMotion: 'reduce', hasTouch: phone })
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
    await opened.waitForFunction(() => document.querySelector('[data-reply-text="text-5"] img')!.getBoundingClientRect().width > 50)
    if (!phone)
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

  /**
   * The words selected: the page's selection, or, once the comment field has the
   * focus, which ended it, what its Copy copies, else the words it paints as
   * selected (a click on a side question's words moves the focus to its card).
   */
  const selected = (opened: Page) => opened.evaluate(() => {
    const native = window.getSelection()?.toString() ?? ''
    if (native !== '' || !document.querySelector('[data-selection-field]'))
      return native
    if (document.activeElement?.closest('[data-selection-field]')) {
      // The field's own Copy handler, asked as the browser asks it.
      const copy = new ClipboardEvent('copy', { clipboardData: new DataTransfer(), bubbles: true, cancelable: true })
      document.activeElement.dispatchEvent(copy)
      return copy.clipboardData?.getData('text/plain') ?? ''
    }
    const painted = [...CSS.highlights.get('comment-selection')!][0] as StaticRange
    const words = document.createRange()
    words.setStart(painted.startContainer, painted.startOffset)
    words.setEnd(painted.endContainer, painted.endOffset)
    return words.toString()
  })
  /**
   * The boxes of the words selected, where the page paints them: the field that
   * has the focus ended the page's selection and paints its words as selected.
   */
  const heldRects = (opened: Page) => opened.evaluate(() => {
    const painted = [...CSS.highlights.get('comment-selection')!][0] as StaticRange
    const words = document.createRange()
    words.setStart(painted.startContainer, painted.startOffset)
    words.setEnd(painted.endContainer, painted.endOffset)
    return [...words.getClientRects()].filter((rect) => rect.width > 0).map(({ left, right, top, bottom }) => ({ left, right, top, bottom }))
  })
  const menuRows = (opened: Page) => opened.locator('[role="menuitem"]').allTextContents().then((rows) => rows.map((row) => row.trim()))
  const field = (opened: Page) => opened.locator('[data-selection-field-layer]')

  test('a right-click inside a selection across two list items keeps it and opens its menu alone, the field hidden until Escape', async () => {
    const opened = await transcript()
    await drag(opened, 'li', '服务器回应', '传输时间')
    const words = await selected(opened)
    expect(words).toContain('服务器回应')
    expect(words).toContain('传输时间')
    await field(opened).waitFor()
    const { start } = await ends(opened, 'li', '放弃了', '放弃了')
    await opened.mouse.click(start.x, start.y, { button: 'right' })
    await opened.locator('[role="menu"]').waitFor()
    expect(await selected(opened)).toBe(words)
    expect(await menuRows(opened)).toEqual(['Copy', 'Comment', 'Ask', 'Send to Conversation'])
    await field(opened).waitFor({ state: 'hidden' })
    await opened.keyboard.press('Escape')
    await opened.locator('[role="menu"]').waitFor({ state: 'detached' })
    expect(await selected(opened)).toBe(words)
    await field(opened).waitFor()
    await opened.close()
  })

  test('a right-click in the gap between two selected list items opens the selection menu', async () => {
    const opened = await transcript()
    await drag(opened, 'li', '服务器回应', '传输时间')
    await opened.locator('[data-selection-field] textarea:focus').waitFor()
    const words = await selected(opened)
    // Halfway between the first item's last line and the second's first, where no text is.
    const gap = await opened.evaluate(() => {
      const [first, second] = [...document.querySelectorAll('li')].map((li) => li.getBoundingClientRect())
      return { x: first!.left + 40, y: (first!.bottom + second!.top) / 2 }
    })
    const under = (await heldRects(opened)).some((rect) => gap.x >= rect.left && gap.x <= rect.right && gap.y >= rect.top && gap.y <= rect.bottom)
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
    await field(opened).waitFor()
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

  test('a drag-selection\'s field appears once, after the release, where it stays', async () => {
    const opened = await transcript()
    const { start, end } = await ends(opened, 'li', '服务器回应', '传输时间')
    await opened.mouse.move(start.x - 3, start.y)
    await opened.mouse.down()
    await opened.mouse.move(end.x + 3, end.y, { steps: 8 })
    const released = await opened.evaluate(() => window.fieldFrames.length)
    await opened.mouse.up()
    await field(opened).waitFor()
    // A few frames after it stands, to see it neither move nor blink.
    await opened.evaluate(() => new Promise((resolve) => {
      let left = 4
      const next = () => (--left === 0 ? resolve(null) : requestAnimationFrame(next))
      requestAnimationFrame(next)
    }))
    const frames = await opened.evaluate(() => window.fieldFrames)
    // Nothing drew it while the drag held.
    expect(frames.slice(0, released).every((frame) => frame === null)).toBe(true)
    const after = frames.slice(released)
    const first = after.findIndex((frame) => frame !== null)
    const shown = after.slice(first)
    // Once drawn it stays, in the one place it first stood.
    expect(shown.every((frame) => frame !== null && frame.top === shown[0]!.top && frame.left === shown[0]!.left)).toBe(true)
    await opened.close()
  })

  /** Two frames after the selection's last change, by which the field shows or stays away. */
  const settled = (opened: Page) => opened.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  const toolRows = (opened: Page) => field(opened).getByRole('button').allTextContents().then((rows) => rows.map((row) => row.trim()).filter((row) => row !== ''))

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
      await field(opened).waitFor()
      expect(await toolRows(opened)).toEqual(['Send', 'Ask', 'Comment'])
      // The field has the focus once the click has ended, even where the click focuses a card; the first key goes into it.
      await opened.locator('[data-selection-field] textarea:focus').waitFor()
      await opened.keyboard.type('Why?')
      expect(await fieldText(opened).inputValue()).toBe('Why?')
      await opened.keyboard.press('Enter')
      await opened.waitForFunction((count) => window.commented.length === count, index + 1)
      expect((await opened.evaluate(() => window.commented)).at(-1)).toBe(quote)
    }
    await opened.close()
  })

  test('a selection that takes words of a side question\'s card shows no field', async () => {
    const opened = await transcript()
    await drag(opened, '[data-reply-text="text-4"]', 'cache key', 'Cache')
    expect(await selected(opened)).toContain('Cache')
    await settled(opened)
    expect(await field(opened).count()).toBe(0)
    await opened.close()
  })

  test('the field stands over the middle of the selected lines when their first word is bold, code or a link', async () => {
    const opened = await transcript()
    for (const [from, to] of [['Restore', 'lockfile.'], ['cargo', 'dependency.'], ['The', 'together.']] as const) {
      // From the last word back to the first: a press on a link would drag the link.
      const { start, end } = await ends(opened, '[data-reply-text="text-3"] p', from, to)
      await opened.mouse.move(end.x + 3, end.y)
      await opened.mouse.down()
      await opened.mouse.move(start.x - 3, start.y, { steps: 8 })
      await opened.mouse.up()
      await field(opened).waitFor()
      await settled(opened)
      // The lines as the browser draws the selection: the leftmost and rightmost of its boxes.
      const rects = await heldRects(opened)
      const lines = { left: Math.min(...rects.map((rect) => rect.left)), right: Math.max(...rects.map((rect) => rect.right)), count: new Set(rects.map((rect) => Math.round(rect.bottom))).size }
      // The paragraph wraps, so its first box is its first word's alone.
      expect(lines.count).toBeGreaterThan(1)
      const box = (await field(opened).boundingBox())!
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

  /** Comments "Why?" on the selection through the field; what the comment quotes. */
  async function commentOnSelection(opened: Page): Promise<string> {
    await opened.locator('[data-selection-field] textarea:focus').waitFor()
    const before = await opened.evaluate(() => window.commented.length)
    await opened.keyboard.type('Why?')
    await opened.locator('[data-selection-field] textarea:focus').waitFor()
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

  test('a selection that ends after a picture covers it: the field stands over the words and the picture, and Comment quotes the words', async () => {
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
    await field(opened).waitFor()
    await settled(opened)
    expect(await toolRows(opened)).toEqual(['Send', 'Ask', 'Comment'])
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
    const box = (await field(opened).boundingBox())!
    expect(Math.abs(box.x + box.width / 2 - center)).toBeLessThan(4)
    expect(await commentOnSelection(opened)).toBe('See the chart:')
    await opened.close()
  })

  /** The comment field's own parts. */
  const fieldBox = (opened: Page) => opened.locator('[data-selection-field]')
  const fieldText = (opened: Page) => fieldBox(opened).locator('textarea')

  test('the field has the focus as it opens, so the first key typed starts in it, while the words selected stay for Copy', async () => {
    const opened = await transcript()
    await drag(opened, 'li', '服务器回应', '传输时间')
    await field(opened).waitFor()
    // A key typed with an input method on starts a composition only in a focused field.
    await opened.waitForFunction(() => document.activeElement?.closest('[data-selection-field]') !== null)
    expect(await selected(opened)).toContain('服务器回应')
    await opened.keyboard.type('Why?')
    expect(await fieldText(opened).inputValue()).toBe('Why?')
    await opened.close()
  })

  test('Return with no words shakes the field and Send is dimmed; Return comments, and Command-Return sends the comment alone', async () => {
    const opened = await transcript()
    await drag(opened, 'li', '服务器回应', '传输时间')
    await field(opened).waitFor()
    await opened.locator('[data-selection-field] textarea:focus').waitFor()
    const send = fieldBox(opened).getByRole('button', { name: 'Send' })
    expect(await send.getAttribute('aria-disabled')).toBe('true')
    await opened.keyboard.press('Enter')
    await opened.waitForFunction(() => document.querySelector('[data-selection-field]')?.classList.contains('selection-field-shake'))
    await fieldBox(opened).getByRole('button', { name: 'Ask' }).click()
    expect(await opened.evaluate(() => [window.commented.length, window.asked.length, window.sentNow.length])).toEqual([0, 0, 0])
    await opened.keyboard.type('Why?')
    expect(await send.getAttribute('aria-disabled')).toBeNull()
    await opened.keyboard.press('ControlOrMeta+Enter')
    await opened.waitForFunction(() => window.sentNow.length === 1)
    expect(await opened.evaluate(() => [window.commented.length, window.sentNow[0]])).toEqual([0, 'Why?'])
    await field(opened).waitFor({ state: 'detached' })
    await opened.close()
  })

  test('Ask in a selection\'s menu opens the same field with Ask chosen, so Return asks', async () => {
    const opened = await transcript()
    await drag(opened, 'li', '服务器回应', '传输时间')
    await field(opened).waitFor()
    await opened.locator('[data-selection-field] textarea:focus').waitFor()
    await opened.keyboard.type('Why?')
    const { start } = await ends(opened, 'li', '放弃了', '放弃了')
    await opened.mouse.click(start.x, start.y, { button: 'right' })
    await opened.locator('[role="menu"]').waitFor()
    await opened.getByRole('menuitem', { name: 'Ask' }).click()
    await opened.locator('[role="menu"]').waitFor({ state: 'detached' })
    // The field comes back with what was typed, and the focus.
    await field(opened).waitFor()
    await opened.waitForFunction(() => document.activeElement?.closest('[data-selection-field]') !== null)
    expect(await fieldText(opened).inputValue()).toBe('Why?')
    await opened.keyboard.press('Enter')
    await opened.waitForFunction(() => window.asked.length === 1)
    expect(await opened.evaluate(() => [window.asked[0], window.commented.length])).toEqual(['Why?', 0])
    await opened.close()
  })

  test('Comment in a picture\'s menu opens the same field, outlining the picture, and keeps the picture with the comment', async () => {
    const opened = await transcript()
    const image = opened.locator('[data-reply-text="text-5"] img')
    const box = (await image.boundingBox())!
    await opened.mouse.click(box.x + box.width / 2, box.y + box.height / 2, { button: 'right' })
    await opened.locator('[role="menu"]').waitFor()
    await opened.getByRole('menuitem', { name: 'Comment' }).click()
    await opened.locator('[role="menu"]').waitFor({ state: 'detached' })
    await field(opened).waitFor()
    await opened.waitForFunction(() => document.activeElement?.closest('[data-selection-field]') !== null)
    await opened.locator('[data-picture-outline="comment-selection"]').waitFor()
    await opened.keyboard.type('Which hours?')
    await opened.keyboard.press('Enter')
    await opened.waitForFunction(() => window.commented.length === 1)
    expect(await opened.evaluate(() => window.commented[0])).toBe('Timeouts by hour')
    await field(opened).waitFor({ state: 'detached' })
    await opened.close()
  })

  test('on a phone the field opens in a bottom sheet without a scrim, follows the handles, comments on the selection as it is, and closes on a swipe down or a tap that clears the selection', async () => {
    const opened = await transcript(true)
    const select = (from: number, to: number) => opened.evaluate(([from, to]) => {
      const words = document.querySelector('[data-reply-text="text-1"] p')!.firstChild as Text
      const range = document.createRange()
      range.setStart(words, from)
      range.setEnd(words, to)
      // What the system's handles do: they set the page's selection.
      window.getSelection()!.setBaseAndExtent(words, from, words, to)
    }, [from, to])
    const sheet = opened.locator('[role="dialog"]')
    await select(4, 8)
    await sheet.locator('[data-selection-field]').waitFor()
    await opened.waitForFunction(() => document.querySelector('[role="dialog"]')!.getBoundingClientRect().height >= 200)
    const bounds = (await sheet.boundingBox())!
    expect(Math.round(bounds.y + bounds.height)).toBe(844)
    expect(Math.round(bounds.width)).toBe(390)
    // The sheet stands over the message field entirely, taller than its own content needs.
    const shell = (await opened.locator('[data-composer-shell]').boundingBox())!
    expect(bounds.y).toBeLessThanOrEqual(shell.y)
    // No scrim: the page stays as it was.
    expect(await opened.locator('.sheet-scrim').count()).toBe(0)
    expect(await opened.evaluate(() => document.activeElement?.closest('[data-selection-field]') === null)).toBe(true)
    const quoted = () => sheet.locator('p.line-clamp-1').first().textContent()
    const reply = () => opened.evaluate(() => document.querySelector('[data-reply-text="text-1"] p')!.textContent ?? '')
    expect(await quoted()).toBe((await reply()).slice(4, 8))
    // A handle extends the selection: the same field follows, its quote with it, and the comment quotes the selection as it is then.
    await select(4, 20)
    await settled(opened)
    expect(await quoted()).toBe((await reply()).slice(4, 20).replace(/\s+/g, ' ').trim())
    expect(await opened.locator('[data-selection-field]').count()).toBe(1)
    const text = (await fieldText(opened).boundingBox())!
    await opened.touchscreen.tap(text.x + 20, text.y + 8)
    await opened.keyboard.type('Why?')
    expect(await fieldText(opened).inputValue()).toBe('Why?')
    expect(await opened.evaluate(() => CSS.highlights.has('comment-selection'))).toBe(true)
    await opened.keyboard.press('Enter')
    await field(opened).waitFor({ state: 'detached' })
    const quote = await opened.evaluate(() => window.commented.at(-1)!)
    expect(quote).toBe(await opened.evaluate(() => (document.querySelector('[data-reply-text="text-1"] p')!.textContent ?? '').slice(4, 20)))
    // A selection over a picture quotes it as the media text.
    await opened.evaluate(() => {
      const image = document.querySelector('[data-reply-text="text-5"] img')!
      const words = document.querySelector('[data-reply-text="text-5"] p')!.firstChild as Text
      const range = document.createRange()
      range.setStart(words, 4)
      range.setEndAfter(image)
      window.getSelection()!.removeAllRanges()
      window.getSelection()!.addRange(range)
    })
    await settled(opened)
    await sheet.locator('[data-selection-field]').waitFor()
    expect(await quoted()).toBe('the chart: [Media]')
    await opened.evaluate(() => window.getSelection()!.removeAllRanges())
    await field(opened).waitFor({ state: 'detached' })
    // A tap on the page that clears the selection closes it and drops what was typed.
    await select(4, 8)
    await sheet.locator('[data-selection-field]').waitFor()
    await opened.evaluate(() => window.getSelection()!.removeAllRanges())
    await field(opened).waitFor({ state: 'detached' })
    await select(4, 8)
    await sheet.locator('[data-selection-field]').waitFor()
    expect(await fieldText(opened).inputValue()).toBe('')
    // With the field written in, a tap on the page elsewhere closes it too.
    const again = (await fieldText(opened).boundingBox())!
    await opened.touchscreen.tap(again.x + 20, again.y + 8)
    await opened.keyboard.type('Why?')
    await opened.touchscreen.tap(200, 300)
    await field(opened).waitFor({ state: 'detached' })
    // A swipe down on the sheet closes it.
    await select(4, 8)
    await sheet.locator('[data-selection-field]').waitFor()
    const grab = (await opened.locator('[data-sheet-handle]').boundingBox())!
    const session = await opened.context().newCDPSession(opened)
    const touch = (type: 'touchStart' | 'touchMove' | 'touchEnd', y: number) => session.send('Input.dispatchTouchEvent', { type, touchPoints: type === 'touchEnd' ? [] : [{ x: grab.x + grab.width / 2, y }] })
    await touch('touchStart', grab.y + 8)
    for (const dy of [30, 60, 90, 120]) await touch('touchMove', grab.y + 8 + dy)
    await touch('touchEnd', 0)
    await field(opened).waitFor({ state: 'detached' })
    await opened.close()
  })

  test('selected whitespace is kept: a code line\'s break and indent, and the space after a bold word', async () => {
    const opened = await transcript()
    const code = '[data-reply-text="text-5"] pre code'
    await opened.locator(code).getByText('name').waitFor()
    await selectChars(opened, code, await at(opened, code, '{', true), await at(opened, code, 'name', true))
    await field(opened).waitFor()
    expect(await commentOnSelection(opened)).toBe('\n  name')
    const bold = '[data-reply-text="text-3"] p'
    await selectChars(opened, bold, 0, (await at(opened, bold, 'Restore', true)) + 1)
    await field(opened).waitFor()
    expect(await commentOnSelection(opened)).toBe('Restore ')
    await opened.close()
  })

  test('a comment across two paragraphs quotes them as Copy copies them, a blank line between', async () => {
    const opened = await transcript()
    const body = '[data-reply-text="text-3"] .markdown-body'
    await selectChars(opened, body, await at(opened, body, 'same lockfile.'), await at(opened, body, 'cargo', true))
    await field(opened).waitFor()
    expect(await commentOnSelection(opened)).toBe('same lockfile.\n\ncargo')
    await opened.close()
  })
})
