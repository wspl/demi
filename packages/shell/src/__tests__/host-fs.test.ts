import { mkdir, mkdtemp, writeFile, readFile as fsReadFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import type { Host, HostSpawnHandle, HostSpawnParams } from '../host'
import { HostBackedFileSystem } from '../host-fs'
import { LocalHost } from '../local-host'

test('HostBackedFileSystem.readFile reads file content via Host.spawn cat', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-read-'))
  await writeFile(join(root, 'hello.txt'), 'hello world\n')
  const fs = new HostBackedFileSystem(new LocalHost(root))
  const content = await fs.readFile(join(root, 'hello.txt'))
  expect(content).toBe('hello world\n')
})

test('HostBackedFileSystem.exists returns true for existing files and false for missing', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-exists-'))
  await writeFile(join(root, 'present.txt'), 'x\n')
  const fs = new HostBackedFileSystem(new LocalHost(root))
  expect(await fs.exists(join(root, 'present.txt'))).toBe(true)
  expect(await fs.exists(join(root, 'missing.txt'))).toBe(false)
})

test('HostBackedFileSystem.stat reports isFile/isDirectory via Host.spawn test', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-stat-'))
  await writeFile(join(root, 'file.txt'), 'x\n')
  await mkdir(join(root, 'subdir'))
  const fs = new HostBackedFileSystem(new LocalHost(root))
  const fileStat = await fs.stat(join(root, 'file.txt'))
  expect(fileStat.isFile).toBe(true)
  expect(fileStat.isDirectory).toBe(false)
  const dirStat = await fs.stat(join(root, 'subdir'))
  expect(dirStat.isFile).toBe(false)
  expect(dirStat.isDirectory).toBe(true)
})

test('HostBackedFileSystem.writeFile writes content via Host.spawn tee', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-write-'))
  const fs = new HostBackedFileSystem(new LocalHost(root))
  await fs.writeFile(join(root, 'out.txt'), 'written bytes\n')
  const onDisk = await fsReadFile(join(root, 'out.txt'), 'utf8')
  expect(onDisk).toBe('written bytes\n')
})

test('HostBackedFileSystem.appendFile appends content via Host.spawn tee -a', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-append-'))
  await writeFile(join(root, 'log.txt'), 'first\n')
  const fs = new HostBackedFileSystem(new LocalHost(root))
  await fs.appendFile(join(root, 'log.txt'), 'second\n')
  const onDisk = await fsReadFile(join(root, 'log.txt'), 'utf8')
  expect(onDisk).toBe('first\nsecond\n')
})

test('HostBackedFileSystem.readdir lists directory entries via Host.spawn ls', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-readdir-'))
  await writeFile(join(root, 'a.txt'), 'a\n')
  await writeFile(join(root, 'b.txt'), 'b\n')
  await mkdir(join(root, 'sub'))
  const fs = new HostBackedFileSystem(new LocalHost(root))
  const entries = await fs.readdir(join(root))
  expect(entries.sort()).toEqual(['a.txt', 'b.txt', 'sub'])
})

test('HostBackedFileSystem.resolvePath joins base and absolute/relative paths', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-resolve-'))
  const fs = new HostBackedFileSystem(new LocalHost(root))
  expect(fs.resolvePath('/some/cwd', 'relative/path')).toBe('/some/cwd/relative/path')
  expect(fs.resolvePath('/some/cwd', '/absolute/path')).toBe('/absolute/path')
})

test('HostBackedFileSystem.readFileBuffer returns raw bytes via Host.spawn cat', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-readbuf-'))
  await writeFile(join(root, 'bytes.bin'), new Uint8Array([0x68, 0x69, 0x0a]))
  const fs = new HostBackedFileSystem(new LocalHost(root))
  const bytes = await fs.readFileBuffer(join(root, 'bytes.bin'))
  expect(Array.from(bytes)).toEqual([0x68, 0x69, 0x0a])
})

test('HostBackedFileSystem routes file operations through Host.spawn', async () => {
  const host = new RecordingFsHost('/workspace')
  const fs = new HostBackedFileSystem(host)

  expect(await fs.readFile('/workspace/read.txt')).toBe('read\n')
  expect(Array.from(await fs.readFileBuffer('/workspace/blob.bin'))).toEqual([0x00, 0xff, 0x41])
  expect(await fs.exists('/workspace/read.txt')).toBe(true)
  expect(await fs.exists('/workspace/missing.txt')).toBe(false)
  expect((await fs.stat('/workspace/read.txt')).isFile).toBe(true)
  expect((await fs.stat('/workspace')).isDirectory).toBe(true)

  await fs.writeFile('/workspace/out.txt', 'new\n')
  await fs.appendFile('/workspace/out.txt', 'tail\n')
  expect(await fs.readdir('/workspace')).toEqual(['blob.bin', 'out.txt', 'read.txt'])
  expect(host.fileText('/workspace/out.txt')).toBe('new\ntail\n')

  expect(host.calls.map((call) => ({ command: call.command, args: call.args, cwd: call.cwd, stdin: call.stdin }))).toEqual([
    { command: 'cat', args: ['--', '/workspace/read.txt'], cwd: '/workspace', stdin: '' },
    { command: 'cat', args: ['--', '/workspace/blob.bin'], cwd: '/workspace', stdin: '' },
    { command: 'test', args: ['-e', '/workspace/read.txt'], cwd: '/workspace', stdin: '' },
    { command: 'test', args: ['-e', '/workspace/missing.txt'], cwd: '/workspace', stdin: '' },
    { command: 'test', args: ['-f', '/workspace/read.txt'], cwd: '/workspace', stdin: '' },
    { command: 'test', args: ['-f', '/workspace'], cwd: '/workspace', stdin: '' },
    { command: 'test', args: ['-d', '/workspace'], cwd: '/workspace', stdin: '' },
    { command: 'tee', args: ['--', '/workspace/out.txt'], cwd: '/workspace', stdin: 'new\n' },
    { command: 'tee', args: ['-a', '--', '/workspace/out.txt'], cwd: '/workspace', stdin: 'tail\n' },
    { command: 'ls', args: ['-1', '-A', '--', '/workspace'], cwd: '/workspace', stdin: '' },
  ])
})

class RecordingFsHost implements Host {
  readonly calls: RecordedHostCall[] = []
  private readonly files = new Map<string, Uint8Array>([
    ['/workspace/read.txt', encode('read\n')],
    ['/workspace/blob.bin', new Uint8Array([0x00, 0xff, 0x41])],
  ])
  private readonly dirs = new Set(['/workspace'])

  constructor(readonly root: string) {}

  fileText(path: string): string {
    const content = this.files.get(path)
    return content ? decode(content) : ''
  }

  async spawn(params: HostSpawnParams): Promise<HostSpawnHandle> {
    const call: RecordedHostCall = {
      command: params.command,
      args: params.args ?? [],
      cwd: params.cwd,
      stdin: '',
    }
    this.calls.push(call)
    let result: Promise<RecordingCommandResult> | null = null
    const finish = async () => {
      result ??= Promise.resolve(this.run(call))
      return result
    }

    return {
      stdout: deferredBytes(async () => (await finish()).stdout),
      stderr: deferredBytes(async () => (await finish()).stderr),
      writeStdin: async (data) => {
        call.stdin += decode(data)
      },
      closeStdin: async () => {},
      kill: async () => {},
      wait: async () => ({ exitCode: (await finish()).exitCode }),
    }
  }

  private run(call: RecordedHostCall): RecordingCommandResult {
    if (call.command === 'cat') {
      const path = requiredArg(call.args, call.args.length - 1)
      const content = this.files.get(path)
      return {
        stdout: content ?? new Uint8Array(),
        stderr: content ? new Uint8Array() : encode(`cat: ${path}: No such file or directory\n`),
        exitCode: content ? 0 : 1,
      }
    }

    if (call.command === 'test') {
      const flag = requiredArg(call.args, 0)
      const path = requiredArg(call.args, 1)
      const exists =
        flag === '-e'
          ? this.files.has(path) || this.dirs.has(path)
          : flag === '-f'
            ? this.files.has(path)
            : flag === '-d'
              ? this.dirs.has(path)
              : flag === '-L'
                ? false
                : false
      return { stdout: new Uint8Array(), stderr: new Uint8Array(), exitCode: exists ? 0 : 1 }
    }

    if (call.command === 'tee') {
      const path = requiredArg(call.args, call.args.length - 1)
      const input = encode(call.stdin)
      const append = call.args.includes('-a')
      const current = append ? (this.files.get(path) ?? new Uint8Array()) : new Uint8Array()
      this.files.set(path, concat(current, input))
      return { stdout: input, stderr: new Uint8Array(), exitCode: 0 }
    }

    if (call.command === 'ls') {
      const path = requiredArg(call.args, call.args.length - 1)
      if (!this.dirs.has(path)) {
        return { stdout: new Uint8Array(), stderr: encode(`ls: cannot access '${path}'\n`), exitCode: 1 }
      }
      const names = [...this.files.keys()]
        .filter((entry) => entry.startsWith(`${path}/`))
        .map((entry) => entry.slice(path.length + 1))
        .filter((entry) => !entry.includes('/'))
        .sort()
      return { stdout: encode(`${names.join('\n')}\n`), stderr: new Uint8Array(), exitCode: 0 }
    }

    return { stdout: new Uint8Array(), stderr: encode(`unsupported command: ${call.command}\n`), exitCode: 127 }
  }
}

interface RecordedHostCall {
  command: string
  args: string[]
  cwd?: string
  stdin: string
}

interface RecordingCommandResult {
  stdout: Uint8Array
  stderr: Uint8Array
  exitCode: number
}

function requiredArg(args: string[], index: number): string {
  const value = args[index]
  if (value === undefined) throw new Error(`missing argument ${index}`)
  return value
}

async function* deferredBytes(read: () => Promise<Uint8Array>): AsyncIterable<Uint8Array> {
  const value = await read()
  if (value.byteLength > 0) yield value
}

function encode(text: string): Uint8Array {
  return new TextEncoder().encode(text)
}

function decode(bytes: Uint8Array): string {
  return new TextDecoder().decode(bytes)
}

function concat(left: Uint8Array, right: Uint8Array): Uint8Array {
  const bytes = new Uint8Array(left.byteLength + right.byteLength)
  bytes.set(left, 0)
  bytes.set(right, left.byteLength)
  return bytes
}
