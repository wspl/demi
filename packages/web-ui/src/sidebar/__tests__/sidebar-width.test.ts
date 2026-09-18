import { expect, test } from 'bun:test'
import { ASIDE_SHARE, asideBounds, asideShareFor, asideWidthFor } from '../sidebar-width'

test('the panel scales with the width it splits with the conversation', () => {
  expect(asideWidthFor(0.4, 1000)).toBe(400)
  expect(asideWidthFor(0.4, 1500)).toBe(600)
  expect(asideWidthFor(0.4, 800)).toBe(320)
})

test('the px floors hold: the panel keeps its minimum and leaves the conversation its own', () => {
  expect(asideWidthFor(0.1, 1000)).toBe(ASIDE_SHARE.minWidth)
  expect(asideWidthFor(0.9, 1000)).toBe(1000 - ASIDE_SHARE.mainMinWidth)
  // Too narrow for both floors: the panel keeps its minimum.
  expect(asideBounds(500)).toEqual({ min: ASIDE_SHARE.minWidth, max: ASIDE_SHARE.minWidth })
  expect(asideWidthFor(0.4, 500)).toBe(ASIDE_SHARE.minWidth)
})

test('a drag stores the share its px are of the shared width, and an unmeasured frame stores the default', () => {
  expect(asideShareFor(450, 1000)).toBe(0.45)
  expect(asideWidthFor(asideShareFor(450, 1000), 1000)).toBe(450)
  expect(asideShareFor(450, 0)).toBe(ASIDE_SHARE.default)
})
