import * as fs from 'tinyjs:fs'
import type { HostDirent, HostFileStat, HostFileSystem } from '@demicodes/shell'
import { dirnamePath, errnoError, errorCode, isAbsolutePath, isFileNotFoundError, normalizePath } from '@demicodes/utils'

const COPY_CHUNK = 1024 * 1024
const S_IFMT = 0o170000
const S_IFCHR = 0o020000
const S_IFIFO = 0o010000

/**
 * The `HostFileSystem` facet over `tinyjs:fs`. The primitives take absolute
 * paths, so every call resolves against the given or default cwd first;
 * `rm -r`, `cp` and a cross-device `mv` are composed here.
 */
export function createRunnerFileSystem(defaultCwd: string): HostFileSystem {
  const resolve = (path: string, cwd?: string): string =>
    isAbsolutePath(path) ? normalizePath(path) : normalizePath(`${cwd ?? defaultCwd}/${path}`)

  return {
    readFile: (path, options) => fs.readFile(resolve(path, options?.cwd)),
    writeFile: async (path, data, options) => {
      const target = resolve(path, options?.cwd)
      if (options?.createParents) await fs.mkdir(dirnamePath(target), { recursive: true })
      await fs.writeFile(target, data)
    },
    appendFile: async (path, data, options) => {
      const target = resolve(path, options?.cwd)
      if (options?.createParents) await fs.mkdir(dirnamePath(target), { recursive: true })
      await fs.writeFile(target, data, { append: true })
    },
    exists: async (path, options) => {
      try {
        await fs.lstat(resolve(path, options?.cwd))
        return true
      } catch (error) {
        if (isFileNotFoundError(error)) return false
        throw error
      }
    },
    stat: async (path, options) => toHostFileStat(await fs.stat(resolve(path, options?.cwd))),
    lstat: async (path, options) => toHostFileStat(await fs.lstat(resolve(path, options?.cwd))),
    readdir: (async (path: string, options?: { cwd?: string; withFileTypes?: boolean }) => {
      const entries = await fs.readdir(resolve(path, options?.cwd))
      return options?.withFileTypes ? entries.map(toHostDirent) : entries.map((entry) => entry.name)
    }) as HostFileSystem['readdir'],
    mkdir: (path, options) => fs.mkdir(resolve(path, options?.cwd), { recursive: options?.recursive === true }),
    rm: async (path, options) => {
      const target = resolve(path, options?.cwd)
      let stat: fs.Stat
      try {
        stat = await fs.lstat(target)
      } catch (error) {
        if (options?.force && isFileNotFoundError(error)) return
        throw error
      }
      if (stat.kind !== 'dir') return fs.unlink(target)
      if (!options?.recursive) throw errnoError('EISDIR', `EISDIR: is a directory, rm '${target}'`, { syscall: 'rm', path: target })
      await removeTree(target)
    },
    cp: async (path, destination, options) => {
      const source = resolve(path, options?.cwd)
      const target = resolve(destination, options?.cwd)
      const stat = await fs.lstat(source)
      if (stat.kind === 'dir' && !options?.recursive) {
        throw errnoError('EISDIR', `EISDIR: is a directory, cp '${source}'`, { syscall: 'cp', path: source })
      }
      await copyEntry(source, target, stat)
    },
    mv: async (path, destination, options) => {
      const source = resolve(path, options?.cwd)
      const target = resolve(destination, options?.cwd)
      try {
        await fs.rename(source, target)
      } catch (error) {
        if (errorCode(error) !== 'EXDEV') throw error
        await copyEntry(source, target, await fs.lstat(source))
        await removeEntry(source)
      }
    },
    chmod: (path, mode, options) => fs.chmod(resolve(path, options?.cwd), mode),
    symlink: (target, path, options) => fs.symlink(target, resolve(path, options?.cwd)),
    link: (existingPath, path, options) => fs.link(resolve(existingPath, options?.cwd), resolve(path, options?.cwd)),
    readlink: (path, options) => fs.readlink(resolve(path, options?.cwd)),
    realpath: (path, options) => fs.realpath(resolve(path, options?.cwd)),
    utimes: (path, atime, mtime, options) => fs.utimes(resolve(path, options?.cwd), atime.getTime(), mtime.getTime()),
  }
}

async function removeTree(dir: string): Promise<void> {
  for (const entry of await fs.readdir(dir)) {
    const path = `${dir}/${entry.name}`
    if (entry.kind === 'dir') await removeTree(path)
    else await fs.unlink(path)
  }
  await fs.rmdir(dir)
}

async function removeEntry(path: string): Promise<void> {
  if ((await fs.lstat(path)).kind === 'dir') await removeTree(path)
  else await fs.unlink(path)
}

async function copyEntry(source: string, target: string, stat: fs.Stat): Promise<void> {
  switch (stat.kind) {
    case 'dir': {
      await fs.mkdir(target, { recursive: true, mode: stat.mode & 0o7777 })
      for (const entry of await fs.readdir(source)) {
        await copyEntry(`${source}/${entry.name}`, `${target}/${entry.name}`, await fs.lstat(`${source}/${entry.name}`))
      }
      return
    }
    case 'symlink':
      return fs.symlink(await fs.readlink(source), target)
    case 'file':
      return copyFile(source, target, stat.mode & 0o7777)
    default:
      throw errnoError('EINVAL', `EINVAL: cannot copy a special file, cp '${source}'`, { syscall: 'cp', path: source })
  }
}

/** Streams a file through fixed-size reads: no whole-file buffer in JS. */
async function copyFile(source: string, target: string, mode: number): Promise<void> {
  const from = await fs.open(source, 'r')
  try {
    const to = await fs.open(target, 'w', mode)
    try {
      for (;;) {
        const chunk = await fs.read(from, COPY_CHUNK)
        if (chunk === null) return
        await fs.write(to, chunk)
      }
    } finally {
      fs.close(to)
    }
  } finally {
    fs.close(from)
  }
}

function toHostFileStat(stat: fs.Stat): HostFileStat {
  return {
    isFile: stat.kind === 'file',
    isDirectory: stat.kind === 'dir',
    isSymbolicLink: stat.kind === 'symlink',
    mode: stat.mode,
    size: stat.size,
    mtime: new Date(stat.mtimeMs),
    uid: stat.uid,
    gid: stat.gid,
    ino: stat.ino,
    dev: stat.dev,
    nlink: stat.nlink,
    isCharacterDevice: (stat.mode & S_IFMT) === S_IFCHR,
    isFIFO: (stat.mode & S_IFMT) === S_IFIFO,
  }
}

function toHostDirent(entry: { name: string; kind: fs.EntryKind }): HostDirent {
  return {
    name: entry.name,
    isFile: entry.kind === 'file',
    isDirectory: entry.kind === 'dir',
    isSymbolicLink: entry.kind === 'symlink',
  }
}
