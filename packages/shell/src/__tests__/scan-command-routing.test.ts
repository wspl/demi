import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { BashEnvironment } from '../bash'
import { LocalHost } from '@demicodes/host-local'

test('grep routes to the real host binary when one exists', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-scan-route-'))
  await writeFile(join(root, 'notes.txt'), 'alpha\nbravo\n')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({ script: `grep -n bravo ${join(root, 'notes.txt')}`, timeoutMs: 10_000 })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(0)
  expect(result.stdout.delta).toBe('2:bravo\n')
  expect(result.audit.map((event) => event.kind)).toContain('system-command')
})

test('grep falls back to the portable implementation when the host has no binary', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-scan-fallback-'))
  await writeFile(join(root, 'notes.txt'), 'alpha\nbravo\n')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    initialEnv: { PATH: `${root}/no-binaries-here` },
  })

  const result = await env.exec({ script: `grep -n bravo ${join(root, 'notes.txt')}`, timeoutMs: 10_000 })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(0)
  expect(result.stdout.delta).toBe('2:bravo\n')
  expect(result.audit.map((event) => event.kind)).toContain('portable-command')
})

test('grep with cwd deleted still uses the host binary', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-scan-cwd-'))
  const dir = join(root, 'gone')
  await mkdir(dir)
  await writeFile(join(dir, 'notes.txt'), 'alpha\nbravo\n')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({
    script: `cd gone && rm -rf "$PWD" && grep -n bravo notes.txt`,
    timeoutMs: 10_000,
  })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.audit.filter((event) => event.kind === 'portable-command' && event.name === 'grep')).toEqual([])
  expect(result.audit.map((event) => event.kind)).toContain('system-command')
})
