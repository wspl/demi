import { describe, expect, test } from 'bun:test'
import { TREE_ROW_PITCH_PX, stickyTreeRows, type FileTreeRow } from '../file-tree'

// src/, src/auth/, src/auth/cookie.ts, src/auth/session.ts, src/index.ts, tests/
const rows: FileTreeRow[] = [
  { path: '/w/src', name: 'src', isDirectory: true, depth: 0, parent: null },
  { path: '/w/src/auth', name: 'auth', isDirectory: true, depth: 1, parent: '/w/src' },
  { path: '/w/src/auth/cookie.ts', name: 'cookie.ts', isDirectory: false, depth: 2, parent: '/w/src/auth' },
  { path: '/w/src/auth/session.ts', name: 'session.ts', isDirectory: false, depth: 2, parent: '/w/src/auth' },
  { path: '/w/src/index.ts', name: 'index.ts', isDirectory: true, depth: 1, parent: '/w/src' },
  { path: '/w/tests', name: 'tests', isDirectory: true, depth: 0, parent: null },
]
const caption = 28
const top = (path: string) => caption + rows.findIndex((row) => row.path === path) * TREE_ROW_PITCH_PX
const selected = '/w/src/auth/session.ts'

describe('sticky tree rows', () => {
  test('nothing pins at the top, or without a selected file', () => {
    expect(stickyTreeRows(rows, top, 0, caption, selected)).toEqual([])
    expect(stickyTreeRows(rows, top, 5 * TREE_ROW_PITCH_PX, caption, null)).toEqual([])
    expect(stickyTreeRows(rows, top, 5 * TREE_ROW_PITCH_PX, caption, '/w/none')).toEqual([])
  })

  test("a selected file's directory pins once its row scrolls under the stack", () => {
    // One pixel in: src's row is under the caption and auth's under the pinned src, so both pin.
    expect(stickyTreeRows(rows, top, 1, caption, selected)).toEqual(['/w/src', '/w/src/auth'])
    // With the tree taller between them, auth's row is still clear of the stack: only src pins.
    const spaced = (path: string) => (path === '/w/src' ? caption : caption + 3 * TREE_ROW_PITCH_PX)
    expect(stickyTreeRows(rows, spaced, 1, caption, selected)).toEqual(['/w/src'])
  })

  test('the stack keeps the selected path even past the file, and ignores other directories', () => {
    expect(stickyTreeRows(rows, top, 5 * TREE_ROW_PITCH_PX, caption, selected)).toEqual(['/w/src', '/w/src/auth'])
    // With index.ts selected, auth never pins however far the tree scrolls.
    expect(stickyTreeRows(rows, top, 3 * TREE_ROW_PITCH_PX, caption, '/w/src/index.ts')).toEqual(['/w/src'])
  })
})
