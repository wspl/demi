import { afterEach, beforeEach, expect, test } from 'bun:test'
import { reloadFor } from '../build-reload'

const storageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'sessionStorage')
const saved = new Map<string, string>()

/** A `sessionStorage` whose writes throw when `failing`. */
function stubStorage(failing: boolean): void {
  Object.defineProperty(globalThis, 'sessionStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => saved.get(key) ?? null,
      setItem: (key: string, value: string) => {
        if (failing) {
          throw new DOMException('The quota has been exceeded.', 'QuotaExceededError')
        }
        saved.set(key, value)
      },
    },
  })
}

beforeEach(() => {
  saved.clear()
})

afterEach(() => {
  if (storageDescriptor) {
    Object.defineProperty(globalThis, 'sessionStorage', storageDescriptor)
  } else {
    Reflect.deleteProperty(globalThis, 'sessionStorage')
  }
})

test('a page loads itself again once per build, so a cache that keeps an old page cannot make it loop', () => {
  stubStorage(false)
  let reloads = 0
  const reload = () => {
    reloads += 1
  }
  expect(reloadFor('b2', reload)).toBe(true)
  // The cache served the old page again: the backend still serves b2.
  expect(reloadFor('b2', reload)).toBe(false)
  // A later release is another build, which the page tries once too.
  expect(reloadFor('b3', reload)).toBe(true)
  expect(reloads).toBe(2)
})

test('a page that cannot keep the record does not load itself again', () => {
  stubStorage(true)
  let reloads = 0
  expect(reloadFor('b2', () => { reloads += 1 })).toBe(false)
  expect(reloads).toBe(0)
})
