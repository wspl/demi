// The Host filesystem contract over Node's `fs/promises`.
import {
  appendFile,
  chmod,
  cp,
  link,
  lstat,
  mkdir,
  readFile,
  readdir,
  readlink,
  realpath,
  rename,
  rm,
  stat,
  symlink,
  utimes,
  writeFile,
} from 'node:fs/promises'
import type { Dirent, Stats } from 'node:fs'
import { dirname, isAbsolute, resolve } from 'node:path'
import { isFileNotFoundError } from '@demicodes/utils'
import type { HostDirent, HostFileStat, HostFileSystem } from '../host'

/** Relative paths resolve against `defaultCwd` unless a call names its own `cwd`. */
export function nodeFileSystem(defaultCwd: string): HostFileSystem {
  return new NodeFileSystem(resolve(defaultCwd))
}

class NodeFileSystem implements HostFileSystem {
  constructor(private readonly defaultCwd: string) {}

  async readFile(path: string, options?: { cwd?: string }): Promise<Uint8Array> {
    return readFile(this.resolvePath(path, options?.cwd))
  }

  async writeFile(path: string, data: Uint8Array, options?: { cwd?: string; createParents?: boolean }): Promise<void> {
    const target = this.resolvePath(path, options?.cwd)
    if (options?.createParents) await mkdir(dirname(target), { recursive: true })
    await writeFile(target, data)
  }

  async appendFile(path: string, data: Uint8Array, options?: { cwd?: string; createParents?: boolean }): Promise<void> {
    const target = this.resolvePath(path, options?.cwd)
    if (options?.createParents) await mkdir(dirname(target), { recursive: true })
    await appendFile(target, data)
  }

  async exists(path: string, options?: { cwd?: string }): Promise<boolean> {
    try {
      await lstat(this.resolvePath(path, options?.cwd))
      return true
    } catch (error) {
      if (isFileNotFoundError(error)) return false
      throw error
    }
  }

  async stat(path: string, options?: { cwd?: string }): Promise<HostFileStat> {
    return toHostFileStat(await stat(this.resolvePath(path, options?.cwd)))
  }

  async lstat(path: string, options?: { cwd?: string }): Promise<HostFileStat> {
    return toHostFileStat(await lstat(this.resolvePath(path, options?.cwd)))
  }

  async readdir(path: string, options: { cwd?: string; withFileTypes: true }): Promise<HostDirent[]>
  async readdir(path: string, options?: { cwd?: string; withFileTypes?: false }): Promise<string[]>
  async readdir(path: string, options?: { cwd?: string; withFileTypes?: boolean }): Promise<string[] | HostDirent[]> {
    const target = this.resolvePath(path, options?.cwd)
    if (options?.withFileTypes) {
      return (await readdir(target, { withFileTypes: true })).map(toHostDirent)
    }
    return readdir(target)
  }

  async mkdir(path: string, options?: { cwd?: string; recursive?: boolean }): Promise<void> {
    // Node rejects `{ recursive: undefined }` (must be boolean or omitted).
    await mkdir(this.resolvePath(path, options?.cwd), { recursive: options?.recursive === true })
  }

  async rm(path: string, options?: { cwd?: string; recursive?: boolean; force?: boolean }): Promise<void> {
    await rm(this.resolvePath(path, options?.cwd), {
      recursive: options?.recursive === true,
      force: options?.force === true,
    })
  }

  async cp(path: string, destination: string, options?: { cwd?: string; recursive?: boolean }): Promise<void> {
    await cp(this.resolvePath(path, options?.cwd), this.resolvePath(destination, options?.cwd), {
      recursive: options?.recursive === true,
    })
  }

  async mv(path: string, destination: string, options?: { cwd?: string }): Promise<void> {
    await rename(this.resolvePath(path, options?.cwd), this.resolvePath(destination, options?.cwd))
  }

  async chmod(path: string, mode: number, options?: { cwd?: string }): Promise<void> {
    await chmod(this.resolvePath(path, options?.cwd), mode)
  }

  async symlink(target: string, path: string, options?: { cwd?: string }): Promise<void> {
    await symlink(target, this.resolvePath(path, options?.cwd))
  }

  async link(existingPath: string, path: string, options?: { cwd?: string }): Promise<void> {
    await link(this.resolvePath(existingPath, options?.cwd), this.resolvePath(path, options?.cwd))
  }

  async readlink(path: string, options?: { cwd?: string }): Promise<string> {
    return readlink(this.resolvePath(path, options?.cwd))
  }

  async realpath(path: string, options?: { cwd?: string }): Promise<string> {
    return realpath(this.resolvePath(path, options?.cwd))
  }

  async utimes(path: string, atime: Date, mtime: Date, options?: { cwd?: string }): Promise<void> {
    await utimes(this.resolvePath(path, options?.cwd), atime, mtime)
  }

  private resolvePath(path: string, cwd?: string): string {
    if (isAbsolute(path)) return resolve(path)
    return resolve(cwd ?? this.defaultCwd, path)
  }
}

function toHostFileStat(value: Stats): HostFileStat {
  return {
    isFile: value.isFile(),
    isDirectory: value.isDirectory(),
    isSymbolicLink: value.isSymbolicLink(),
    mode: value.mode,
    size: value.size,
    mtime: value.mtime,
    uid: value.uid,
    gid: value.gid,
    ino: value.ino,
    dev: value.dev,
    nlink: value.nlink,
    isCharacterDevice: value.isCharacterDevice(),
    isFIFO: value.isFIFO(),
  }
}

function toHostDirent(value: Dirent): HostDirent {
  return {
    name: value.name,
    isFile: value.isFile(),
    isDirectory: value.isDirectory(),
    isSymbolicLink: value.isSymbolicLink(),
  }
}
