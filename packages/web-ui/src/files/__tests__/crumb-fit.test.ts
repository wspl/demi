import { expect, test } from 'bun:test'
import { fitCrumbs } from '../crumb-fit'

// Three crumbs: 60, 80 and 120 wide with their names, 24 as glyphs, 14 per separator.
const crumbs = [
  { full: 60, icon: 24 },
  { full: 80, icon: 24 },
  { full: 120, icon: 24 },
]

test('crumbs keep their names while they fit', () => {
  expect(fitCrumbs(crumbs, 14, 288)).toEqual({ collapsed: 0, clipped: false })
})

test('names go one at a time from the left, the last crumb keeping its name longest', () => {
  expect(fitCrumbs(crumbs, 14, 287)).toEqual({ collapsed: 1, clipped: false })
  expect(fitCrumbs(crumbs, 14, 196)).toEqual({ collapsed: 2, clipped: false })
  expect(fitCrumbs(crumbs, 14, 195)).toEqual({ collapsed: 3, clipped: false })
})

test('a row of glyphs that still overflows is clipped', () => {
  expect(fitCrumbs(crumbs, 14, 100)).toEqual({ collapsed: 3, clipped: false })
  expect(fitCrumbs(crumbs, 14, 99)).toEqual({ collapsed: 3, clipped: true })
})

test('no crumbs fit anywhere', () => {
  expect(fitCrumbs([], 14, 0)).toEqual({ collapsed: 0, clipped: false })
})
