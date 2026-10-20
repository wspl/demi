import { afterEach, beforeEach, expect, test } from 'bun:test'
import { IDBFactory, IDBKeyRange } from 'fake-indexeddb'
import { pluginCache } from '../cache'

// fake-indexeddb stands in for the browser's database; a few milliseconds.

const real = { indexedDB: globalThis.indexedDB, IDBKeyRange: globalThis.IDBKeyRange }

beforeEach(() => {
  globalThis.indexedDB = new IDBFactory()
  globalThis.IDBKeyRange = IDBKeyRange
})

afterEach(() => {
  globalThis.indexedDB = real.indexedDB
  globalThis.IDBKeyRange = real.IDBKeyRange
})

const bytes = (size: number, fill: number) => new Uint8Array(size).fill(fill)

/**
 * Planted defects this catches: entries let go of in the order they were
 * written rather than used, a limit counted across plugins, and none at all.
 */
test('a plugin’s cache keeps what fits its size, letting go of the least recently used first, and leaves another plugin’s alone', async () => {
  let clock = 0
  const now = () => (clock += 1)
  const mobile = pluginCache('mobile', 30, now)
  const browser = pluginCache('browser', 30, now)
  await browser.write('t1', bytes(20, 9))
  await mobile.write('a', bytes(10, 1))
  await mobile.write('b', bytes(10, 2))
  // Read now, so `b` is the least recently used.
  expect(await mobile.read('a')).toEqual(bytes(10, 1))
  await mobile.write('c', bytes(15, 3))
  expect(await mobile.read('b')).toBeNull()
  expect(await mobile.read('a')).toEqual(bytes(10, 1))
  expect(await mobile.read('c')).toEqual(bytes(15, 3))
  expect(await browser.read('t1')).toEqual(bytes(20, 9))
})
