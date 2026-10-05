import { describe, expect, test } from 'bun:test'
import { TREE_ROW_PITCH_PX, TREE_ROW_PX, revealTreeRow, stickyTreeRows, treeBlock, type TreeRow } from '../tree'

// src/, src/auth/, src/auth/cookie.ts, src/auth/session.ts, src/index.ts, tests/
const rows: TreeRow[] = [
  { path: '/w/src', name: 'src', isDirectory: true, depth: 0, parent: null, open: true },
  { path: '/w/src/auth', name: 'auth', isDirectory: true, depth: 1, parent: '/w/src', open: true },
  { path: '/w/src/auth/cookie.ts', name: 'cookie.ts', isDirectory: false, depth: 2, parent: '/w/src/auth', open: false },
  { path: '/w/src/auth/session.ts', name: 'session.ts', isDirectory: false, depth: 2, parent: '/w/src/auth', open: false },
  { path: '/w/src/index.ts', name: 'index.ts', isDirectory: false, depth: 1, parent: '/w/src', open: false },
  { path: '/w/tests', name: 'tests', isDirectory: true, depth: 0, parent: null, open: false },
]
const caption = 28
const top = (path: string) => caption + rows.findIndex((row) => row.path === path) * TREE_ROW_PITCH_PX
const paths = (scrollTop: number) => stickyTreeRows(rows, top, scrollTop, caption).paths

describe('sticky tree rows', () => {
  test('the directories under the top pin as their rows reach the stack', () => {
    // At the top, src's row lies exactly in the first slot and auth's in the second: pinned
    // copies over the rows themselves, so the swap shows nothing.
    expect(paths(0)).toEqual(['/w/src', '/w/src/auth'])
    // One pixel in: src's row is under the caption and auth's under the pinned src.
    expect(paths(1)).toEqual(['/w/src', '/w/src/auth'])
  })

  test('a directory goes once its last row is past the slot it would hold', () => {
    // auth's last row, session.ts, sits above auth's slot: auth is past; src still holds index.ts.
    expect(paths(3 * TREE_ROW_PITCH_PX)).toEqual(['/w/src'])
    // Everything in src is past: nothing pins.
    expect(paths(5 * TREE_ROW_PITCH_PX)).toEqual([])
  })

  test('the stack rides up with the deepest directory as it leaves', () => {
    // src alone, its last row index.ts ending 11px above the stack bottom.
    const stack = stickyTreeRows(rows, top, 4 * TREE_ROW_PITCH_PX + 10, caption)
    expect(stack.paths).toEqual(['/w/src'])
    expect(stack.offset).toBe(TREE_ROW_PX - TREE_ROW_PITCH_PX - 10)
  })

  test('a closed directory at the top never pins', () => {
    // tests, closed, holds the first slot: nothing under it to keep in sight.
    expect(paths(5 * TREE_ROW_PITCH_PX + 1)).toEqual([])
  })

  test('nothing pins past the last row', () => {
    expect(paths(10 * TREE_ROW_PITCH_PX)).toEqual([])
  })
})

test('a pinned directory that folds rests in its pinned slot, in sight under the stack', () => {
  // README.md, .claude/, .claude/screenshots/ with 40 shots, .claude/settings.json, then 20 files.
  const file = (path: string, depth: number, parent: string | null): TreeRow =>
    ({ path, name: path.split('/').pop()!, isDirectory: false, depth, parent, open: false })
  const shots = '/w/.claude/screenshots'
  const head: TreeRow[] = [
    file('/w/README.md', 0, null),
    { path: '/w/.claude', name: '.claude', isDirectory: true, depth: 0, parent: null, open: true },
  ]
  const tail: TreeRow[] = [
    file('/w/.claude/settings.json', 1, '/w/.claude'),
    ...Array.from({ length: 20 }, (_, index) => file(`/w/file-${index}.ts`, 0, null)),
  ]
  const open: TreeRow[] = [
    ...head,
    { path: shots, name: 'screenshots', isDirectory: true, depth: 1, parent: '/w/.claude', open: true },
    ...Array.from({ length: 40 }, (_, index) => file(`${shots}/shot-${index}.png`, 2, shots)),
    ...tail,
  ]
  const folded: TreeRow[] = [
    ...head,
    { path: shots, name: 'screenshots', isDirectory: true, depth: 1, parent: '/w/.claude', open: false },
    ...tail,
  ]
  const topIn = (list: TreeRow[]) => (path: string) => caption + list.findIndex((row) => row.path === path) * TREE_ROW_PITCH_PX
  const view = { scrollTop: 12 * TREE_ROW_PITCH_PX, clientHeight: 220 }
  const slot = caption + TREE_ROW_PITCH_PX

  // Ten shots in, screenshots is pinned in the second slot, under .claude.
  expect(stickyTreeRows(open, topIn(open), view.scrollTop, caption).paths).toEqual(['/w/.claude', shots])
  // A click on it folds it; left where it was, the tree would show rows far below it.
  const rowTop = topIn(folded)(shots)
  const scrollTop = revealTreeRow(view, { depth: 1, top: rowTop }, caption) ?? view.scrollTop
  const stack = stickyTreeRows(folded, topIn(folded), scrollTop, caption)
  // Its row comes to rest where the pinned copy was, with .claude still pinned above it.
  expect(rowTop - scrollTop).toBe(slot)
  expect(stack.paths).toEqual(['/w/.claude'])
  expect(caption + stack.paths.length * TREE_ROW_PITCH_PX + stack.offset).toBeLessThanOrEqual(slot)
})

test('a directory\'s block runs from its row to the last row under it', () => {
  expect(treeBlock(rows, '/w/src')).toEqual({ first: 0, last: 4 })
  expect(treeBlock(rows, '/w/src/auth')).toEqual({ first: 1, last: 3 })
  // A closed directory, or a file, holds no rows.
  expect(treeBlock(rows, '/w/tests')).toEqual({ first: 5, last: 5 })
  expect(treeBlock(rows, '/w/gone')).toBeNull()
})
