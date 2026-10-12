import { expect, test } from 'bun:test'
import { stepHighlight } from '../list-highlight'

test.each([
  // [from, step, count, to]
  [-1, 1, 3, 0],
  [-1, -1, 3, 2],
  [0, 1, 3, 1],
  [1, -1, 3, 0],
  [2, 1, 3, 0],
  [0, -1, 3, 2],
  [0, 1, 1, 0],
  [0, -1, 1, 0],
  [0, 1, 0, -1],
  [-1, -1, 0, -1],
] as const)('the highlight at %i moves %i in a list of %i to %i, wrapping at the ends', (from, step, count, to) => {
  expect(stepHighlight(from, step, count)).toBe(to)
})
