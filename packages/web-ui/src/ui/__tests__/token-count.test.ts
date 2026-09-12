import { expect, test } from 'bun:test'
import { displayTokenCount, parseTokenCount } from '../token-count'

test('switching units preserves individual tokens, including counts below one million', () => {
  for (const count of [0, 1, 1001, 1234567]) {
    for (const unit of ['K', 'M'] as const) {
      expect(parseTokenCount(displayTokenCount(count, unit), unit)).toBe(count)
    }
  }
  expect(parseTokenCount('1,024', 'K')).toBe(1024000)
  expect(parseTokenCount('0.00125', 'M')).toBe(1250)
})

test('unit conversion refuses empty, negative, non-finite and overflowing counts', () => {
  for (const value of ['', ' ', '-1', 'NaN', 'Infinity', '1e308']) {
    expect(parseTokenCount(value, 'M')).toBeNull()
  }
})
