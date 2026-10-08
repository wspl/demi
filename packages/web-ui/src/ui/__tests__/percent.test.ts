import { expect, test } from 'bun:test'
import { formatPercent } from '../percent'

// Cost: pure formatting; microseconds.

test('a meter reads whole percent of a vendor float, and full only once full', () => {
  // As a provider's used ÷ limit × 100 yields it.
  expect(formatPercent(7.000000000000001, 100)).toBe('7%')
  expect(formatPercent(0.07 * 100, 100)).toBe('7%')
  expect(formatPercent(0.4, 100)).toBe('0%')
  expect(formatPercent(99.6, 100)).toBe('99%')
  expect(formatPercent(100, 100)).toBe('100%')
  expect(formatPercent(3, 0)).toBe('0%')
})
