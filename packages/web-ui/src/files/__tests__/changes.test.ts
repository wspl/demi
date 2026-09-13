import { describe, expect, test } from 'bun:test'
import type { ShellFileChange } from '@demicodes/agent'
import { changeTotals, changeTreeRows } from '../changes'

const files: ShellFileChange[] = [
  { path: 'src/auth/cookie.ts', kind: 'modified', added: 12, removed: 3 },
  { path: 'src/auth/session.ts', kind: 'added', added: 20, removed: 0 },
  { path: 'README.md', kind: 'modified', added: 1, removed: 1 },
  { path: 'src/auth/sid.ts', kind: 'deleted', added: 0, removed: 15 },
]

describe('change set', () => {
  test('totals add up every file', () => {
    expect(changeTotals(files)).toEqual({ added: 33, removed: 19 })
  })

  test('rows are a tree, directories first, sorted, files at the root last', () => {
    const rows = changeTreeRows(files, new Set())
    expect(rows.map((row) => `${row.kind === 'directory' ? 'd' : 'f'}${row.depth}:${row.path}`)).toEqual([
      'd0:src',
      'd1:src/auth',
      'f2:src/auth/cookie.ts',
      'f2:src/auth/session.ts',
      'f2:src/auth/sid.ts',
      'f0:README.md',
    ])
  })

  test('a folded directory keeps its row and hides what is under it', () => {
    const rows = changeTreeRows(files, new Set(['src/auth']))
    expect(rows.map((row) => row.path)).toEqual(['src', 'src/auth', 'README.md'])
  })
})
