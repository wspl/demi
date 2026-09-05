import { expect, test } from 'bun:test'
import {
  alignShown,
  holdIncompleteMarkdown,
  nextShownText,
  segmentStreamUnits,
  STREAM_REVEAL,
  visibleFrontierLength,
} from '../stream-reveal'

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

test('nextShownText grows on word boundaries when the gap is small', () => {
  const target = 'Hello world'
  const next = nextShownText('', target, 16, STREAM_REVEAL.maxLagMs)
  expect(target.startsWith(next)).toBe(true)
  expect(next.length).toBeGreaterThan(0)
  expect(next.length).toBeLessThan(target.length)
})

test('a dumped paragraph catches up within the max lag', () => {
  const target = 'a'.repeat(80)
  let shown = ''
  let elapsed = 0
  while (shown !== target && elapsed <= STREAM_REVEAL.maxLagMs + 48) {
    shown = nextShownText(shown, target, 16, STREAM_REVEAL.maxLagMs - elapsed)
    elapsed += 16
  }
  expect(shown).toBe(target)
  expect(elapsed).toBeLessThanOrEqual(STREAM_REVEAL.maxLagMs + 48)
})

test('flush is faster once the block is no longer live', () => {
  const target = 'a'.repeat(40)
  let live = ''
  let liveMs = 0
  while (live !== target) {
    live = nextShownText(live, target, 16, STREAM_REVEAL.maxLagMs - liveMs)
    liveMs += 16
  }
  let flush = ''
  let flushMs = 0
  while (flush !== target) {
    flush = nextShownText(flush, target, 16, STREAM_REVEAL.flushLagMs - flushMs)
    flushMs += 16
  }
  expect(flushMs).toBeLessThan(liveMs)
})

test('holdIncompleteMarkdown parks unmatched markers and links', () => {
  expect(holdIncompleteMarkdown('hello **')).toEqual({ visible: 'hello ', held: '**' })
  expect(holdIncompleteMarkdown('hello **bold**')).toEqual({ visible: 'hello **bold**', held: '' })
  expect(holdIncompleteMarkdown('see `')).toEqual({ visible: 'see ', held: '`' })
  expect(holdIncompleteMarkdown('go [docs')).toEqual({ visible: 'go ', held: '[docs' })
  expect(holdIncompleteMarkdown('go [docs](https://ex')).toEqual({ visible: 'go ', held: '[docs](https://ex' })
})

test('visibleFrontierLength only counts frontier that made it into the visible string', () => {
  expect(visibleFrontierLength('hello world', 'world')).toBe(5)
  expect(visibleFrontierLength('hello ', ' **')).toBe(0)
  expect(visibleFrontierLength('hello', 'lo')).toBe(2)
  expect(visibleFrontierLength('hello', 'xyz')).toBe(0)
})
