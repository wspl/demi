import { expect, test } from 'bun:test'
import {
  eventHitsInside,
  eventHitsSelector,
  isPrimaryMousePointerDown,
  shouldDismissOutside,
} from '../dismissOutside'

function eventOn(path: EventTarget[]): Event {
  return {
    target: path[0],
    composedPath: () => path,
  } as unknown as Event
}

function pointerDown(partial: Partial<PointerEvent>): PointerEvent {
  return {
    isPrimary: true,
    button: 0,
    pointerType: 'mouse',
    ...partial,
  } as PointerEvent
}

test('a path that includes the target is inside', () => {
  const panel = { id: 'panel' } as unknown as HTMLElement
  const child = { id: 'child' } as unknown as HTMLElement
  expect(eventHitsInside(eventOn([child, panel]), panel)).toBe(true)
  expect(eventHitsInside(eventOn([child]), panel)).toBe(false)
  expect(eventHitsInside(eventOn([child]), null)).toBe(false)
})

test('a selector matches any element in the path', () => {
  const menu = {
    matches: (selector: string) => selector === '[data-overlay-panel]',
  } as unknown as Element
  const label = { matches: () => false } as unknown as Element
  expect(eventHitsSelector(eventOn([label, menu]), '[data-overlay-panel]')).toBe(
    true,
  )
  expect(eventHitsSelector(eventOn([label]), '[data-overlay-panel]')).toBe(false)
})

test('outside dismiss skips the window, ignored overlays, and a missing target', () => {
  const panel = { id: 'panel' } as unknown as HTMLElement
  const under = { id: 'under' } as unknown as HTMLElement
  const menu = {
    matches: (selector: string) => selector === '[data-overlay-panel]',
  } as unknown as Element
  expect(shouldDismissOutside(eventOn([under]), panel)).toBe(true)
  expect(shouldDismissOutside(eventOn([under, panel]), panel)).toBe(false)
  expect(
    shouldDismissOutside(eventOn([menu]), panel, ['[data-overlay-panel]']),
  ).toBe(false)
  expect(shouldDismissOutside(eventOn([under]), null)).toBe(false)
  const chip = {
    matches: (selector: string) => selector === '[data-session-overlay-toggle]',
  } as unknown as Element
  expect(
    shouldDismissOutside(eventOn([chip]), panel, ['[data-session-overlay-toggle]']),
  ).toBe(false)
})

test('only a primary mouse down is consumed before click', () => {
  expect(isPrimaryMousePointerDown(pointerDown({}))).toBe(true)
  expect(isPrimaryMousePointerDown(pointerDown({ pointerType: 'touch' }))).toBe(
    false,
  )
  expect(isPrimaryMousePointerDown(pointerDown({ pointerType: 'pen' }))).toBe(false)
  expect(isPrimaryMousePointerDown(pointerDown({ button: 2 }))).toBe(false)
})
