export interface Host {
  defaultCwd: string
  /**
   * Directory where command artifacts (stdout.txt / stderr.txt / stdout.bin /
   * meta.json) are written as plain files, laid out as
   * `<dir>/<storageId>/<commandId>/`. Contract: the path is reachable through
   * `fs` AND visible to processes started via `process.spawn` — one shared
   * filesystem namespace, so any tool (portable or real) can read and search
   * artifacts with ordinary file operations.
   */
  commandArtifactsDir: string
  fs: HostFileSystem
  process: HostProcess
  store: HostStore
  identity: HostIdentity
}

export interface HostIdentity {
  uid: number
  gid: number
  hostname: string
}

export interface HostFileSystem {
  readFile(path: string, options?: { cwd?: string }): Promise<Uint8Array>
  writeFile(path: string, data: Uint8Array, options?: { cwd?: string; createParents?: boolean }): Promise<void>
  appendFile(path: string, data: Uint8Array, options?: { cwd?: string; createParents?: boolean }): Promise<void>
  exists(path: string, options?: { cwd?: string }): Promise<boolean>
  stat(path: string, options?: { cwd?: string }): Promise<HostFileStat>
  lstat(path: string, options?: { cwd?: string }): Promise<HostFileStat>
  readdir(path: string, options?: { cwd?: string; withFileTypes?: false }): Promise<string[]>
  readdir(path: string, options: { cwd?: string; withFileTypes: true }): Promise<HostDirent[]>
  mkdir(path: string, options?: { cwd?: string; recursive?: boolean }): Promise<void>
  rm(path: string, options?: { cwd?: string; recursive?: boolean; force?: boolean }): Promise<void>
  cp(path: string, destination: string, options?: { cwd?: string; recursive?: boolean }): Promise<void>
  mv(path: string, destination: string, options?: { cwd?: string }): Promise<void>
  chmod(path: string, mode: number, options?: { cwd?: string }): Promise<void>
  symlink(target: string, path: string, options?: { cwd?: string }): Promise<void>
  link(existingPath: string, path: string, options?: { cwd?: string }): Promise<void>
  readlink(path: string, options?: { cwd?: string }): Promise<string>
  realpath(path: string, options?: { cwd?: string }): Promise<string>
  utimes(path: string, atime: Date, mtime: Date, options?: { cwd?: string }): Promise<void>
}

export interface HostProcess {
  spawn(params: HostSpawnParams): Promise<HostSpawnHandle>
  openCwd(path: string): Promise<HostCwd>
}

export type SpawnErrorKind =
  | 'executable_not_found'
  | 'permission_denied'
  | 'cwd_unusable'
  | 'is_directory'
  | 'other'

export interface HostCwd {
  readonly path: string
  spawnPath(): string
  chdir(path: string): Promise<void>
  snapshot(): Promise<{ restore(): void }>
  close(): Promise<void>
}

/**
 * Keyed JSON state storage. Implementations must round-trip `Uint8Array` and
 * `bigint` values (e.g. via the portable JSON codec in `@demicodes/utils`),
 * since stored values such as agent session snapshots carry binary content.
 */
export interface HostStore {
  readJson<T>(key: string): Promise<T | null>
  writeJson<T>(key: string, value: T): Promise<void>
  delete(key: string): Promise<void>
  list(prefix: string): Promise<string[]>
}

export interface HostFileStat {
  isFile: boolean
  isDirectory: boolean
  isSymbolicLink: boolean
  mode: number
  size: number
  mtime: Date
  uid?: number
  gid?: number
  ino?: number
  dev?: number
  nlink?: number
  isCharacterDevice?: boolean
  isFIFO?: boolean
}

export interface HostDirent {
  name: string
  isFile: boolean
  isDirectory: boolean
  isSymbolicLink: boolean
}

export interface HostSpawnParams {
  command: string
  args?: string[]
  cwd?: string
  env?: Record<string, string | undefined>
  killProcessGroup?: boolean
}

export interface HostSpawnHandle {
  stdout: AsyncIterable<Uint8Array>
  stderr: AsyncIterable<Uint8Array>
  output?: AsyncIterable<HostProcessOutputChunk>
  writeStdin(data: Uint8Array): Promise<void>
  closeStdin(): Promise<void>
  kill(signal?: string): Promise<void>
  wait(): Promise<HostSpawnExit>
}

export interface HostProcessOutputChunk {
  stream: 'stdout' | 'stderr'
  chunk: Uint8Array
}

export interface HostSpawnError {
  kind: SpawnErrorKind
  /** Optional Host-specific guidance appended to the shell's error message
   *  (e.g. a virtual target explaining that real programs need a device). */
  detail?: string
}

export interface HostSpawnExit {
  exitCode: number | null
  signal?: string
  spawnError?: HostSpawnError
}

/** Path-string cwd for test doubles and Hosts that cannot hold a directory fd. */
export function createLogicalHostCwd(initialPath: string): HostCwd {
  let path = initialPath
  return {
    get path() {
      return path
    },
    spawnPath() {
      return path
    },
    async chdir(next: string) {
      if (next === '.') return
      path = resolveLogicalCwd(path, next)
    },
    async snapshot() {
      const saved = path
      return {
        restore() {
          path = saved
        },
      }
    },
    async close() {},
  }
}

function resolveLogicalCwd(base: string, next: string): string {
  if (next.startsWith('/')) return next
  const parts = base.split('/').filter(Boolean)
  for (const part of next.split('/')) {
    if (!part || part === '.') continue
    if (part === '..') parts.pop()
    else parts.push(part)
  }
  return `/${parts.join('/')}`
}

