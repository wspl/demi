import { expect, test } from 'bun:test'
import { REVEAL_MARGIN_PX, revealOffset } from '../reveal'

test('a block that fits the view stands in its middle', () => {
  // A 28px row in a 600px view: 286px above it, 286px below.
  expect(revealOffset(28, 600)).toBe(286)
  // A block that just fits with its margins is still centered.
  expect(revealOffset(600 - 2 * REVEAL_MARGIN_PX, 600)).toBe(REVEAL_MARGIN_PX)
})

test('a block taller than the view starts near its top, so it reads from its first line', () => {
  expect(revealOffset(601, 600)).toBe(REVEAL_MARGIN_PX)
  expect(revealOffset(2400, 600)).toBe(REVEAL_MARGIN_PX)
  // Too tall to stand clear of both margins: its start keeps the top one.
  expect(revealOffset(590, 600)).toBe(REVEAL_MARGIN_PX)
})
