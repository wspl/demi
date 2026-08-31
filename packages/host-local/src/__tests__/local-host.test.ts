import { expect, test } from 'bun:test'
import { mkdir, mkdtemp, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { LocalHost } from '../local-host'

test('LocalHost spawns a command and captures stdout', async () => {
  const host = new LocalHost(process.cwd())
  const handle = await host.process.spawn({ command: 'printf', args: ['hello\\n'] })

  const [stdout, exit] = await Promise.all([collectText(handle.stdout), handle.wait()])

  expect(stdout).toBe('hello\n')
  expect(exit).toEqual({ exitCode: 0, signal: undefined })
})

test('LocalHost writes stdin to a spawned process', async () => {
  const host = new LocalHost(process.cwd())
  const handle = await host.process.spawn({
    command: 'sh',
    args: ['-c', 'IFS= read -r line; printf "%s" "$line"'],
  })

  await handle.writeStdin(Buffer.from('from stdin\n'))
  await handle.closeStdin()
  const [stdout, exit] = await Promise.all([collectText(handle.stdout), handle.wait()])

  expect(stdout).toBe('from stdin')
  expect(exit.exitCode).toBe(0)
})

test('LocalHost can terminate a foreground process', async () => {
  const host = new LocalHost(process.cwd())
  const handle = await host.process.spawn({ command: 'sleep', args: ['10'] })

  await handle.kill()
  const exit = await handle.wait()

  expect(exit.exitCode).toBeNull()
  expect(exit.signal).toBe('SIGTERM')
})

test('LocalHost.fs supports local file operations', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-local-host-fs-'))
  const host = new LocalHost(root)

  await host.fs.mkdir('src', { cwd: root, recursive: true })
  await host.fs.writeFile('src/file.txt', new TextEncoder().encode('hello\n'), { cwd: root })
  await host.fs.appendFile('src/file.txt', new TextEncoder().encode('tail\n'), { cwd: root })

  expect(new TextDecoder().decode(await host.fs.readFile('src/file.txt', { cwd: root }))).toBe('hello\ntail\n')
  const entries = await host.fs.readdir('src', { cwd: root })
  expect(entries).toEqual(['file.txt'])
  const typedEntries = await host.fs.readdir('src', { cwd: root, withFileTypes: true })
  expect(typedEntries[0]).toMatchObject({ name: 'file.txt', isFile: true })
  const stat = await host.fs.stat('src/file.txt', { cwd: root })
  expect(stat.isFile).toBe(true)
  expect(stat.size).toBe('hello\ntail\n'.length)

  await host.fs.rm('src/file.txt', { cwd: root, force: true })
  expect(await host.fs.exists('src/file.txt', { cwd: root })).toBe(false)
})

test('LocalHost children receive only the env passed to spawn', async () => {
  const host = new LocalHost(process.cwd())
  const handle = await host.process.spawn({
    command: 'printenv',
    args: ['HOME'],
    env: { PATH: '/usr/bin:/bin' },
  })
  const [stdout, exit] = await Promise.all([collectText(handle.stdout), handle.wait()])
  expect(exit.exitCode).toBe(1)
  expect(stdout).toBe('')
})

test('LocalHost classifies a missing binary as executable_not_found', async () => {
  const host = new LocalHost(process.cwd())
  const handle = await host.process.spawn({
    command: 'definitely-not-a-local-host-binary',
    env: { PATH: '/usr/bin:/bin' },
  })
  const exit = await handle.wait()
  expect(exit.exitCode).toBeNull()
  expect(exit.spawnError?.kind).toBe('executable_not_found')
})

test('LocalHost classifies a missing cwd path as cwd_unusable, not a missing binary', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-local-cwd-miss-'))
  const dir = join(root, 'gone')
  await mkdir(dir)
  await rm(dir, { recursive: true, force: true })
  const host = new LocalHost(root)
  const handle = await host.process.spawn({
    command: '/bin/echo',
    args: ['ok'],
    cwd: dir,
    env: { PATH: '/usr/bin:/bin' },
  })
  const exit = await handle.wait()
  expect(exit.exitCode).toBeNull()
  expect(exit.spawnError?.kind).toBe('cwd_unusable')
})

test('LocalHost missing binary with a live dirfd is executable_not_found', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-local-cwd-missbin-'))
  const dir = join(root, 'keep')
  await mkdir(dir)
  const cwd = await hostCwd(root, dir)
  const host = new LocalHost(root)
  const handle = await host.process.spawn({
    command: 'definitely-not-a-local-host-binary',
    cwd: cwd.spawnPath(),
    env: { PATH: '/usr/bin:/bin' },
  })
  const exit = await handle.wait()
  expect(exit.spawnError?.kind).toBe('executable_not_found')
  await cwd.close()
})

test('LocalHost spawn after unlinking cwd follows the platform cwd anchor', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-local-cwd-'))
  const dir = join(root, 'gone')
  await mkdir(dir)
  const cwd = await hostCwd(root, dir)
  await rm(dir, { recursive: true, force: true })
  const host = new LocalHost(root)
  const handle = await host.process.spawn({
    command: '/bin/echo',
    args: ['ok'],
    cwd: cwd.spawnPath(),
    env: { PATH: '/usr/bin:/bin' },
  })
  const [stdout, exit] = await Promise.all([collectText(handle.stdout), handle.wait()])
  if (process.platform === 'linux') {
    // The /proc/self/fd anchor keeps the unlinked directory reachable.
    expect(exit.exitCode).toBe(0)
    expect(stdout).toBe('ok\n')
  } else {
    // Without a dirfd anchor (macOS devfs cannot traverse directory fds) the
    // deleted path is honestly unusable.
    expect(exit.exitCode).toBeNull()
    expect(exit.spawnError?.kind).toBe('cwd_unusable')
  }
  await cwd.close()
})

async function hostCwd(root: string, dir: string) {
  const host = new LocalHost(root)
  return host.process.openCwd(dir)
}

async function collectText(iterable: AsyncIterable<Uint8Array>): Promise<string> {
  const chunks: Uint8Array[] = []
  for await (const chunk of iterable) chunks.push(chunk)
  return Buffer.concat(chunks).toString('utf8')
}
