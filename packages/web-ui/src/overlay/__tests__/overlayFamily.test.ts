import { expect, test } from 'bun:test'
import { createOverlayFamily, isInsideOverlayFamily } from '../overlayFamily'

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

test('a click on a child panel is inside the family', () => {
  const family = createOverlayFamily()
  const parent = fakePanel()
  const child = fakePanel()
  family.register(parent)
  family.register(child)
  const event = {
    composedPath: () => [child],
  } as unknown as Event
  expect(isInsideOverlayFamily(family, event)).toBe(true)
  expect(isInsideOverlayFamily(family, { composedPath: () => [fakePanel()] } as unknown as Event)).toBe(false)
})
