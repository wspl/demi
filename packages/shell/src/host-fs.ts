import { decodeUtf8, encodeUtf8, isAbsolutePath, normalizePath } from '@demi/utils'
import type {
  BufferEncoding,
  CpOptions,
  DirentEntry,
  FileContent,
  FsStat,
  IFileSystem,
  MkdirOptions,
  ReadFileOptions,
  RmOptions,
  WriteFileOptions,
} from 'just-bash/fs/interface'
import type { Host, HostDirent, HostFileStat } from './host'

export type VirtualFileSystemNode =
  | { kind: 'file'; content: Uint8Array; mode?: number; mtime?: Date }
  | { kind: 'directory'; entries: DirentEntry[]; mode?: number; mtime?: Date }

export interface VirtualFileSystemProvider {
  lookup(path: string): Promise<VirtualFileSystemNode | null> | VirtualFileSystemNode | null
}

export class HostBackedFileSystem implements IFileSystem {
  constructor(
    private readonly host: Host,
    private readonly virtualProvider?: VirtualFileSystemProvider,
  ) {}

  async readFile(path: string, options?: ReadFileOptions | BufferEncoding): Promise<string> {
    return decodeBytes(await this.readFileBuffer(path), encodingFrom(options))
  }

  async readFileBuffer(path: string): Promise<Uint8Array> {
    const virtual = await this.lookupVirtual(path)
    if (virtual) {
      if (virtual.kind !== 'file') throw new Error(`EISDIR: illegal operation on a directory, read '${path}'`)
      return virtual.content
    }
    return this.host.fs.readFile(path, { cwd: this.host.defaultCwd })
  }

  async writeFile(path: string, content: FileContent, options?: WriteFileOptions | BufferEncoding): Promise<void> {
    this.assertWritablePath(path)
    await this.host.fs.writeFile(path, encodeContent(content, encodingFrom(options)), { cwd: this.host.defaultCwd })
  }

  async appendFile(path: string, content: FileContent, options?: WriteFileOptions | BufferEncoding): Promise<void> {
    this.assertWritablePath(path)
    await this.host.fs.appendFile(path, encodeContent(content, encodingFrom(options)), { cwd: this.host.defaultCwd })
  }

  async exists(path: string): Promise<boolean> {
    if (isVirtualPath(path)) return (await this.lookupVirtual(path)) !== null
    return this.host.fs.exists(path, { cwd: this.host.defaultCwd })
  }

  async stat(path: string): Promise<FsStat> {
    const virtual = await this.lookupVirtual(path)
    if (virtual) return virtualStat(virtual)
    return toFsStat(await this.host.fs.stat(path, { cwd: this.host.defaultCwd }))
  }

  async lstat(path: string): Promise<FsStat> {
    const virtual = await this.lookupVirtual(path)
    if (virtual) return virtualStat(virtual)
    return toFsStat(await this.host.fs.lstat(path, { cwd: this.host.defaultCwd }))
  }

  async readdir(path: string): Promise<string[]> {
    const virtual = await this.lookupVirtual(path)
    if (virtual) {
      if (virtual.kind !== 'directory') throw new Error(`ENOTDIR: not a directory, scandir '${path}'`)
      return virtual.entries.map((entry) => entry.name)
    }
    return this.host.fs.readdir(path, { cwd: this.host.defaultCwd })
  }

  async readdirWithFileTypes(path: string): Promise<DirentEntry[]> {
    const virtual = await this.lookupVirtual(path)
    if (virtual) {
      if (virtual.kind !== 'directory') throw new Error(`ENOTDIR: not a directory, scandir '${path}'`)
      return virtual.entries
    }
    const entries = await this.host.fs.readdir(path, { cwd: this.host.defaultCwd, withFileTypes: true })
    return entries.map(toDirentEntry)
  }

  resolvePath(base: string, path: string): string {
    if (isAbsolutePath(path)) return normalizePath(path)
    return normalizePath(`${base.replace(/[\\/]+$/, '')}/${path}`)
  }

  getAllPaths(): string[] {
    return []
  }

  async mkdir(path: string, options?: MkdirOptions): Promise<void> {
    this.assertWritablePath(path)
    await this.host.fs.mkdir(path, { cwd: this.host.defaultCwd, recursive: options?.recursive })
  }

  async rm(path: string, options?: RmOptions): Promise<void> {
    this.assertWritablePath(path)
    await this.host.fs.rm(path, { cwd: this.host.defaultCwd, recursive: options?.recursive, force: options?.force })
  }

  async cp(src: string, dest: string, options?: CpOptions): Promise<void> {
    this.assertWritablePath(dest)
    if (isVirtualPath(src)) {
      const virtual = await this.lookupVirtual(src)
      if (!virtual) throw new Error(`ENOENT: no such file or directory, copyfile '${src}'`)
      if (virtual.kind !== 'file') throw new Error(`EISDIR: illegal operation on a directory, copyfile '${src}'`)
      await this.host.fs.writeFile(dest, virtual.content, { cwd: this.host.defaultCwd, createParents: options?.recursive })
      return
    }
    await this.host.fs.cp(src, dest, { cwd: this.host.defaultCwd, recursive: options?.recursive })
  }

  async mv(src: string, dest: string): Promise<void> {
    this.assertWritablePath(src)
    this.assertWritablePath(dest)
    await this.host.fs.mv(src, dest, { cwd: this.host.defaultCwd })
  }

  async chmod(path: string, mode: number): Promise<void> {
    this.assertWritablePath(path)
    await this.host.fs.chmod(path, mode, { cwd: this.host.defaultCwd })
  }

  async symlink(target: string, linkPath: string): Promise<void> {
    this.assertWritablePath(linkPath)
    await this.host.fs.symlink(target, linkPath, { cwd: this.host.defaultCwd })
  }

  async link(existingPath: string, newPath: string): Promise<void> {
    this.assertWritablePath(existingPath)
    this.assertWritablePath(newPath)
    await this.host.fs.link(existingPath, newPath, { cwd: this.host.defaultCwd })
  }

  async readlink(path: string): Promise<string> {
    if (isVirtualPath(path)) throw new Error(`EINVAL: invalid argument, readlink '${path}'`)
    return this.host.fs.readlink(path, { cwd: this.host.defaultCwd })
  }

  async realpath(path: string): Promise<string> {
    if (isVirtualPath(path)) {
      const virtual = await this.lookupVirtual(path)
      if (!virtual) throw new Error(`ENOENT: no such file or directory, realpath '${path}'`)
      return normalizePath(path)
    }
    return this.host.fs.realpath(path, { cwd: this.host.defaultCwd })
  }

  async utimes(path: string, atime: Date, mtime: Date): Promise<void> {
    this.assertWritablePath(path)
    await this.host.fs.utimes(path, atime, mtime, { cwd: this.host.defaultCwd })
  }

  private async lookupVirtual(path: string): Promise<VirtualFileSystemNode | null> {
    if (!this.virtualProvider || !isVirtualPath(path)) return null
    return (await this.virtualProvider.lookup(normalizePath(path))) ?? null
  }

  private assertWritablePath(path: string): void {
    if (isVirtualPath(path)) throw new Error(`EROFS: read-only virtual filesystem, '${path}'`)
  }
}

function encodingFrom(options?: ReadFileOptions | WriteFileOptions | BufferEncoding): BufferEncoding | undefined {
  if (!options) return undefined
  if (typeof options === 'string') return options
  return options.encoding ?? undefined
}

function encodeContent(content: FileContent, encoding?: BufferEncoding): Uint8Array {
  if (content instanceof Uint8Array) return content
  if (encoding === 'binary' || encoding === 'latin1') return latin1ToBytes(content)
  if (encoding === 'base64') return base64ToBytes(content)
  if (encoding === 'hex') return hexToBytes(content)
  return encodeUtf8(content)
}

function decodeBytes(bytes: Uint8Array, encoding?: BufferEncoding | null): string {
  if (encoding === 'binary' || encoding === 'latin1') return bytesToLatin1(bytes)
  if (encoding === 'base64') return bytesToBase64(bytes)
  if (encoding === 'hex') return [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('')
  return decodeUtf8(bytes)
}

function latin1ToBytes(content: string): Uint8Array {
  const bytes = new Uint8Array(content.length)
  for (let index = 0; index < content.length; index += 1) bytes[index] = content.charCodeAt(index) & 0xff
  return bytes
}

function bytesToLatin1(bytes: Uint8Array): string {
  let result = ''
  for (let index = 0; index < bytes.length; index += 8192) {
    result += String.fromCharCode(...bytes.subarray(index, index + 8192))
  }
  return result
}

function base64ToBytes(content: string): Uint8Array {
  return latin1ToBytes(atob(content))
}

function bytesToBase64(bytes: Uint8Array): string {
  return btoa(bytesToLatin1(bytes))
}

function hexToBytes(content: string): Uint8Array {
  const bytes = new Uint8Array(Math.floor(content.length / 2))
  for (let index = 0; index < bytes.length; index += 1) {
    bytes[index] = Number.parseInt(content.slice(index * 2, index * 2 + 2), 16)
  }
  return bytes
}

function toFsStat(value: HostFileStat): FsStat {
  return {
    isFile: value.isFile,
    isDirectory: value.isDirectory,
    isSymbolicLink: value.isSymbolicLink,
    mode: value.mode,
    size: value.size,
    mtime: value.mtime,
  }
}

function toDirentEntry(value: HostDirent): DirentEntry {
  return {
    name: value.name,
    isFile: value.isFile,
    isDirectory: value.isDirectory,
    isSymbolicLink: value.isSymbolicLink,
  }
}

function virtualStat(node: VirtualFileSystemNode): FsStat {
  const now = new Date(0)
  if (node.kind === 'directory') {
    return {
      isFile: false,
      isDirectory: true,
      isSymbolicLink: false,
      mode: node.mode ?? 0o555,
      size: 0,
      mtime: node.mtime ?? now,
    }
  }
  return {
    isFile: true,
    isDirectory: false,
    isSymbolicLink: false,
    mode: node.mode ?? 0o444,
    size: node.content.byteLength,
    mtime: node.mtime ?? now,
  }
}

function isVirtualPath(path: string): boolean {
  const normalized = normalizePath(path)
  return normalized === '/@' || normalized.startsWith('/@/')
}
