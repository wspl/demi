import { describe, expect, test } from 'bun:test'
import { TREE_ROW_PITCH_PX, stickyTreeRows, type FileTreeRow } from '../file-tree'

// src/, src/auth/, src/auth/cookie.ts, src/auth/session.ts, src/index.ts, tests/
const rows: FileTreeRow[] = [
  { path: '/w/src', name: 'src', isDirectory: true, depth: 0, parent: null },
  { path: '/w/src/auth', name: 'auth', isDirectory: true, depth: 1, parent: '/w/src' },
  { path: '/w/src/auth/cookie.ts', name: 'cookie.ts', isDirectory: false, depth: 2, parent: '/w/src/auth' },
  { path: '/w/src/auth/session.ts', name: 'session.ts', isDirectory: false, depth: 2, parent: '/w/src/auth' },
  { path: '/w/src/index.ts', name: 'index.ts', isDirectory: false, depth: 1, parent: '/w/src' },
  { path: '/w/tests', name: 'tests', isDirectory: true, depth: 0, parent: null },
]
const caption = 28
const top = (path: string) => caption + rows.findIndex((row) => row.path === path) * TREE_ROW_PITCH_PX

describe('sticky tree rows', () => {
  test('nothing pins at the top', () => {
    expect(stickyTreeRows(rows, top, 0, caption).paths).toEqual([])
  })

  test('each level pins the directory at that depth above the row it covers', () => {
    // Scrolled so cookie.ts is under the caption: src pins over it, auth over session.ts.
    expect(stickyTreeRows(rows, top, 2 * TREE_ROW_PITCH_PX, caption).paths).toEqual(['/w/src', '/w/src/auth'])
    // Scrolled so the auth row itself is under the caption: it still pins, over cookie.ts.
    expect(stickyTreeRows(rows, top, 1 * TREE_ROW_PITCH_PX, caption).paths).toEqual(['/w/src', '/w/src/auth'])
  })

  test('a directory stays pinned while its last descendant is under the caption', () => {
    expect(stickyTreeRows(rows, top, 4 * TREE_ROW_PITCH_PX, caption).paths).toEqual(['/w/src'])
  })

  test('a level goes once the row it covers leaves its directory', () => {
    // Scrolled so session.ts is first: src and auth are candidates, but auth would
    // cover index.ts, which is outside auth, so only src pins.
    expect(stickyTreeRows(rows, top, 3 * TREE_ROW_PITCH_PX, caption).paths).toEqual(['/w/src'])
    // Scrolled past src entirely: nothing pins.
    expect(stickyTreeRows(rows, top, 5 * TREE_ROW_PITCH_PX, caption).paths).toEqual([])
  })

  test('the stack is pushed up as the deepest pinned directory scrolls out', () => {
    // src pinned alone. Its last row, index.ts, ends 11px above the stack's
    // bottom (the stack is a caption and a row with its gap), so the stack
    // rides up by that much.
    const scrollTop = 4 * TREE_ROW_PITCH_PX + 10
    const stack = stickyTreeRows(rows, top, scrollTop, caption)
    expect(stack.paths).toEqual(['/w/src'])
    expect(stack.offset).toBe(-11)
  })
})
