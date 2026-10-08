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

test('a stacked entry keeps what it stands on and goes with it', () => {
  const store = createOverlayStore()
  let outerClosed = 0
  let innerClosed = 0
  store.push('dialog', () => {
    outerClosed += 1
  }, 'exclusive', 'dialog')
  store.push('inner', () => {
    innerClosed += 1
  }, 'stacked', 'dialog')
  expect(outerClosed).toBe(0)
  expect(store.state.entries.map((entry) => entry.id)).toEqual(
    ['dialog', 'inner']
  )
  expect(store.isTop('inner')).toBe(true)
  expect(store.isTop('dialog')).toBe(false)

  store.closeTop()
  expect(innerClosed).toBe(1)
  expect(outerClosed).toBe(0)
  // The closed dialog unregisters itself, as the component does when it is no longer open.
  store.remove('inner')

  store.push('inner-2', () => {
    innerClosed += 1
  }, 'stacked', 'dialog')
  store.remove('dialog')
  expect(innerClosed).toBe(2)
  expect(store.state.entries).toHaveLength(0)
})

test('a stacked dialog a menu row opens replaces the menu and stays when the menu closes', () => {
  // The host menu's Cloud row asks before a move, and Add Device… pairs:
  // each opens a stacked dialog as its menu closes.
  const store = createOverlayStore()
  let menuClosed = 0
  let dialogClosed = 0
  store.push('menu', () => {
    menuClosed += 1
  })
  store.push('question', () => {
    dialogClosed += 1
  }, 'stacked', 'dialog')
  expect(menuClosed).toBe(1)
  // The menu unregisters itself once it is closed, which takes nothing with it.
  store.remove('menu')
  expect(dialogClosed).toBe(0)
  expect(store.state.entries.map((entry) => entry.id)).toEqual(['question'])
  expect(store.isTop('question')).toBe(true)
})

test('a stacked dialog over a picker with a menu open stands on the picker', () => {
  const store = createOverlayStore()
  let pickerClosed = 0
  store.push('picker', () => {
    pickerClosed += 1
  }, 'exclusive', 'dialog')
  store.push('question', () => {}, 'stacked', 'dialog')
  expect(pickerClosed).toBe(0)
  expect(store.state.entries.find((entry) => entry.id === 'question')?.parent).toBe('picker')
})
