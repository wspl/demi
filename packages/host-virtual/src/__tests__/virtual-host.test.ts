import { mkdtemp, symlink as realSymlink, writeFile as realWriteFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'
import { expect, test } from 'bun:test'
import { LocalHost } from '@demicodes/host-local'
import { BashEnvironment } from '@demicodes/shell'
import { memoryHostStore } from '@demicodes/testkit'
import { VIRTUAL_UPGRADE_GUIDANCE, VirtualHost, scopedFsBackend } from '../index'

const text = (value: string) => new TextEncoder().encode(value)
const read = async (host: VirtualHost, path: string) => new TextDecoder().decode(await host.fs.readFile(path))

async function makeHost(quota?: { maxFileBytes?: number; maxTotalBytes?: number }) {
  const realRoot = await mkdtemp(join(tmpdir(), 'demi-virtual-'))
  const local = new LocalHost(realRoot)
  const host = new VirtualHost({
    backend: scopedFsBackend(realRoot, local.fs),
    store: memoryHostStore(),
    quota,
  })
  await host.ensureLayout()
  return { host, realRoot }
}

test('virtual namespace: paths resolve against the virtual root, never the real one', async () => {
  const { host, realRoot } = await makeHost()

  await host.fs.writeFile('/workspace/notes.txt', text('hi'))
  expect(await read(host, '/workspace/notes.txt')).toBe('hi')
  // Relative paths resolve against the virtual default cwd.
  expect(await read(host, 'notes.txt')).toBe('hi')
  // cwd option is a virtual path.
  await host.fs.mkdir('/workspace/sub')
  await host.fs.writeFile('inner.txt', text('deep'), { cwd: '/workspace/sub' })
  expect(await read(host, '/workspace/sub/inner.txt')).toBe('deep')

  // Escapes clamp to the virtual root (chroot semantics): this reads the
  // namespace's own /etc, not the machine's.
  await host.fs.mkdir('/etc')
  await host.fs.writeFile('/etc/passwd', text('virtual-passwd'))
  expect(await read(host, '/../../../../etc/passwd')).toBe('virtual-passwd')

  // realpath answers in virtual terms; the real root never leaks.
  const resolved = await host.fs.realpath('/workspace/sub/../notes.txt')
  expect(resolved).toBe('/workspace/notes.txt')
  expect(resolved.includes(realRoot)).toBe(false)

  const entries = await host.fs.readdir('/')
  expect(entries.sort()).toEqual(['.artifacts', 'etc', 'workspace'])
})

test('symlinks stay inside the namespace', async () => {
  const { host, realRoot } = await makeHost()

  // Absolute targets are virtual-absolute: written translated, read back virtual.
  await host.fs.writeFile('/workspace/target.txt', text('linked'))
  await host.fs.symlink('/workspace/target.txt', '/workspace/link')
  expect(await host.fs.readlink('/workspace/link')).toBe('/workspace/target.txt')
  expect(await read(host, '/workspace/link')).toBe('linked')

  // Relative targets that walk above the root are refused.
  await expect(host.fs.symlink('../../../etc/passwd', '/workspace/escape')).rejects.toThrow('escapes the virtual workspace')

  // A pre-existing real symlink pointing outside the root cannot be resolved to a virtual path.
  await realWriteFile(join(realRoot, '..', `outside-${process.pid}.txt`), 'outside')
  await realSymlink(join(realRoot, '..', `outside-${process.pid}.txt`), join(realRoot, 'workspace', 'sneaky'))
  await expect(host.fs.realpath('/workspace/sneaky')).rejects.toThrow('outside the virtual workspace')
})

test('quota: per-file and per-conversation caps; artifacts excluded', async () => {
  const { host } = await makeHost({ maxFileBytes: 10, maxTotalBytes: 25 })

  await host.fs.writeFile('/workspace/a.txt', text('1234567890'))
  await expect(host.fs.writeFile('/workspace/big.txt', text('12345678901'))).rejects.toThrow('per-file limit')
  await expect(host.fs.appendFile('/workspace/a.txt', text('x'))).rejects.toThrow('per-file limit')

  await host.fs.writeFile('/workspace/b.txt', text('1234567890'))
  expect(await host.usage()).toBe(20)
  await expect(host.fs.writeFile('/workspace/c.txt', text('1234567890'))).rejects.toThrow('total size limit')
  // Overwriting counts growth, not gross size.
  await host.fs.writeFile('/workspace/b.txt', text('123456789A'))
  // Artifact writes are never quota-gated.
  await host.fs.writeFile('/.artifacts/cmd-1/stdout.txt', text('this artifact write is larger than caps'), {
    createParents: true,
  })
  expect(await host.usage()).toBe(20)
})

test('spawn refuses with executable_not_found and upgrade guidance', async () => {
  const { host } = await makeHost()
  const handle = await host.process.spawn({ command: 'python3', args: ['-V'] })
  const exit = await handle.wait()
  expect(exit.exitCode).toBeNull()
  expect(exit.spawnError?.kind).toBe('executable_not_found')
  expect(exit.spawnError?.detail).toBe(VIRTUAL_UPGRADE_GUIDANCE)
})

test('BashEnvironment on a virtual host: portable commands work, real programs surface guidance', async () => {
  const { host } = await makeHost()
  const env = new BashEnvironment({
    host,
    shellIdFactory: () => 'virtual-shell',
    initialEnv: { PATH: '/usr/bin:/bin' },
  })

  const portable = await env.exec({ script: 'printf hello > f.txt && cat f.txt && ls' })
  expect(portable.status).toBe('exited')
  if (portable.status !== 'exited') throw new Error('expected exited')
  expect(portable.exitCode).toBe(0)
  expect(portable.stdout.delta).toBe('hellof.txt\n')

  const real = await env.exec({ script: 'python3 -V' })
  expect(real.status).toBe('exited')
  if (real.status !== 'exited') throw new Error('expected exited')
  expect(real.exitCode).not.toBe(0)
  expect(real.stderr.delta).toContain('command not found')
  expect(real.stderr.delta).toContain(VIRTUAL_UPGRADE_GUIDANCE)
})
