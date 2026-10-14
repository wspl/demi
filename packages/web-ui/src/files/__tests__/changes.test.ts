import { describe, expect, test } from 'bun:test'
import { changeTreeRows, requestTreeFiles, type ChangeTreeFile } from '../changes'

// Cost: pure, under a millisecond.

const file = (path: string, key = path): ChangeTreeFile => ({ key, path, kind: 'modified' })

const files: ChangeTreeFile[] = [
  file('src/web/app/page.ts'),
  file('src/web/app/form.ts'),
  file('README.md'),
  file('src/auth/sid.ts'),
  file('src/auth/cookie.ts'),
  file('docs/guide/setup/intro.md'),
  file('src/main.ts'),
]

const shown = (rows: ReturnType<typeof changeTreeRows>) =>
  rows.map((row) => `${row.isDirectory ? 'd' : 'f'}${row.depth}:${row.name}${row.isDirectory && !row.open ? ' (folded)' : ''}`)

describe('the change tree', () => {
  test('lists folders first, then files, each by name, and joins a folder whose only content is one folder', () => {
    expect(shown(changeTreeRows(files, new Set()))).toEqual([
      'd0:docs/guide/setup',
      'f1:intro.md',
      'd0:src',
      'd1:auth',
      'f2:cookie.ts',
      'f2:sid.ts',
      'd1:web/app',
      'f2:form.ts',
      'f2:page.ts',
      'f1:main.ts',
      'f0:README.md',
    ])
  })

  test('a row knows its folder, a joined folder goes by its whole path, and a file by its key', () => {
    const rows = changeTreeRows([file('src/web/app/page.ts', '/w/src/web/app/page.ts'), file('src/main.ts', '/w/src/main.ts')], new Set())
    expect(rows.map((row) => [row.path, row.parent])).toEqual([
      ['src', null],
      ['src/web/app', 'src'],
      ['/w/src/web/app/page.ts', 'src/web/app'],
      ['/w/src/main.ts', 'src'],
    ])
    expect(rows[2]!.file?.key).toBe('/w/src/web/app/page.ts')
    expect(rows[0]!.file).toBeNull()
  })

  test('a folded folder keeps its row, closed, and hides what is under it', () => {
    expect(shown(changeTreeRows(files, new Set(['src/web/app', 'docs/guide/setup'])))).toEqual([
      'd0:docs/guide/setup (folded)',
      'd0:src',
      'd1:auth',
      'f2:cookie.ts',
      'f2:sid.ts',
      'd1:web/app (folded)',
      'f1:main.ts',
      'f0:README.md',
    ])
  })
})

describe('a request\u2019s files in the change tree', () => {
  test('on a Windows Host sit in their folders under the workspace', () => {
    const files = requestTreeFiles('C:\\Users\\u\\shop', [
      { path: 'C:\\Users\\u\\shop\\src\\cart.ts', kind: 'modified', edits: [] },
      { path: 'C:\\Users\\u\\shop\\README.md', kind: 'added', edits: [] },
    ])
    expect(shown(changeTreeRows(files, new Set()))).toEqual(['d0:src', 'f1:cart.ts', 'f0:README.md'])
    expect(files[0]!.key).toBe('C:\\Users\\u\\shop\\src\\cart.ts')
  })
})
