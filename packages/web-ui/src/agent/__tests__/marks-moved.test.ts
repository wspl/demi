import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { chromium, type Browser } from 'playwright'

// The marks on a reply's words stay painted when the page moves the list
// holding them, as a window opening beside the transcript moves its
// ancestors: a move collapses every live Range inside what moved and changes
// nothing the comments layer watches, so it does not measure again. Ranges and
// the Custom Highlight API are the browser's, so this runs in a real Chromium;
// happy-dom has neither. About 2 s: the page's build and the browser's start.

declare global {
  interface Window {
    /**
     * Shows a reply and publishes a side question's mark on `quote`, inside a
     * wrapper the test moves.
     */
    markReply(quote: string): void
    /** The words each painted `ask` mark covers now. */
    painted(): string[]
    /**
     * Moves the reply's wrapper in the page, as a sibling inserted before it
     * does.
     */
    moveReply(): void
  }
}

const PAGE_ENTRY = `
import { publishMarks } from '../comments/highlights'

window.markReply = (quote) => {
  const wrapper = document.createElement('div')
  wrapper.id = 'wrapper'
  wrapper.innerHTML = '<p>A deploy updates your application, but the CDN keeps the old copy.</p>'
  document.body.append(wrapper)
  const text = wrapper.querySelector('p').firstChild
  const range = document.createRange()
  range.setStart(text, text.data.indexOf(quote))
  range.setEnd(text, text.data.indexOf(quote) + quote.length)
  publishMarks(Symbol('reply'), { 'comment': [], 'comment-sent': [], 'comment-active': [], 'comment-selection': [], 'ask': [range], 'ask-active': [] })
}

window.painted = () => [...(CSS.highlights.get('ask') ?? [])].map((range) => {
  const live = document.createRange()
  live.setStart(range.startContainer, range.startOffset)
  live.setEnd(range.endContainer, range.endOffset)
  return live.toString()
})

window.moveReply = () => {
  const wrapper = document.getElementById('wrapper')
  document.body.insertBefore(document.createElement('div'), wrapper)
  document.body.append(wrapper)
}
`

describe.skipIf(!existsSync(chromium.executablePath()))('in Chromium', () => {
  let browser: Browser
  let script: string
  beforeAll(async () => {
    const entry = fileURLToPath(new URL('./marks-moved-entry.ts', import.meta.url))
    const build = await Bun.build({ entrypoints: [entry], files: { [entry]: PAGE_ENTRY }, target: 'browser' })
    if (!build.success)
      throw new AggregateError(build.logs, 'The page did not build')
    script = await build.outputs[0]!.text()
    browser = await chromium.launch()
  })
  afterAll(async () => {
    await browser?.close()
  })

  test('a mark keeps its words when the page moves the list holding them', async () => {
    const page = await browser.newPage()
    await page.route('https://demi.test/', (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html><body></body>' }))
    await page.goto('https://demi.test/')
    await page.addScriptTag({ content: script, type: 'module' })
    await page.waitForFunction(() => typeof window.markReply === 'function')
    await page.evaluate(() => window.markReply('deploy'))
    expect(await page.evaluate(() => window.painted())).toEqual(['deploy'])
    await page.evaluate(() => window.moveReply())
    expect(await page.evaluate(() => window.painted())).toEqual(['deploy'])
    await page.close()
  })
})
