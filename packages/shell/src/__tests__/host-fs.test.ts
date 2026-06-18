import { mkdir, mkdtemp, writeFile, readFile as fsReadFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
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
