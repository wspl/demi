import { expect, test } from 'bun:test'
import { mkdtemp } from 'node:fs/promises'
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

async function collectText(iterable: AsyncIterable<Uint8Array>): Promise<string> {
  const chunks: Uint8Array[] = []
  for await (const chunk of iterable) chunks.push(chunk)
  return Buffer.concat(chunks).toString('utf8')
}
