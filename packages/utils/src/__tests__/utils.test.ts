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
  tail,
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
})

test('createId is unique-ish', () => {
  expect(createId()).not.toBe(createId())
})
