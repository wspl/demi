import { isAbsolutePath, normalizePath } from '@demicodes/utils'

export interface Host {
  defaultCwd: string
  fs: HostFileSystem
  process: HostProcess
  store: HostStore
  identity: HostIdentity
}

export interface HostIdentity {
  uid: number
  gid: number
  hostname: string
  /**
   * The home directory of the user the Host runs as: where session-bound and
   * granted hosts start a shell.
   */
  homeDir: string
}

export interface HostFileSystem {
  readFile(path: string, options?: { cwd?: string }): Promise<Uint8Array>
  /**
   * The file's bytes as they are read: `length` bytes from `offset`, or to
   * the end when `length` is absent. Resolves once the file is open, so a
   * missing or unreadable file rejects here, before any byte. Ending the
   * iteration early, or aborting `signal`, stops the read.
   */
  readStream(
    path: string,
    options?: {
      cwd?: string;
      offset?: number;
      length?: number;
      signal?: AbortSignal
    }
  ): Promise<AsyncIterable<Uint8Array>>
  /**
   * Replaces the file whole with `data`: its bytes at once, or a stream of
   * them written as they come, the next taken once the last is written. A
   * failed write, a stream that fails, or aborting `signal` leaves the file
   * as it was; the stream's failure, or the abort's reason, is the one
   * thrown.
   */
  writeFile(
    path: string,
    data: Uint8Array | AsyncIterable<Uint8Array>,
    options?: {
      cwd?: string;
      createParents?: boolean;
      signal?: AbortSignal
    }
  ): Promise<void>
  exists(path: string, options?: { cwd?: string }): Promise<boolean>
  stat(path: string, options?: { cwd?: string }): Promise<HostFileStat>
  lstat(path: string, options?: { cwd?: string }): Promise<HostFileStat>
  readdir(
    path: string,
    options?: {
      cwd?: string;
      withFileTypes?: false
    }
  ): Promise<string[]>
  readdir(
    path: string,
    options: {
      cwd?: string;
      withFileTypes: true
    }
  ): Promise<HostDirent[]>
  mkdir(
    path: string,
    options?: {
      cwd?: string;
      recursive?: boolean
    }
  ): Promise<void>
  rm(
    path: string,
    options?: {
      cwd?: string;
      recursive?: boolean;
      force?: boolean
    }
  ): Promise<void>
  cp(
    path: string,
    destination: string,
    options?: {
      cwd?: string;
      recursive?: boolean
    }
  ): Promise<void>
  mv(
    path: string,
    destination: string,
    options?: { cwd?: string }
  ): Promise<void>
  chmod(path: string, mode: number, options?: { cwd?: string }): Promise<void>
  symlink(
    target: string,
    path: string,
    options?: { cwd?: string }
  ): Promise<void>
  link(
    existingPath: string,
    path: string,
    options?: { cwd?: string }
  ): Promise<void>
  readlink(path: string, options?: { cwd?: string }): Promise<string>
  realpath(path: string, options?: { cwd?: string }): Promise<string>
  utimes(
    path: string,
    atime: Date,
    mtime: Date,
    options?: { cwd?: string }
  ): Promise<void>
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
  /**
   * The stored document, or `null` when the key holds nothing. Stored JSON is
   * data from outside this process: validate it with a schema, the reader's
   * own, before using it.
   */
  readJson(key: string): Promise<unknown>
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
  /** Extend the target process environment; undefined entries remove inherited names. */
  inheritEnv?: boolean
  killProcessGroup?: boolean
  /**
   * The process is kept between pieces of work and is not itself work: a Host
   * that counts activity leaves it out, and may stop with it running.
   */
  retained?: boolean
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

/**
 * Path-string cwd for test doubles and Hosts that cannot hold a directory fd.
 */
export function createLogicalHostCwd(
  initialPath: string,
  validate?: (path: string) => Promise<void>
): HostCwd {
  let path = initialPath
  return {
    get path() {
      return path
    },
    spawnPath() {
      return path
    },
    async chdir(next: string) {
      if (next === '.')
        return
      const target = normalizePath(isAbsolutePath(next) ? next : `${path}/${next}`)
      await validate?.(target)
      path = target
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
