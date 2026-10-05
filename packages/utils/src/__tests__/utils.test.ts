import { expect, test } from 'bun:test'
import { clientPlatform, delay, sliceHead, truncate } from '../index'

test('an aborted delay ends at once instead of after its time', async () => {
  const stop = new AbortController()
  stop.abort()
  await delay(60_000, stop.signal)
})

test('the platform comes from the web browser that reports it', () => {
  expect(clientPlatform({ platform: 'MacIntel', userAgent: 'Mozilla/5.0 (Macintosh)' })).toBe('mac')
  expect(clientPlatform({ platform: 'Win32', userAgent: 'Mozilla/5.0 (Windows NT 10.0)' })).toBe('windows')
  expect(clientPlatform({ platform: '', userAgent: 'Mozilla/5.0 (X11; Linux x86_64)' })).toBe('linux')
  expect(clientPlatform({ platform: '', userAgent: 'Mozilla/5.0 (SomethingElse)' })).toBe('other')
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
