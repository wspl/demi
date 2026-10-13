// `--limit` is read before the call reaches the slot's server: one longer
// than a timer holds ended the call as it started, saying it had run for
// the whole limit. Costs under a millisecond.
import { expect, test } from 'bun:test'
import { parseCallLimit } from './limit'

test('--limit counts seconds or minutes, and refuses one longer than a timer holds', () => {
  expect(parseCallLimit('90')).toBe(90_000)
  expect(parseCallLimit('15m')).toBe(900_000)
  expect(() => parseCallLimit('2400000')).toThrow('at most 24 days')
  expect(() => parseCallLimit('ten')).toThrow('seconds or minutes')
})
