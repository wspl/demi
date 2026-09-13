import { describe, expect, test } from 'bun:test'
import { TREE_ROW_PITCH_PX, TREE_ROW_PX, stickyTreeRows, type TreeRow } from '../tree'

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
