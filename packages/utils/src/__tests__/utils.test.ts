import { expect, test } from 'bun:test'
import {
  asError,
  clamp,
  createId,
  deferred,
  delay,
  sliceHead,
  truncate,
  utf8Bytes,
} from '../index'

test('errors', () => {
  expect(asError(new Error('boom')).message).toBe('boom')
  expect(asError('boom').message).toBe('boom')
})

test('async', async () => {
  const d = deferred<number>()
  queueMicrotask(() => d.resolve(42))
  expect(await d.promise).toBe(42)
  await delay(1)
  const stop = new AbortController()
  stop.abort()
  // An aborted signal ends the wait at once instead of after the delay.
  await delay(60_000, stop.signal)
})

test('bytes', () => {
  expect(utf8Bytes('héllo')).toBe(6)
})

test('strings', () => {
  expect(clamp(5, 0, 3)).toBe(3)
  expect(clamp(-1, 0, 3)).toBe(0)
  expect(truncate('hello', 10)).toBe('hello')
  expect(truncate('hello world', 8)).toBe('hello w…')
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

test('createId is unique-ish', () => {
  expect(createId()).not.toBe(createId())
})
