import { expect, test } from 'bun:test'
import { resolveCompactionThreshold } from '../session/compaction'

test('resolveCompactionThreshold uses ratio when absolute tokens are unset', () => {
  expect(resolveCompactionThreshold(272_000, 0.8, null)).toBe(217_600)
  expect(resolveCompactionThreshold(372_000, 0.9, undefined)).toBe(334_800)
})

test('resolveCompactionThreshold prefers absolute tokens over ratio', () => {
  expect(resolveCompactionThreshold(372_000, 0.8, 334_800)).toBe(334_800)
  expect(resolveCompactionThreshold(272_000, 0.8, 250_000)).toBe(250_000)
})

test('resolveCompactionThreshold clamps absolute tokens to the context window', () => {
  expect(resolveCompactionThreshold(272_000, 0.8, 400_000)).toBe(272_000)
  expect(resolveCompactionThreshold(100_000, 0.8, -10)).toBe(0)
})

test('resolveCompactionThreshold disables compaction for non-finite ratio', () => {
  expect(resolveCompactionThreshold(372_000, Number.POSITIVE_INFINITY, 334_800)).toBe(
    Number.POSITIVE_INFINITY,
  )
  expect(resolveCompactionThreshold(0, 0.8, 10_000)).toBe(0)
})
