import { expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'
import {
  alignShown,
  closeOpenInlineMarkdown,
  holdIncompleteMarkdown,
  nextStepEnd,
  renderedLength,
  segmentStreamUnits,
  startOfLast,
  STREAM_PACE,
  stepInterval,
  wrapSteps,
} from '../stream-reveal'

// Cost: pure, a few milliseconds for the file.

test('segments cover the source, including CJK and spaces', () => {
  const text = 'Hello, world. 输入壳是 44px。'
  expect(segmentStreamUnits(text).join('')).toBe(text)
})

test('alignShown keeps a matching prefix and snaps a rewrite', () => {
  expect(alignShown('输入壳', '输入壳是胶囊')).toBe('输入壳')
  expect(alignShown('输入壳是胶囊', '输入壳')).toBe('输入壳')
  expect(alignShown('abc', 'xyz')).toBe('')
  expect(alignShown('same', 'same')).toBe('same')
})

const settled = (text: string) => holdIncompleteMarkdown(text).visible

test('a step shows about 22 characters and ends on a word, a CJK word too', () => {
  const english = 'The login test fails because it still expects the old cookie name.'
  const end = nextStepEnd(english, 0, english.length, settled)
  expect(english.slice(0, end)).toBe('The login test fails ')
  const chinese = '登录测试失败，是因为它还在找旧的 cookie 名。上周把辅助函数改了名。'
  const cut = nextStepEnd(chinese, 0, chinese.length, settled)
  const boundaries = new Set(segmentStreamUnits(chinese).map((_, index, units) => units.slice(0, index + 1).join('').length))
  expect(boundaries.has(cut)).toBe(true)
  expect(cut).toBeLessThanOrEqual(STREAM_PACE.stepChars)
})

test('a step never leaves a link half shown: it stops before it, or takes it whole when it starts the step', () => {
  const text = 'It reads [the cookie helper](src/auth/cookie.ts) first.'
  const first = nextStepEnd(text, 0, text.length, settled)
  expect(text.slice(0, first)).toBe('It reads ')
  const second = nextStepEnd(text, first, text.length, settled)
  expect(text.slice(first, second)).toBe('[the cookie helper](src/auth/cookie.ts)')
})

test('steps come faster as text waits, within their bounds', () => {
  expect(stepInterval(1)).toBe(STREAM_PACE.maxStepMs)
  expect(stepInterval(10_000)).toBe(STREAM_PACE.minStepMs)
  // 242 characters waiting: 22 at a time catch up in about the target lag.
  expect(stepInterval(242)).toBe(50)
})

test('a step\'s rendered length leaves out its marks and line breaks, and finds where it starts in the render', () => {
  expect(renderedLength('**bold** and `code`\n- item')).toBe('bold and codeitem'.length)
  expect(renderedLength('```ts\nconst a = 1\n```')).toBe('const a = 1'.length)
  expect(startOfLast('ab\ncd', 3)).toBe(1)
})

test('holdIncompleteMarkdown parks unmatched markers and links', () => {
  expect(holdIncompleteMarkdown('hello **')).toEqual(
    { visible: 'hello ', held: '**' }
  )
  expect(holdIncompleteMarkdown('hello **bold**')).toEqual(
    { visible: 'hello **bold**', held: '' }
  )
  expect(holdIncompleteMarkdown('see `')).toEqual({ visible: 'see ', held: '`' })
  expect(holdIncompleteMarkdown('go [docs')).toEqual(
    { visible: 'go ', held: '[docs' }
  )
  expect(holdIncompleteMarkdown('go [docs](https://ex')).toEqual(
    {
      visible: 'go ',
      held: '[docs](https://ex'
    }
  )
})

test('closeOpenInlineMarkdown closes what the last paragraph left open', () => {
  expect(closeOpenInlineMarkdown('2. **Work')).toBe('**')
  expect(closeOpenInlineMarkdown('2. **Work through** one')).toBe('')
  expect(closeOpenInlineMarkdown('a *b **c')).toBe('***')
  expect(closeOpenInlineMarkdown('use `code')).toBe('`')
  expect(closeOpenInlineMarkdown('use `**not bold')).toBe('`')
  expect(closeOpenInlineMarkdown('done **here**\n\n* item\n* other')).toBe('')
  expect(closeOpenInlineMarkdown('2 * 3 = 6')).toBe('')
  expect(closeOpenInlineMarkdown('```ts\nconst a = 1')).toBe('')
  expect(closeOpenInlineMarkdown('~~gone')).toBe('~~')
})

// A step's fade in a real Chromium, since happy-dom computes no animated
// style: about 1 s, the browser's start. The flash lives in what the engine
// draws between a step being shown and its fade starting on the timeline.
test.skipIf(!existsSync(chromium.executablePath()))(
  'a step\'s new text shows transparent until its fade starts, never solid first',
  async () => {
    const build = await Bun.build({
      entrypoints: ['/entry.ts'],
      files: { '/entry.ts': `import { wrapSteps } from ${JSON.stringify(fileURLToPath(new URL('../stream-reveal.ts', import.meta.url)))}\nObject.assign(window, { wrapSteps })` },
      target: 'browser',
    })
    const browser = await chromium.launch()
    try {
      const page = await browser.newPage()
      await page.addScriptTag({ content: await build.outputs[0]!.text() })
      const opacity = await page.evaluate(() => new Promise<number>((resolve) => {
        const wrap: typeof wrapSteps = Reflect.get(window, 'wrapSteps')
        // A step is shown by the stream's timer, a task after a frame began,
        // so the timeline still reads that frame's time, before the step's.
        requestAnimationFrame(() => setTimeout(() => {
          const message = document.createElement('p')
          message.textContent = 'Seen before. Just shown.'
          document.body.append(message)
          wrap(message, message.textContent, [{ start: 13, end: 24, at: performance.now() }])
          resolve(Number(getComputedStyle(message.querySelector('.stream-ink')!).opacity))
        }))
      }))
      expect(opacity).toBe(0)
    } finally {
      await browser.close()
    }
  },
)
