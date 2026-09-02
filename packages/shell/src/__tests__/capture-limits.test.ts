import { mkdtemp, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { BashEnvironment } from '../bash'
import { HostBackedFileSystem } from '../host-fs'
import { LocalHost } from '@demicodes/host-local'

function makeEnv(root: string, shellId: string, extra: { maxCaptureBytes?: number; maxOutputBytes?: number } = {}): BashEnvironment {
  return new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => shellId,
    initialEnv: { PATH: process.env.PATH ?? '' },
    ...extra,
  })
}

test('a foreground process over the capture limit is killed and fails loudly', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-capture-fg-'))
  const env = makeEnv(root, 'shell-capture-fg', { maxCaptureBytes: 64 * 1024 })

  const result = await env.exec({
    script: `sh -c 'yes 0123456789abcdef | head -c 400000; true'`,
    timeoutMs: 30_000,
  })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(137)
  expect(result.stderr.delta).toContain('capture limit')
  expect(result.stdout.delta).toBe('')
})

test('a foreground process within the capture limit is untouched', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-capture-ok-'))
  const env = makeEnv(root, 'shell-capture-ok', { maxCaptureBytes: 64 * 1024 })

  const result = await env.exec({ script: `sh -c 'printf hello'`, timeoutMs: 30_000 })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(0)
  expect(result.stdout.delta).toBe('hello')
})

test('a chatty background job keeps only the retained tail and says what it dropped', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-capture-bg-'))
  const env = makeEnv(root, 'shell-capture-bg', { maxOutputBytes: 512 })

  const started = await env.exec({
    script: `sh -c 'yes 0123456789abcdef | head -c 100000' &`,
    timeoutMs: 30_000,
  })
  expect(started.status).toBe('exited')

  const waited = await env.exec({ shellId: started.shellId, script: 'wait %1', timeoutMs: 30_000 })
  expect(waited.status).toBe('exited')
  if (waited.status !== 'exited') throw new Error('expected exited result')
  expect(waited.stdout.delta).toContain('dropped')
  expect(waited.stdout.delta.length).toBeLessThan(1024)
})

test('an in-shell file read over the limit fails with an explicit error', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-capture-read-'))
  await writeFile(join(root, 'big.txt'), 'x'.repeat(100))
  const fs = new HostBackedFileSystem(new LocalHost(root), { maxFileReadBytes: 8 })

  await expect(fs.readFile(join(root, 'big.txt'))).rejects.toThrow('in-shell read limit')
})

test('an in-shell file read within the limit is untouched', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-capture-read-ok-'))
  await writeFile(join(root, 'small.txt'), 'ok\n')
  const fs = new HostBackedFileSystem(new LocalHost(root), { maxFileReadBytes: 8 })

  expect(await fs.readFile(join(root, 'small.txt'))).toBe('ok\n')
})
