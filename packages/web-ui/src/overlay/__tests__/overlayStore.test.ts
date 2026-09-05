import { expect, test } from 'bun:test'
import { createOverlayStore } from '../overlayStore'

test('exclusive push dismisses hints', () => {
  const store = createOverlayStore()
  let hintClosed = 0
  store.push('hint-1', () => {
    hintClosed += 1
  }, 'hint')
  expect(store.state.entries).toHaveLength(1)

  store.push('menu-1', () => {})
  expect(hintClosed).toBe(1)
  expect(store.hasExclusive()).toBe(true)
  expect(store.state.entries.map((entry) => entry.layer)).toEqual(['exclusive'])
})

test('hint will not register while an exclusive overlay is open', () => {
  const store = createOverlayStore()
  store.push('menu-1', () => {})
  const unregister = store.push('hint-1', () => {}, 'hint')
  expect(store.state.entries).toHaveLength(1)
  expect(store.state.entries[0]?.id).toBe('menu-1')
  unregister()
  expect(store.state.entries).toHaveLength(1)
})

test('a new exclusive root closes the previous exclusive', () => {
  const store = createOverlayStore()
  let firstClosed = 0
  store.push('menu-1', () => {
    firstClosed += 1
  })
  store.push('menu-2', () => {})
  expect(firstClosed).toBe(1)
  expect(store.state.entries.map((entry) => entry.id)).toEqual(['menu-2'])
})

test('hasEntries and closeTop ignore a hint-only stack', () => {
  const store = createOverlayStore()
  let hintClosed = 0
  store.push('hint-1', () => {
    hintClosed += 1
  }, 'hint')
  expect(store.hasEntries()).toBe(false)
  store.closeTop()
  expect(hintClosed).toBe(0)
  expect(store.state.entries).toHaveLength(1)
})
