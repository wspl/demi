import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { existsSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { chromium, type Browser, type Page } from 'playwright'

// What a waiting formula hides (`docs/decisions/0033-a-region-draws-once.md`):
// in a message that has not shown yet, the formula and everything after it that
// has not shown, up to the transcript's later rows; in one that has shown, the
// formula alone. The rules are the stylesheet's own, on a page of the markup
// the message and the transcript give. In a real Chromium, for `:has`: about
// 1 s for the browser's start, shared by the tests.

/** The stylesheet's rules for `.math-waiting`, whole. */
function waitingRules(): string {
  const css = readFileSync(fileURLToPath(new URL('../base.css', import.meta.url)), 'utf8')
  const start = css.indexOf('.markdown-body .math-waiting {')
  const end = css.indexOf('/* A code block, table or equation that scrolls sideways')
  if (start === -1 || end === -1)
    throw new Error('the rules for a waiting formula are not where the test looks')
  return css.slice(start, end)
}

describe.skipIf(!existsSync(chromium.executablePath()))('in Chromium', () => {
  let browser: Browser
  beforeAll(async () => {
    browser = await chromium.launch()
  })
  afterAll(async () => {
    await browser?.close()
  })

  /** A page of two transcript rows, each a message and its footer, the first shown or not. */
  async function transcript(shown: boolean): Promise<Page> {
    const page = await browser.newPage()
    await page.setContent(`<style>${waitingRules()}</style>
      <div data-row-key="a">
        <div class="markdown-body" ${shown ? 'data-shown' : ''}>
          <p id="above">Words above.</p>
          <p id="holds">Words and <span class="math math-inline math-waiting">x^2</span> in a line.</p>
          <p id="below">Words below.</p>
        </div>
        <div id="footer">Footer</div>
      </div>
      <div data-row-key="b"><p id="later">A later row.</p></div>`)
    return page
  }
  /** What of the page is hidden: the ids of the elements. */
  async function hidden(page: Page): Promise<string[]> {
    return page.evaluate(() => ['above', 'holds', 'below', 'footer', 'later'].filter((id) => getComputedStyle(document.getElementById(id)!).visibility === 'hidden'))
  }

  test('a message that has not shown hides its formula, what follows it, its footer and the later rows, and shows what is above', async () => {
    const page = await transcript(false)
    expect(await hidden(page)).toEqual(['holds', 'below', 'footer', 'later'])
    await page.close()
  })

  test('a message that has shown hides its formula alone', async () => {
    const page = await transcript(true)
    expect(await hidden(page)).toEqual([])
    expect(await page.evaluate(() => getComputedStyle(document.querySelector('.math-waiting')!).visibility)).toBe('hidden')
    await page.close()
  })
})
