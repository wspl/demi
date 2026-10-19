import { expect, test } from 'bun:test'
import { fittingActions } from '../toolbar-fit'

// A toolbar too narrow for its actions keeps those that fit beside its More
// menu, as a macOS window's toolbar does, and shows all once they fit:
// ten 36 px actions 2 px apart, as the Android device panel's on a phone,
// the More button as wide as one. Pure arithmetic, under a millisecond.

const widths = Array.from({ length: 10 }, () => 36)

test('a toolbar shows every action that fits, and More only when some do not', () => {
  // 10 × 36 + 9 × 2 = 378.
  expect(fittingActions(widths, 2, 378, 36)).toBe(10)
  // One pixel short: More takes the room of the last two, as eight and More
  // take 8 × 36 + 8 × 2 + 36 = 340 and nine 378.
  expect(fittingActions(widths, 2, 377, 36)).toBe(8)
  expect(fittingActions(widths, 2, 340, 36)).toBe(8)
  expect(fittingActions(widths, 2, 339, 36)).toBe(7)
  // No room even for one beside More.
  expect(fittingActions(widths, 2, 40, 36)).toBe(0)
})
