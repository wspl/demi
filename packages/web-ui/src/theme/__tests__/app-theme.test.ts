import { expect, test } from 'bun:test'
import { appThemeStore, applyThemeToDocument, setTheme, setThemeChoice, themeChoice } from '../appTheme'

test('an in-memory system choice survives storage errors and releases its OS listener', () => {
  const descriptors = Object.fromEntries(['window', 'document', 'localStorage'].map((key) => [
    key, Object.getOwnPropertyDescriptor(globalThis, key),
  ]))
  const previous = themeChoice()
  const media = new EventTarget()
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {
    matchMedia: () => Object.assign(media, { matches: false }),
  } })
  Object.defineProperty(globalThis, 'document', { configurable: true, value: {
    documentElement: { setAttribute() {} },
  } })
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    setItem() { throw new Error('blocked') },
    removeItem() { throw new Error('blocked') },
  } })
  let dispose: (() => void) | undefined
  try {
    setTheme('dark')
    setThemeChoice('system')
    expect(themeChoice()).toBe('system')
    dispose = applyThemeToDocument()
    media.dispatchEvent(Object.assign(new Event('change'), { matches: true }))
    expect(appThemeStore.state.mode).toBe('light')
    dispose()
    media.dispatchEvent(Object.assign(new Event('change'), { matches: false }))
    expect(appThemeStore.state.mode).toBe('light')
  } finally {
    dispose?.()
    for (const [key, descriptor] of Object.entries(descriptors)) {
      if (descriptor) {
        Object.defineProperty(globalThis, key, descriptor)
      } else {
        Reflect.deleteProperty(globalThis, key)
      }
    }
    setThemeChoice(previous)
  }
})
