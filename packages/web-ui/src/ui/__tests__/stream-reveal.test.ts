import { expect, test } from 'bun:test'
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
