import { expect, test } from 'bun:test'
import { OUTLINE_ROOM, deviceOutline } from '../outline'

// The wireframe frame a view draws when the device's own is not installed
// (`mobile.md` § The live view): the outline keeps the screen's corners at a
// bezel's distance, and each button is a mark outside its edge, where the
// fraction of the edge puts it. Pure geometry, under a millisecond.

const screen = { left: 100, top: 50, width: 400, height: 800 }

test('the outline follows the screen with its corners and marks each button outside its edge', () => {
  const { body, marks } = deviceOutline(screen, 40, [
    { side: 'left', start: 0.25, length: 0.1 },
    { side: 'right', start: 0.5, length: 0.1 },
    { side: 'top', start: 0.5, length: 0.25 },
    { side: 'bottom', start: 0, length: 0.1 },
  ])
  // The body is wider than the screen on every side by the same bezel, its corners rounder by it.
  const bezel = screen.left - body.left
  expect(bezel).toBeGreaterThan(0)
  expect(body).toEqual({
    left: screen.left - bezel,
    top: screen.top - bezel,
    width: screen.width + 2 * bezel,
    height: screen.height + 2 * bezel,
    radius: 40 + bezel,
  })
  const [left, right, top, bottom] = marks
  // Each mark is beyond its edge, within the room the outline takes.
  expect(left!.left + left!.width).toBeLessThanOrEqual(body.left)
  expect(screen.left - left!.left).toBeLessThanOrEqual(OUTLINE_ROOM)
  expect(right!.left).toBeGreaterThanOrEqual(body.left + body.width)
  expect(right!.left + right!.width - (screen.left + screen.width)).toBeLessThanOrEqual(OUTLINE_ROOM)
  expect(top!.top + top!.height).toBeLessThanOrEqual(body.top)
  expect(bottom!.top).toBeGreaterThanOrEqual(body.top + body.height)
  // Along its edge, where its fractions say.
  expect(left!.top).toBeCloseTo(body.top + 0.25 * body.height)
  expect(left!.height).toBeCloseTo(0.1 * body.height)
  expect(right!.top).toBeCloseTo(body.top + 0.5 * body.height)
  expect(top!.left).toBeCloseTo(body.left + 0.5 * body.width)
  expect(top!.width).toBeCloseTo(0.25 * body.width)
  expect(bottom!.left).toBeCloseTo(body.left)
})

test('a square screen keeps square corners', () => {
  expect(deviceOutline(screen, 0, []).body.radius).toBe(0)
})
