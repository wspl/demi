import type {
  Host,
  HostFileSystem,
  HostIdentity,
  HostProcess,
  HostStore,
} from '@demicodes/shell'
import { createLogicalHostCwd } from '@demicodes/shell'
import { dirnamePath, errnoError, isAbsolutePath, normalizePath } from '@demicodes/utils'

/**
 * The zero-setup execution target: a `Host` whose filesystem is a
 * per-conversation virtual namespace over a pluggable backend and whose
 */

/** Per-file write cap. */
export const VIRTUAL_MAX_FILE_BYTES = 16 * 1024 * 1024
/** Per-conversation namespace cap. */
export const VIRTUAL_MAX_TOTAL_BYTES = 256 * 1024 * 1024

/**
 * A filesystem over the virtual namespace: every path is virtual-absolute and
 * already normalized (no `cwd` options arrive). Implementations translate to
 * their storage (a scoped local directory, object-store keys, …) and fail the
 * operations their storage cannot express.
 */
export interface VirtualFsBackend extends HostFileSystem {
  /** Total bytes of the files in the namespace — the quota's measure, counted however the storage counts. */
  usage(): Promise<number>
}

export interface VirtualHostOptions {
  backend: VirtualFsBackend
  /** Backend-composed store; conversation state does not live in the virtual fs. */
  store: HostStore
  /** Virtual working directory (default `/workspace`). */
  defaultCwd?: string
  /** Directories `ensureLayout` creates besides the working directory (the namespace). */
  directories?: readonly string[]
  identity?: HostIdentity
  /** Injectable for tests; product code keeps the hardcoded caps. */
  quota?: { maxFileBytes?: number; maxTotalBytes?: number }
}

export class VirtualHost implements Host {
  readonly defaultCwd: string
  readonly directories: readonly string[]
  readonly identity: HostIdentity
  readonly store: HostStore
  readonly fs: HostFileSystem
  readonly process: HostProcess

  private readonly backend: VirtualFsBackend
  private readonly maxFileBytes: number
  private readonly maxTotalBytes: number

  constructor(options: VirtualHostOptions) {
    this.backend = options.backend
    this.store = options.store
    this.defaultCwd = normalizePath(options.defaultCwd ?? '/workspace')
    this.directories = (options.directories ?? []).map((dir) => normalizePath(dir))
    this.identity = options.identity ?? { uid: 1000, gid: 1000, hostname: 'virtual', homeDir: this.defaultCwd }
    this.maxFileBytes = options.quota?.maxFileBytes ?? VIRTUAL_MAX_FILE_BYTES
    this.maxTotalBytes = options.quota?.maxTotalBytes ?? VIRTUAL_MAX_TOTAL_BYTES
    this.fs = this.createFs()
    this.process = {
      openCwd: async (path) => createLogicalHostCwd(this.resolve(path)),
    }
  }

  /** Creates the namespace's directory layout (the working dir and the declared directories). */
  async ensureLayout(): Promise<void> {
    await this.backend.mkdir(this.defaultCwd, { recursive: true })
    for (const dir of this.directories) await this.backend.mkdir(dir, { recursive: true })
  }

  /** Total bytes of the namespace's files, as the backend counts them. */
  usage(): Promise<number> {
    return this.backend.usage()
  }

  /**
   * Resolves a caller path + optional cwd into a normalized virtual-absolute
   * path. `..` above the root clamps to `/` — chroot semantics; nothing can
   * name a path outside the namespace.
   */
  private resolve(path: string, cwd?: string): string {
    if (isAbsolutePath(path)) return normalizePath(path)
    const base = cwd !== undefined ? normalizePath(cwd) : this.defaultCwd
    return normalizePath(`${base}/${path}`)
  }

  private async enforceQuota(path: string, incomingBytes: number, append: boolean): Promise<void> {
    let existingSize = 0
    try {
      existingSize = (await this.backend.stat(path)).size
    } catch {
      // New file.
    }
    const fileSize = append ? existingSize + incomingBytes : incomingBytes
    if (fileSize > this.maxFileBytes) {
      throw errnoError('EFBIG', `${path}: file exceeds the virtual workspace per-file limit (${this.maxFileBytes} bytes)`)
    }
    const total = await this.usage()
    const growth = append ? incomingBytes : Math.max(0, incomingBytes - existingSize)
    if (total + growth > this.maxTotalBytes) {
      throw errnoError('EDQUOT', `${path}: virtual workspace is over its total size limit (${this.maxTotalBytes} bytes)`)
    }
  }

  private createFs(): HostFileSystem {
    const backend = this.backend
    const resolve = (path: string, options?: { cwd?: string }) => this.resolve(path, options?.cwd)
    return {
      readFile: (path, options) => backend.readFile(resolve(path, options)),
      writeFile: async (path, data, options) => {
        const target = resolve(path, options)
        await this.enforceQuota(target, data.byteLength, false)
        await backend.writeFile(target, data, options?.createParents ? { createParents: true } : undefined)
      },
      appendFile: async (path, data, options) => {
        const target = resolve(path, options)
        await this.enforceQuota(target, data.byteLength, true)
        await backend.appendFile(target, data, options?.createParents ? { createParents: true } : undefined)
      },
      exists: (path, options) => backend.exists(resolve(path, options)),
      stat: (path, options) => backend.stat(resolve(path, options)),
      lstat: (path, options) => backend.lstat(resolve(path, options)),
      readdir: ((path: string, options?: { cwd?: string; withFileTypes?: boolean }) =>
        options?.withFileTypes
          ? backend.readdir(resolve(path, options), { withFileTypes: true })
          : backend.readdir(resolve(path, options))) as HostFileSystem['readdir'],
      mkdir: (path, options) =>
        backend.mkdir(resolve(path, options), options?.recursive ? { recursive: true } : undefined),
      rm: (path, options) =>
        backend.rm(resolve(path, options), {
          ...(options?.recursive ? { recursive: true } : {}),
          ...(options?.force ? { force: true } : {}),
        }),
      cp: (path, destination, options) =>
        backend.cp(resolve(path, options), resolve(destination, options), options?.recursive ? { recursive: true } : undefined),
      mv: (path, destination, options) => backend.mv(resolve(path, options), resolve(destination, options)),
      chmod: (path, mode, options) => backend.chmod(resolve(path, options), mode),
      symlink: async (target, path, options) => {
        const linkPath = resolve(path, options)
        if (!symlinkTargetStaysInside(linkPath, target)) {
          throw errnoError('EPERM', `${path}: symlink target escapes the virtual workspace`)
        }
        await backend.symlink(target, linkPath)
      },
      link: (existingPath, path, options) => backend.link(resolve(existingPath, options), resolve(path, options)),
      readlink: (path, options) => backend.readlink(resolve(path, options)),
      realpath: (path, options) => backend.realpath(resolve(path, options)),
      utimes: (path, atime, mtime, options) => backend.utimes(resolve(path, options), atime, mtime),
    }
  }
}

/**
 * True when a symlink target cannot walk above the namespace root: absolute
 * targets are virtual-absolute by definition; relative targets must not pop
 * past `/` at any point during resolution (clamping would silently change
 * meaning, and a scoped real-directory backend would materialize an escape).
 */
function symlinkTargetStaysInside(linkPath: string, target: string): boolean {
  if (isAbsolutePath(target)) return true
  const segments = dirnamePath(linkPath).split('/').filter(Boolean)
  for (const segment of target.replace(/\\/g, '/').split('/')) {
    if (segment === '' || segment === '.') continue
    if (segment === '..') {
      if (segments.length === 0) return false
      segments.pop()
      continue
    }
    segments.push(segment)
  }
  return true
}

