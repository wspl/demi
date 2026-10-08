import { expect, test } from 'bun:test'
import { slotNumber, slotPorts } from './slot'

test('a checkout in demi-slots/<n> is slot n, any other checkout slot 0', () => {
  const cases: [string, number][] = [
    ['/Users/zan/Projects/demi-slots/2', 2],
    ['/Users/zan/Projects/demi-slots/4/', 4],
    ['/Users/zan/Projects/demi', 0],
    ['/Users/zan/Projects/demi-worktrees/2', 0],
    ['/Users/zan/Projects/demi-slots/12', 0],
    ['/Users/zan/Projects/demi-slots/x', 0],
  ]
  for (const [root, slot] of cases) {
    expect([root, slotNumber(root)]).toEqual([root, slot])
  }
})

test('slots get disjoint ports in 33n0-33n9, never the user\'s own servers\' ports', () => {
  expect(slotPorts(2)).toEqual({ backend: 3320, web: 3321, gallery: 3322, network: 3323 })
  const all = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9].flatMap((slot) => Object.values(slotPorts(slot)))
  expect(new Set(all).size).toBe(all.length)
  expect(all).not.toContain(3291)
})
