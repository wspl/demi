import { expect, test } from 'bun:test'
import { codePath } from '../code-paths'
import { targetLine } from '../filePath'
import { replyCodeFiles } from '../code-files'
import type { MessageFiles } from '../types'

// Inline code that names a file, and a link's line. Pure, a few milliseconds.

test('inline code reads as a path when it has a slash or is a name with an extension, with an optional line', () => {
  const read = (code: string) => codePath(code)
  expect(read('src/app.ts')).toEqual({ path: 'src/app.ts', line: null })
  expect(read('src/app.ts:42')).toEqual({ path: 'src/app.ts', line: 42 })
  expect(read('/etc/hosts')).toEqual({ path: '/etc/hosts', line: null })
  expect(read('README.md')).toEqual({ path: 'README.md', line: null })
  expect(read('./scripts/plot.py')).toEqual({ path: './scripts/plot.py', line: null })
  expect(read('../shared/a.md:7')).toEqual({ path: '../shared/a.md', line: 7 })
  expect(read('C:/Users/alex/a.txt')).toEqual({ path: 'C:/Users/alex/a.txt', line: null })
  expect(read('app/[id]/page.tsx')).toEqual({ path: 'app/[id]/page.tsx', line: null })
  for (const code of [
    'runner', 'a b/c', 'https://x', 'https://example.com/a.md', 'foo.', 'src/', '--force', '*.ts',
    'npm test', 'v1.2.3', '.env', '~/notes.md', '$HOME/a.md', 'a.ts:x', 'a:b/c', 'src/app.ts:0', '', '/', '..',
  ])
    expect([code, read(code)]).toEqual([code, null])
})

test('a link points at a line with #L or a :line, the first of a range', () => {
  const cases: [string, number | null][] = [
    ['docs/execution/runner.md#L438', 438],
    ['docs/a.md#L42-L50', 42],
    ['/etc/hosts#L3', 3],
    ['file:///tmp/a.txt#L9', 9],
    ['scripts/plot.py:12', 12],
    ['scripts/plot.py:12-20', 12],
    ['docs/a.md#usage', null],
    ['docs/a.md', null],
    ['docs/a.md#L0', null],
  ]
  for (const [target, line] of cases)
    expect([target, targetLine(target)]).toEqual([target, line])
})

test('a reply asks which of its code paths are files once it is complete, all in one question', () => {
  const asked: (readonly string[])[] = []
  const files: MessageFiles = {
    cwd: '/work',
    open: () => {},
    codeFiles: (paths) => {
      asked.push(paths)
      return null
    },
  }
  const reply = 'Edit `src/app.ts:42` and `README.md`, not `runner` or `npm test`; see `/etc/hosts` and `src/app.ts` again.'
  // While it streams, nothing is asked, however much has arrived.
  expect(replyCodeFiles(reply.slice(0, 30), true, files)).toBeNull()
  expect(replyCodeFiles(reply, true, files)).toBeNull()
  expect(asked).toEqual([])
  replyCodeFiles(reply, false, files)
  expect(asked).toEqual([['/work/src/app.ts', '/work/README.md', '/etc/hosts']])
  // A reply that names no path asks nothing.
  replyCodeFiles('Run `npm test` in `runner`.', false, files)
  expect(asked).toHaveLength(1)
})
