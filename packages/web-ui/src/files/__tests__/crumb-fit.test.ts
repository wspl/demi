import { expect, test } from 'bun:test'
import { fitCrumbs, pathCrumbs } from '../crumb-fit'

test('the path bar shows the whole path from the Host root, the root and the home wearing their glyphs', () => {
  expect(pathCrumbs('/Users/alex/Projects/demi/docs/guide.md', { platform: 'macos', home: '/Users/alex' })).toEqual([
    { name: '/', path: '/', icon: 'folder-macos' },
    { name: 'Users', path: '/Users', icon: undefined },
    { name: 'alex', path: '/Users/alex', icon: 'folder-home' },
    { name: 'Projects', path: '/Users/alex/Projects', icon: undefined },
    { name: 'demi', path: '/Users/alex/Projects/demi', icon: undefined },
    { name: 'docs', path: '/Users/alex/Projects/demi/docs', icon: undefined },
    { name: 'guide.md', path: '/Users/alex/Projects/demi/docs/guide.md', icon: undefined },
  ])
})

// Three crumbs: 60, 80 and 120 wide with their names, 24 as glyphs, 14 per separator; the ellipsis item 24.
const crumbs = [
  { full: 60, icon: 24 },
  { full: 80, icon: 24 },
  { full: 120, icon: 24 },
]
const none = { collapsed: 0, hidden: null, clipped: false, truncated: null }

test('crumbs keep their names while they fit', () => {
  expect(fitCrumbs(crumbs, 14, 24, 288)).toEqual(none)
})

test('names go one at a time from the left, the last crumb keeping its name longest', () => {
  expect(fitCrumbs(crumbs, 14, 24, 287)).toEqual({ ...none, collapsed: 1 })
  expect(fitCrumbs(crumbs, 14, 24, 196)).toEqual({ ...none, collapsed: 2 })
})

test('the widest name still shown is cut short to fill the room instead of leaving the bar half empty', () => {
  // Another crumb, 300 wide, sits where it would be given up next: a glyph for it would leave 276 of the 400 unused.
  const long = [{ full: 60, icon: 24 }, { full: 300, icon: 24 }, { full: 50, icon: 24 }]
  expect(fitCrumbs(long, 14, 24, 400)).toEqual({ ...none, collapsed: 1, truncated: { index: 1, width: 400 - 24 - 50 - 28 } })
})

// Eight crumbs of 70, 24 as glyphs: with the last named, the glyphs take 336.
const deep = Array.from({ length: 8 }, () => ({ full: 70, icon: 24 }))

test('the middle folders go behind one ellipsis, from the one after the first, so the first and the last stay whole', () => {
  expect(fitCrumbs(deep, 14, 24, 336)).toEqual({ ...none, collapsed: 7 })
  // Hiding one glyph behind an ellipsis as wide gains nothing; two gain 38.
  expect(fitCrumbs(deep, 14, 24, 300)).toEqual({ ...none, collapsed: 7, hidden: { from: 1, count: 2 } })
  expect(fitCrumbs(deep, 14, 24, 200)).toEqual({ ...none, collapsed: 7, hidden: { from: 1, count: 5 } })
})

test('the last crumb is cut short once only the first, the ellipsis and it are left, then is a glyph', () => {
  expect(fitCrumbs(deep, 14, 24, 140)).toEqual({ ...none, collapsed: 7, hidden: { from: 1, count: 6 }, truncated: { index: 7, width: 64 } })
  expect(fitCrumbs(deep, 14, 24, 120)).toEqual({ ...none, collapsed: 8, hidden: { from: 1, count: 6 } })
  // Three crumbs have one in the middle, which an ellipsis as wide would not shorten: the last name is cut first.
  expect(fitCrumbs(crumbs, 14, 24, 195)).toEqual({ ...none, collapsed: 2, truncated: { index: 2, width: 119 } })
})

test('narrower still, the first goes behind the ellipsis too, and only then is the row clipped', () => {
  expect(fitCrumbs(crumbs, 14, 24, 100)).toEqual({ ...none, collapsed: 3 })
  expect(fitCrumbs(crumbs, 14, 24, 99)).toEqual({ ...none, collapsed: 3, hidden: { from: 0, count: 2 } })
  expect(fitCrumbs(crumbs, 14, 24, 61)).toEqual({ ...none, collapsed: 3, hidden: { from: 0, count: 2 }, clipped: true })
})

test('no crumbs fit anywhere', () => {
  expect(fitCrumbs([], 14, 24, 0)).toEqual(none)
})
