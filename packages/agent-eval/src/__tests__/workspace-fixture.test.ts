import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import {
  diffWorkspace,
  matchesPathPattern,
  prepareWorkspace,
  renderWorkspaceDiff,
  snapshotWorkspace,
} from '../workspace'

test('fixtures copy fresh with ignore rules applied', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'demi-fixture-'))
  await mkdir(join(fixture, 'src'), { recursive: true })
  await mkdir(join(fixture, 'node_modules', 'dep'), { recursive: true })
  await writeFile(join(fixture, 'src', 'app.ts'), 'export {}')
  await writeFile(join(fixture, 'node_modules', 'dep', 'index.js'), 'x')

  const workspace = await prepareWorkspace(fixture, ['node_modules'])
  expect(await readFile(join(workspace, 'src', 'app.ts'), 'utf8')).toBe('export {}')
  const snapshot = await snapshotWorkspace(workspace)
  expect([...snapshot.files.keys()]).toEqual(['src/app.ts'])

  // A second prepare is isolated from the first.
  const workspace2 = await prepareWorkspace(fixture, ['node_modules'])
  await writeFile(join(workspace2, 'src', 'app.ts'), 'mutated')
  expect(await readFile(join(workspace, 'src', 'app.ts'), 'utf8')).toBe('export {}')
})

test('workspace diff reports added, changed, and removed files', async () => {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-diff-'))
  await writeFile(join(workspace, 'a.txt'), 'one')
  await writeFile(join(workspace, 'b.txt'), 'two')
  const before = await snapshotWorkspace(workspace)

  await writeFile(join(workspace, 'a.txt'), 'changed')
  await writeFile(join(workspace, 'c.txt'), 'new')
  const { rm } = await import('node:fs/promises')
  await rm(join(workspace, 'b.txt'))
  const after = await snapshotWorkspace(workspace)

  const diff = diffWorkspace(before, after)
  expect(diff).toEqual([
    { path: 'a.txt', status: 'changed' },
    { path: 'b.txt', status: 'removed' },
    { path: 'c.txt', status: 'added' },
  ])
  const rendered = renderWorkspaceDiff(diff)
  expect(rendered).toContain('changed a.txt')
  expect(rendered).toContain('removed b.txt')
})

test('path patterns support directory prefixes and globs', () => {
  expect(matchesPathPattern('src/app.ts', 'src')).toBe(true)
  expect(matchesPathPattern('src/app.ts', 'src/app.ts')).toBe(true)
  expect(matchesPathPattern('srcx/app.ts', 'src')).toBe(false)
  expect(matchesPathPattern('logs/run.log', '**/*.log')).toBe(true)
  expect(matchesPathPattern('run.log', '*.log')).toBe(true)
  expect(matchesPathPattern('nested/run.log', '*.log')).toBe(false)
  expect(matchesPathPattern('a/b/c.txt', 'a/**')).toBe(true)
})
