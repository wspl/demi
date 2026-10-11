import { expect, test } from 'bun:test'
import { dragVelocity, dragCloses } from '../drag-close'

test('a slow pull closes a sheet or a drawer past 30% of its size and springs back short of it', () => {
  expect(dragCloses(120, 400, 0.1)).toBe(true)
  expect(dragCloses(119, 400, 0.1)).toBe(false)
})

test('a flick toward closing closes however short the pull, and a flick back keeps it open', () => {
  expect(dragCloses(20, 400, 0.6)).toBe(true)
  expect(dragCloses(300, 400, -0.6)).toBe(false)
  expect(dragCloses(0, 400, 2)).toBe(false)
})

test('the speed of a drag is how the finger left, over its last 80 ms', () => {
  expect(dragVelocity([{ at: 0, t: 0 }, { at: 10, t: 200 }, { at: 50, t: 260 }, { at: 90, t: 280 }])).toBe(1)
  expect(dragVelocity([{ at: 0, t: 0 }])).toBe(0)
})
