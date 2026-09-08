import type { HostDirent, HostFileStat, HostFileSystem } from '@demicodes/shell'
import { dirnamePath, errnoError, errorCode, isAbsolutePath, isFileNotFoundError, normalizePath } from '@demicodes/utils'

const COPY_CHUNK = 1024 * 1024
const S_IFMT = 0o170000
const S_IFCHR = 0o020000
const S_IFIFO = 0o010000

/**
 * The `HostFileSystem` facet over txiki.js. The primitives take absolute
 * paths, so every call resolves against the given or default cwd first;
 * `rm -r`, `cp` and a cross-device `mv` are composed here.
 */
export function createRunnerFileSystem(defaultCwd: string): HostFileSystem {
  const resolve = (path: string, cwd?: string): string =>
    isAbsolutePath(path) ? normalizePath(path) : normalizePath(`${cwd ?? defaultCwd}/${path}`)

  return {
    readFile: (path, options) => tjs.readFile(resolve(path, options?.cwd)),
    writeFile: async (path, data, options) => {
      const target = resolve(path, options?.cwd)
      if (options?.createParents) await tjs.makeDir(dirnamePath(target), { recursive: true })
      await tjs.writeFile(target, data)
    },
    appendFile: async (path, data, options) => {
      const target = resolve(path, options?.cwd)
      if (options?.createParents) await tjs.makeDir(dirnamePath(target), { recursive: true })
      const file = await tjs.open(target, 'a')
      try {
        await writeAll(file, data)
      } finally {
        await file.close()
      }
    },
    exists: async (path, options) => {
      try {
        await tjs.lstat(resolve(path, options?.cwd))
        return true
      } catch (error) {
        if (isFileNotFoundError(error)) return false
        throw error
      }
    },
    stat: async (path, options) => toHostFileStat(await tjs.stat(resolve(path, options?.cwd))),
    lstat: async (path, options) => toHostFileStat(await tjs.lstat(resolve(path, options?.cwd))),
    readdir: (async (path: string, options?: { cwd?: string; withFileTypes?: boolean }) => {
      const entries = await readDirectory(resolve(path, options?.cwd))
      return options?.withFileTypes ? entries.map(toHostDirent) : entries.map((entry) => entry.name)
    }) as HostFileSystem['readdir'],
    mkdir: (path, options) => tjs.makeDir(resolve(path, options?.cwd), { recursive: options?.recursive === true }),
    rm: async (path, options) => {
      const target = resolve(path, options?.cwd)
      let stat: tjs.StatResult
      try {
        stat = await tjs.lstat(target)
      } catch (error) {
        if (options?.force && isFileNotFoundError(error)) return
        throw error
      }
      if (!stat.isDirectory) return tjs.remove(target)
      if (!options?.recursive) throw errnoError('EISDIR', `EISDIR: is a directory, rm '${target}'`, { syscall: 'rm', path: target })
      await removeTree(target)
    },
    cp: async (path, destination, options) => {
      const source = resolve(path, options?.cwd)
      const target = resolve(destination, options?.cwd)
      const stat = await tjs.lstat(source)
      if (stat.isDirectory && !options?.recursive) {
        throw errnoError('EISDIR', `EISDIR: is a directory, cp '${source}'`, { syscall: 'cp', path: source })
      }
      await copyEntry(source, target, stat)
    },
    mv: async (path, destination, options) => {
      const source = resolve(path, options?.cwd)
      const target = resolve(destination, options?.cwd)
      try {
        await tjs.rename(source, target)
      } catch (error) {
        if (errorCode(error) !== 'EXDEV') throw error
        await copyEntry(source, target, await tjs.lstat(source))
        await removeEntry(source)
      }
    },
    chmod: (path, mode, options) => tjs.chmod(resolve(path, options?.cwd), mode),
    symlink: (target, path, options) => tjs.symlink(target, resolve(path, options?.cwd)),
    link: (existingPath, path, options) => tjs.link(resolve(existingPath, options?.cwd), resolve(path, options?.cwd)),
    readlink: (path, options) => tjs.readLink(resolve(path, options?.cwd)),
    realpath: (path, options) => tjs.realPath(resolve(path, options?.cwd)),
    utimes: (path, atime, mtime, options) => tjs.utime(resolve(path, options?.cwd), atime, mtime),
  }
}

async function removeTree(dir: string): Promise<void> {
  for (const entry of await readDirectory(dir)) {
    const path = `${dir}/${entry.name}`
    if (entry.isDirectory) await removeTree(path)
    else await tjs.remove(path)
  }
  await tjs.remove(dir)
}

async function removeEntry(path: string): Promise<void> {
  if ((await tjs.lstat(path)).isDirectory) await removeTree(path)
  else await tjs.remove(path)
}

async function copyEntry(source: string, target: string, stat: tjs.StatResult): Promise<void> {
  switch (true) {
    case stat.isDirectory: {
      await tjs.makeDir(target, { recursive: true, mode: stat.mode & 0o7777 })
      for (const entry of await readDirectory(source)) {
        const sourcePath = `${source}/${entry.name}`
        const targetPath = `${target}/${entry.name}`
        await copyEntry(sourcePath, targetPath, await tjs.lstat(sourcePath))
      }
      return
    }
    case stat.isSymbolicLink:
      return tjs.symlink(await tjs.readLink(source), target)
    case stat.isFile:
      return copyFile(source, target, stat.mode & 0o7777)
    default:
      throw errnoError('EINVAL', `EINVAL: cannot copy a special file, cp '${source}'`, { syscall: 'cp', path: source })
  }
}

/** Streams a file through fixed-size reads: no whole-file buffer in JS. */
async function copyFile(source: string, target: string, mode: number): Promise<void> {
  const from = await tjs.open(source, 'r')
  try {
    const to = await tjs.open(target, 'w', mode)
    try {
      for (;;) {
        const buffer = new Uint8Array(COPY_CHUNK)
        const size = await from.read(buffer)
        if (size === null) return
        await writeAll(to, buffer.subarray(0, size))
      }
    } finally {
      await to.close()
    }
  } finally {
    await from.close()
  }
}

function toHostFileStat(stat: tjs.StatResult): HostFileStat {
  return {
    isFile: stat.isFile,
    isDirectory: stat.isDirectory,
    isSymbolicLink: stat.isSymbolicLink,
    mode: stat.mode,
    size: stat.size,
    mtime: stat.mtim,
    uid: stat.uid,
    gid: stat.gid,
    ino: stat.ino,
    dev: stat.dev,
    nlink: stat.nlink,
    isCharacterDevice: (stat.mode & S_IFMT) === S_IFCHR,
    isFIFO: (stat.mode & S_IFMT) === S_IFIFO,
  }
}

function toHostDirent(entry: tjs.DirEnt): HostDirent {
  return {
    name: entry.name,
    isFile: entry.isFile,
    isDirectory: entry.isDirectory,
    isSymbolicLink: entry.isSymbolicLink,
  }
}

async function readDirectory(path: string): Promise<tjs.DirEnt[]> {
  const entries: tjs.DirEnt[] = []
  for await (const entry of await tjs.readDir(path)) entries.push(entry)
  return entries
}

/** File writes can be partial; finish each chunk before accepting another. */
export async function writeAll(file: tjs.FileHandle, data: Uint8Array): Promise<void> {
  const writer = file.writable.getWriter()
  try {
    await writer.write(data)
  } finally {
    writer.releaseLock()
  }
}
