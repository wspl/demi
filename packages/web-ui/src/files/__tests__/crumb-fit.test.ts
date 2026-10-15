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

// Three crumbs: 60, 80 and 120 wide with their names, 24 as glyphs, 14 per separator.
const crumbs = [
  { full: 60, icon: 24 },
  { full: 80, icon: 24 },
  { full: 120, icon: 24 },
]

test('crumbs keep their names while they fit', () => {
  expect(fitCrumbs(crumbs, 14, 288)).toEqual({ collapsed: 0, clipped: false, truncated: null })
})

test('names go one at a time from the left, the last crumb keeping its name longest', () => {
  expect(fitCrumbs(crumbs, 14, 287)).toEqual({ collapsed: 1, clipped: false, truncated: null })
  expect(fitCrumbs(crumbs, 14, 196)).toEqual({ collapsed: 2, clipped: false, truncated: null })
})

test('the widest name still shown is cut short to fill the room instead of leaving the bar half empty', () => {
  // Another crumb, 300 wide, sits where it would be given up next: a glyph for it would leave 276 of the 400 unused.
  const long = [{ full: 60, icon: 24 }, { full: 300, icon: 24 }, { full: 50, icon: 24 }]
  expect(fitCrumbs(long, 14, 400)).toEqual({ collapsed: 1, clipped: false, truncated: { index: 1, width: 400 - 24 - 50 - 28 } })
})

test('the last crumb is cut short when it alone overflows', () => {
  expect(fitCrumbs(crumbs, 14, 195)).toEqual({ collapsed: 2, clipped: false, truncated: { index: 2, width: 119 } })
})

test('a row of glyphs that still overflows is clipped', () => {
  expect(fitCrumbs(crumbs, 14, 100)).toEqual({ collapsed: 3, clipped: false, truncated: null })
  expect(fitCrumbs(crumbs, 14, 99)).toEqual({ collapsed: 3, clipped: true, truncated: null })
})

test('no crumbs fit anywhere', () => {
  expect(fitCrumbs([], 14, 0)).toEqual({ collapsed: 0, clipped: false, truncated: null })
})
