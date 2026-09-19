import { expect, test } from 'bun:test'
import { createOverlayFamily } from '../overlayFamily'

function fakePanel(): HTMLElement {
  return { id: crypto.randomUUID() } as HTMLElement
}

test('register adds a panel and unregister removes it', () => {
  const family = createOverlayFamily()
  const panel = fakePanel()
  const unregister = family.register(panel)
  expect(family.panels).toEqual([panel])
  unregister()
  expect(family.panels).toEqual([])
})
