import { expect, test } from 'bun:test'
import {
  AbortError,
  abortable,
  asError,
  asRecord,
  asString,
  clamp,
  concatBytes,
  createId,
  decodeUtf8,
  deferred,
  delay,
  encodeUtf8,
  errorMessage,
  isAbortError,
  isRecord,
  numberOrZero,
  shortHash,
  sliceHead,
  sliceTail,
  tail,
  toWellFormedText,
  throwIfAborted,
  truncate,
  utf8Bytes,
  utf8Slice,
} from '../index'

test('guards', () => {
  expect(isRecord({})).toBe(true)
  expect(isRecord([])).toBe(false)
  expect(isRecord(null)).toBe(false)
  expect(isRecord('x')).toBe(false)
  expect(asRecord({ a: 1 })).toEqual({ a: 1 })
  expect(() => asRecord([], 'nope')).toThrow('nope')
  expect(asString('s')).toBe('s')
  expect(asString(1)).toBeUndefined()
  expect(numberOrZero(3)).toBe(3)
  expect(numberOrZero(Number.NaN)).toBe(0)
  expect(numberOrZero('3')).toBe(0)
})

test('errors', () => {
  expect(asError(new Error('boom')).message).toBe('boom')
  expect(asError('boom').message).toBe('boom')
  expect(errorMessage(new Error('m'))).toBe('m')
  expect(errorMessage('m')).toBe('m')
  expect(errorMessage(new Error(''))).toBe(String(new Error('')))
  expect(isAbortError(new AbortError())).toBe(true)
  expect(isAbortError(new Error('x'))).toBe(false)
  expect(isAbortError({ name: 'AbortError' })).toBe(true)
  expect(isAbortError(new DOMException('stop', 'AbortError'))).toBe(true)
  const live = new AbortController()
  expect(() => throwIfAborted(live.signal)).not.toThrow()
  live.abort()
  expect(() => throwIfAborted(live.signal)).toThrow(AbortError)
})

test('abortable rejects on abort', async () => {
  const controller = new AbortController()
  const pending = abortable(new Promise<number>(() => {}), controller.signal)
  controller.abort()
  await expect(pending).rejects.toBeInstanceOf(AbortError)
  await expect(abortable(Promise.resolve(7), new AbortController().signal)).resolves.toBe(7)
})

test('async', async () => {
  const d = deferred<number>()
  queueMicrotask(() => d.resolve(42))
  expect(await d.promise).toBe(42)
  const start = await Promise.resolve(true)
  expect(start).toBe(true)
  await delay(1)
})

test('bytes round-trip and slice', () => {
  const bytes = encodeUtf8('héllo')
  expect(decodeUtf8(bytes)).toBe('héllo')
  expect(utf8Bytes('héllo')).toBe(6)
  expect(utf8Slice('abcdef', 1, 4)).toBe('bcd')
  expect(decodeUtf8(concatBytes([encodeUtf8('ab'), encodeUtf8('cd')]))).toBe('abcd')
})

test('strings', () => {
  expect(clamp(5, 0, 3)).toBe(3)
  expect(clamp(-1, 0, 3)).toBe(0)
  expect(truncate('hello', 10)).toBe('hello')
  expect(truncate('hello world', 8)).toBe('hello w…')
  expect(tail('hello world', 5)).toBe('world')
  expect(shortHash('abc')).toBe(shortHash('abc'))
  expect(shortHash('abc')).not.toBe(shortHash('abd'))
})

test('surrogate-safe slicing', () => {
  // '🙂' is '🙂' — two UTF-16 units.
  expect(sliceHead('a🙂b', 2)).toBe('a')
  expect(sliceHead('a🙂b', 3)).toBe('a🙂')
  expect(sliceHead('a🙂b', 4)).toBe('a🙂b')
  expect(sliceHead('abc', 0)).toBe('')
  expect(sliceTail('a🙂b', 2)).toBe('b')
  expect(sliceTail('a🙂b', 3)).toBe('🙂b')
  expect(sliceTail('a🙂b', 4)).toBe('a🙂b')
  expect(sliceTail('abc', 0)).toBe('')
  expect(truncate('🙂🙂🙂', 4)).toBe('🙂…')
  expect(truncate('🙂🙂🙂', 2, '')).toBe('🙂')
  expect(tail('🙂🙂🙂', 3)).toBe('🙂')
})

test('toWellFormedText', () => {
  expect(toWellFormedText('a🙂b')).toBe('a🙂b')
  expect(toWellFormedText('a\ud83d')).toBe('a�')
  expect(toWellFormedText('\ude42b')).toBe('�b')
  expect(toWellFormedText('a\ud83d-\ude42b')).toBe('a�-�b')
})

test('createId is unique-ish', () => {
  expect(createId()).not.toBe(createId())
})
