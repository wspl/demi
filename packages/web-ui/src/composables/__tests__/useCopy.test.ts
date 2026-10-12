import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'
import { effectScope, ref, type EffectScope } from 'vue'

// What a Copy control does when the clipboard takes the copy, refuses it, or
// the control's text changes after it (`useCopy`). A page of happy-dom with a
// clipboard stand-in; a few milliseconds.

GlobalRegistrator.register({ url: 'https://demi.test/' })
// After the page, since the library looks for a browser as it loads.
const { useCopy } = await import('../useCopy')
const { toasts } = await import('../../infra/toast')

let written: ClipboardItem[][] = []
let refuse: Error | null = null
let scope: EffectScope

beforeEach(() => {
  written = []
  refuse = null
  toasts.length = 0
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: {
      write: async (items: ClipboardItem[]) => {
        if (refuse)
          throw refuse
        written.push(items)
      },
    },
  })
  scope = effectScope()
})

afterEach(() => scope.stop())

test('a copy puts the text on the clipboard and shows Copied', async () => {
  const { copy, copied } = scope.run(() => useCopy(() => 'abc'))!
  expect(copied.value).toBe(false)
  await copy()
  expect(written).toHaveLength(1)
  expect(await (await written[0]![0]!.getType('text/plain')).text()).toBe('abc')
  expect(copied.value).toBe(true)
  expect(toasts).toHaveLength(0)
})

test('a refused copy tells the user why, shows no Copied, and does not reject', async () => {
  refuse = new Error('Writing to the clipboard is not allowed.')
  const { copy, copied } = scope.run(() => useCopy(() => 'abc'))!
  await copy()
  expect(copied.value).toBe(false)
  expect(toasts.map(({ title, message, tone }) => ({ title, message, tone }))).toEqual([
    { title: 'Could Not Copy', message: 'Writing to the clipboard is not allowed.', tone: 'danger' },
  ])
})

test('a page whose clipboard is unusable fails visibly, not silently', async () => {
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: undefined })
  const { copy, copied } = scope.run(() => useCopy(() => 'abc'))!
  await copy()
  expect(copied.value).toBe(false)
  expect(toasts.map(({ title }) => title)).toEqual(['Could Not Copy'])
})

test('Copied goes once the control would copy something else', async () => {
  const text = ref('first')
  const { copy, copied } = scope.run(() => useCopy(() => text.value))!
  await copy()
  expect(copied.value).toBe(true)
  text.value = 'second'
  expect(copied.value).toBe(false)
})

test('a source with nothing to copy copies nothing', async () => {
  const { copy, copied } = scope.run(() => useCopy(() => undefined))!
  await copy()
  expect(written).toHaveLength(0)
  expect(copied.value).toBe(false)
  expect(toasts).toHaveLength(0)
})

test('a clipboard item goes as it is', async () => {
  const item = new ClipboardItem({ 'text/plain': new Blob(['rich'], { type: 'text/plain' }) })
  const { copy, copied } = scope.run(() => useCopy(() => item))!
  await copy()
  expect(written[0]![0]).toBe(item)
  expect(copied.value).toBe(true)
})
