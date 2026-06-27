import { mkdir, mkdtemp, readFile as fsReadFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import type { Host, HostDirent, HostFileStat, HostFileSystem, HostStore } from '../host'
import { HostBackedFileSystem } from '../host-fs'
import { LocalHost } from '@demicodes/host-local'

test('HostBackedFileSystem.readFile reads file content via Host.fs', async () => {
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

test('HostBackedFileSystem.stat reports file and directory metadata via Host.fs', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-stat-'))
  await writeFile(join(root, 'file.txt'), 'x\n')
  await mkdir(join(root, 'subdir'))
  const fs = new HostBackedFileSystem(new LocalHost(root))
  const fileStat = await fs.stat(join(root, 'file.txt'))
  expect(fileStat.isFile).toBe(true)
  expect(fileStat.isDirectory).toBe(false)
  expect(fileStat.size).toBe(2)
  const dirStat = await fs.stat(join(root, 'subdir'))
  expect(dirStat.isFile).toBe(false)
  expect(dirStat.isDirectory).toBe(true)
})

test('HostBackedFileSystem.writeFile and appendFile write bytes via Host.fs', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-write-'))
  const fs = new HostBackedFileSystem(new LocalHost(root))
  await fs.writeFile(join(root, 'out.txt'), 'written bytes\n')
  await fs.appendFile(join(root, 'out.txt'), 'tail\n')
  const onDisk = await fsReadFile(join(root, 'out.txt'), 'utf8')
  expect(onDisk).toBe('written bytes\ntail\n')
})

test('HostBackedFileSystem.readdir lists directory entries via Host.fs', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-readdir-'))
  await writeFile(join(root, 'a.txt'), 'a\n')
  await writeFile(join(root, 'b.txt'), 'b\n')
  await mkdir(join(root, 'sub'))
  const fs = new HostBackedFileSystem(new LocalHost(root))
  const entries = await fs.readdir(join(root))
  expect(entries.sort()).toEqual(['a.txt', 'b.txt', 'sub'])
  const typed = await fs.readdirWithFileTypes(join(root))
  expect(typed.find((entry) => entry.name === 'sub')?.isDirectory).toBe(true)
})

test('HostBackedFileSystem.resolvePath joins base and absolute/relative paths', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-resolve-'))
  const fs = new HostBackedFileSystem(new LocalHost(root))
  expect(fs.resolvePath('/some/cwd', 'relative/path')).toBe('/some/cwd/relative/path')
  expect(fs.resolvePath('/some/cwd', '/absolute/path')).toBe('/absolute/path')
})

test('HostBackedFileSystem.readFileBuffer returns raw bytes via Host.fs', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-fs-readbuf-'))
  await writeFile(join(root, 'bytes.bin'), new Uint8Array([0x68, 0x69, 0x0a]))
  const fs = new HostBackedFileSystem(new LocalHost(root))
  const bytes = await fs.readFileBuffer(join(root, 'bytes.bin'))
  expect(Array.from(bytes)).toEqual([0x68, 0x69, 0x0a])
})

test('HostBackedFileSystem serves read-only virtual files under /@', async () => {
  const host = new RecordingHost('/workspace')
  const fs = new HostBackedFileSystem(host, {
    lookup: (path) => {
      if (path === '/@') return { kind: 'directory', entries: [dirent('commands', 'directory')] }
      if (path === '/@/commands') return { kind: 'directory', entries: [dirent('cmd-1', 'directory')] }
      if (path === '/@/commands/cmd-1') {
        return { kind: 'directory', entries: [dirent('stdout.txt', 'file'), dirent('stderr.txt', 'file')] }
      }
      if (path === '/@/commands/cmd-1/stdout.txt') return { kind: 'file', content: encode('virtual out\n') }
      if (path === '/@/commands/cmd-1/stderr.txt') return { kind: 'file', content: encode('virtual err\n') }
      return null
    },
  })

  expect(await fs.readFile('/@/commands/cmd-1/stdout.txt')).toBe('virtual out\n')
  expect(await fs.exists('/@/commands/cmd-1/stderr.txt')).toBe(true)
  expect((await fs.stat('/@/commands/cmd-1/stdout.txt')).size).toBe(12)
  expect(await fs.readdir('/@/commands/cmd-1')).toEqual(['stdout.txt', 'stderr.txt'])

  await fs.cp('/@/commands/cmd-1/stdout.txt', '/workspace/copied.txt')
  expect(host.fileText('/workspace/copied.txt')).toBe('virtual out\n')
  await expect(fs.writeFile('/@/commands/cmd-1/stdout.txt', 'nope')).rejects.toThrow('EROFS')
})

test('HostBackedFileSystem routes IFileSystem operations to Host.fs and never Host.process.spawn', async () => {
  const host = new RecordingHost('/workspace')
  const fs = new HostBackedFileSystem(host)

  expect(await fs.readFile('/workspace/read.txt')).toBe('read\n')
  expect(Array.from(await fs.readFileBuffer('/workspace/blob.bin'))).toEqual([0x00, 0xff, 0x41])
  expect(await fs.exists('/workspace/read.txt')).toBe(true)
  expect(await fs.exists('/workspace/missing.txt')).toBe(false)
  expect((await fs.stat('/workspace/read.txt')).isFile).toBe(true)
  expect((await fs.stat('/workspace')).isDirectory).toBe(true)

  await fs.writeFile('/workspace/out.txt', 'new\n')
  await fs.appendFile('/workspace/out.txt', 'tail\n')
  await fs.mkdir('/workspace/sub', { recursive: true })
  await fs.rm('/workspace/sub', { recursive: true, force: true })
  expect(await fs.readdir('/workspace')).toEqual(['blob.bin', 'out.txt', 'read.txt'])
  expect(host.fileText('/workspace/out.txt')).toBe('new\ntail\n')

  expect(host.processSpawnCalls).toBe(0)
  expect(host.fs.calls).toEqual([
    ['readFile', '/workspace/read.txt', '/workspace'],
    ['readFile', '/workspace/blob.bin', '/workspace'],
    ['exists', '/workspace/read.txt', '/workspace'],
    ['exists', '/workspace/missing.txt', '/workspace'],
    ['stat', '/workspace/read.txt', '/workspace'],
    ['stat', '/workspace', '/workspace'],
    ['writeFile', '/workspace/out.txt', '/workspace', 'new\n'],
    ['appendFile', '/workspace/out.txt', '/workspace', 'tail\n'],
    ['mkdir', '/workspace/sub', '/workspace', true],
    ['rm', '/workspace/sub', '/workspace', true, true],
    ['readdir', '/workspace', '/workspace', false],
  ])
})

class RecordingHost implements Host {
  readonly defaultCwd: string
  readonly fs: RecordingHostFileSystem
  readonly store: HostStore = new MemoryHostStore()
  processSpawnCalls = 0
  readonly process = {
    spawn: async (): Promise<never> => {
      this.processSpawnCalls += 1
      throw new Error('Host.process.spawn must not be used for filesystem operations')
    },
  }

  constructor(defaultCwd: string) {
    this.defaultCwd = defaultCwd
    this.fs = new RecordingHostFileSystem(defaultCwd)
  }

  fileText(path: string): string {
    const content = this.fs.files.get(path)
    return content ? decode(content) : ''
  }

}

function dirent(name: string, type: 'file' | 'directory') {
  return {
    name,
    isFile: type === 'file',
    isDirectory: type === 'directory',
    isSymbolicLink: false,
  }
}

class MemoryHostStore implements HostStore {
  async readJson<T>(): Promise<T | null> { return null }
  async writeJson<T>(): Promise<void> {}
  async delete(): Promise<void> {}
  async list(): Promise<string[]> { return [] }
}

class RecordingHostFileSystem implements HostFileSystem {
  readonly calls: unknown[][] = []
  readonly files = new Map<string, Uint8Array>([
    ['/workspace/read.txt', encode('read\n')],
    ['/workspace/blob.bin', new Uint8Array([0x00, 0xff, 0x41])],
  ])
  private readonly dirs = new Set(['/workspace'])

  constructor(private readonly root: string) {}

  async readFile(path: string, options?: { cwd?: string }): Promise<Uint8Array> {
    this.calls.push(['readFile', path, options?.cwd])
    const content = this.files.get(this.resolve(path, options?.cwd))
    if (!content) throw new Error(`ENOENT: ${path}`)
    return content
  }

  async writeFile(path: string, data: Uint8Array, options?: { cwd?: string }): Promise<void> {
    this.calls.push(['writeFile', path, options?.cwd, decode(data)])
    this.files.set(this.resolve(path, options?.cwd), data)
  }

  async appendFile(path: string, data: Uint8Array, options?: { cwd?: string }): Promise<void> {
    this.calls.push(['appendFile', path, options?.cwd, decode(data)])
    const target = this.resolve(path, options?.cwd)
    this.files.set(target, concat(this.files.get(target) ?? new Uint8Array(), data))
  }

  async exists(path: string, options?: { cwd?: string }): Promise<boolean> {
    this.calls.push(['exists', path, options?.cwd])
    const target = this.resolve(path, options?.cwd)
    return this.files.has(target) || this.dirs.has(target)
  }

  async stat(path: string, options?: { cwd?: string }): Promise<HostFileStat> {
    this.calls.push(['stat', path, options?.cwd])
    const target = this.resolve(path, options?.cwd)
    if (this.files.has(target)) return statFor({ isFile: true, size: this.files.get(target)?.byteLength ?? 0 })
    if (this.dirs.has(target)) return statFor({ isDirectory: true })
    throw new Error(`ENOENT: ${path}`)
  }

  async lstat(path: string, options?: { cwd?: string }): Promise<HostFileStat> {
    this.calls.push(['lstat', path, options?.cwd])
    return this.stat(path, options)
  }

  async readdir(path: string, options: { cwd?: string; withFileTypes: true }): Promise<HostDirent[]>
  async readdir(path: string, options?: { cwd?: string; withFileTypes?: false }): Promise<string[]>
  async readdir(path: string, options?: { cwd?: string; withFileTypes?: boolean }): Promise<string[] | HostDirent[]> {
    this.calls.push(['readdir', path, options?.cwd, options?.withFileTypes === true])
    const target = this.resolve(path, options?.cwd)
    if (!this.dirs.has(target)) throw new Error(`ENOTDIR: ${path}`)
    const names = [...this.files.keys()]
      .filter((entry) => entry.startsWith(`${target}/`))
      .map((entry) => entry.slice(target.length + 1))
      .filter((entry) => !entry.includes('/'))
      .sort()
    if (options?.withFileTypes) {
      return names.map((name) => ({ name, isFile: true, isDirectory: false, isSymbolicLink: false }))
    }
    return names
  }

  async mkdir(path: string, options?: { cwd?: string; recursive?: boolean }): Promise<void> {
    this.calls.push(['mkdir', path, options?.cwd, options?.recursive === true])
    this.dirs.add(this.resolve(path, options?.cwd))
  }

  async rm(path: string, options?: { cwd?: string; recursive?: boolean; force?: boolean }): Promise<void> {
    this.calls.push(['rm', path, options?.cwd, options?.recursive === true, options?.force === true])
    const target = this.resolve(path, options?.cwd)
    this.files.delete(target)
    this.dirs.delete(target)
  }

  async cp(): Promise<void> {}
  async mv(): Promise<void> {}
  async chmod(): Promise<void> {}
  async symlink(): Promise<void> {}
  async link(): Promise<void> {}
  async readlink(): Promise<string> { return '' }
  async realpath(path: string, options?: { cwd?: string }): Promise<string> { return this.resolve(path, options?.cwd) }
  async utimes(): Promise<void> {}

  private resolve(path: string, cwd?: string): string {
    if (path.startsWith('/')) return path
    return `${cwd ?? this.root}/${path}`
  }
}

function statFor(overrides: Partial<HostFileStat>): HostFileStat {
  return {
    isFile: false,
    isDirectory: false,
    isSymbolicLink: false,
    mode: 0o644,
    size: 0,
    mtime: new Date(0),
    ...overrides,
  }
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
