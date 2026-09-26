import { expect, test } from 'bun:test'
import { delay, sliceHead, truncate } from '../index'

test('an aborted delay ends at once instead of after its time', async () => {
  const stop = new AbortController()
  stop.abort()
  await delay(60_000, stop.signal)
})

test('surrogate-safe slicing', () => {
  // '🙂' is two UTF-16 units.
  expect(sliceHead('a🙂b', 2)).toBe('a')
  expect(sliceHead('a🙂b', 3)).toBe('a🙂')
  expect(sliceHead('a🙂b', 4)).toBe('a🙂b')
  expect(sliceHead('abc', 0)).toBe('')
  expect(truncate('🙂🙂🙂', 4)).toBe('🙂…')
  expect(truncate('🙂🙂🙂', 2, '')).toBe('🙂')
})
