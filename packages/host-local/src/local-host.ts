import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { homedir, hostname, userInfo } from 'node:os'
import { dirname, isAbsolute, join, resolve } from 'node:path'
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
import type { Readable } from 'node:stream'
import { isFileNotFoundError } from '@demicodes/utils'
import type {
  Host,
  HostCwd,
  HostDirent,
  HostFileStat,
  HostFileSystem,
  HostIdentity,
  HostProcess,
  HostProcessOutputChunk,
  HostSpawnExit,
  HostSpawnHandle,
  HostSpawnParams,
  HostStore,
  SpawnErrorKind,
} from '@demicodes/shell'
import { LocalHostStore } from './local-store'
import { LocalHostCwd } from './local-cwd'

export interface LocalHostOptions {
  storeRoot?: string
  /** Where command artifacts land as plain files; defaults next to the store. */
  commandArtifactsDir?: string
  /** Bring-your-own persistence, e.g. to wrap or gate writes; defaults to a LocalHostStore rooted at storeRoot. */
  store?: HostStore
}

export class LocalHost implements Host {
  readonly defaultCwd: string
  readonly commandArtifactsDir: string
  readonly fs: HostFileSystem
  readonly process: HostProcess
  readonly store: HostStore
  readonly identity: HostIdentity

  constructor(defaultCwd: string, options: LocalHostOptions = {}) {
    this.defaultCwd = resolve(defaultCwd)
    const storeRoot = options.storeRoot ?? defaultStoreRoot(this.defaultCwd)
    this.commandArtifactsDir = resolve(options.commandArtifactsDir ?? join(storeRoot, 'command-artifacts'))
    this.fs = new LocalHostFileSystem(this.defaultCwd)
    this.process = new LocalHostProcess(this.defaultCwd)
    this.store = options.store ?? new LocalHostStore(storeRoot)
    const info = userInfo()
    this.identity = { uid: info.uid, gid: info.gid, hostname: hostname() }
  }
}

class LocalHostProcess implements HostProcess {
  constructor(private readonly defaultCwd: string) {}

  async openCwd(path: string): Promise<HostCwd> {
    return LocalHostCwd.open(path)
  }

  async spawn(params: HostSpawnParams): Promise<HostSpawnHandle> {
    const cwd = params.cwd ?? this.defaultCwd
    const child = spawn(params.command, params.args ?? [], {
      cwd,
      ...(params.env ? { env: definedEnv(params.env) } : {}),
      detached: params.killProcessGroup === true,
      stdio: ['pipe', 'pipe', 'pipe'],
    })

    let settled = false
    const waitPromise = new Promise<HostSpawnExit>((resolveWait) => {
      child.once('error', (error) => {
        if (settled) return
        settled = true
        void classifySpawnFailure(error, cwd).then((kind) => {
          resolveWait({
            exitCode: null,
            signal: error.message,
            spawnError: { kind },
          })
        })
      })
      child.once('close', (exitCode, signal) => {
        setImmediate(() => {
          if (settled) return
          settled = true
          resolveWait({ exitCode, signal: signal ?? undefined })
        })
      })
    })

    return {
      stdout: streamBytes(child.stdout),
      stderr: streamBytes(child.stderr),
      output: streamMergedOutput(child.stdout, child.stderr),
      writeStdin: async (data) => {
        if (!child.stdin || child.stdin.destroyed) return
        await new Promise<void>((resolve, reject) => {
          child.stdin.write(data, (error) => {
            if (error) reject(error)
            else resolve()
          })
        })
      },
      closeStdin: async () => {
        if (!child.stdin || child.stdin.destroyed) return
        child.stdin.end()
      },
      kill: async (signal = 'SIGTERM') => {
        if (!child.pid) return
        if (params.killProcessGroup === true) {
          try {
            process.kill(-child.pid, signal as NodeJS.Signals)
            return
          } catch {
            // Fall through to the direct child when process-group signaling is unavailable.
          }
        }
        if (!child.killed) child.kill(signal as NodeJS.Signals)
      },
      wait: () => waitPromise,
    }
  }
}

async function* streamMergedOutput(
  stdout: Readable | null,
  stderr: Readable | null,
): AsyncIterable<HostProcessOutputChunk> {
  const queue: Array<HostProcessOutputChunk | { done: true }> = []
  let wake: (() => void) | null = null
  let open = 0

  const push = (item: HostProcessOutputChunk | { done: true }) => {
    queue.push(item)
    wake?.()
    wake = null
  }
  const attach = (stream: Readable | null, name: 'stdout' | 'stderr') => {
    if (!stream) return []
    open += 1
    let ended = false
    const onData = (chunk: Buffer) => push({ stream: name, chunk })
    const onEnd = () => {
      if (ended) return
      ended = true
      open -= 1
      if (open === 0) push({ done: true })
    }
    stream.on('data', onData)
    stream.once('end', onEnd)
    stream.once('close', onEnd)
    return [
      () => stream.off('data', onData),
      () => stream.off('end', onEnd),
      () => stream.off('close', onEnd),
    ]
  }

  const cleanup = [
    ...attach(stdout, 'stdout'),
    ...attach(stderr, 'stderr'),
  ]
  if (open === 0) push({ done: true })

  try {
    while (true) {
      if (queue.length === 0) {
        await new Promise<void>((resolve) => {
          wake = resolve
        })
      }
      const item = queue.shift()
      if (!item) continue
      if ('done' in item) break
      yield item
    }
  } finally {
    for (const remove of cleanup) remove()
  }
}

class LocalHostFileSystem implements HostFileSystem {
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

// The store persists conversations and shell artifacts, so it lives in the
// platform data directory, not tmpdir (which evaporates on reboot/cleanup).
function defaultStoreRoot(defaultCwd: string): string {
  const key = createHash('sha256').update(defaultCwd).digest('hex').slice(0, 16)
  return join(dataHome(), 'demi', 'host-local', key)
}

function dataHome(): string {
  const xdg = process.env.XDG_DATA_HOME
  if (xdg && xdg.trim()) return xdg
  if (process.platform === 'win32') {
    const localAppData = process.env.LOCALAPPDATA
    if (localAppData && localAppData.trim()) return localAppData
    return join(homedir(), 'AppData', 'Local')
  }
  return join(homedir(), '.local', 'share')
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

function definedEnv(env: Record<string, string | undefined>): Record<string, string> {
  const defined: Record<string, string> = {}
  for (const [key, value] of Object.entries(env)) {
    if (value !== undefined) defined[key] = value
  }
  return defined
}

async function classifySpawnFailure(error: Error, cwd: string): Promise<SpawnErrorKind> {
  try {
    const cwdStat = await stat(cwd)
    if (!cwdStat.isDirectory()) return 'cwd_unusable'
  } catch {
    return 'cwd_unusable'
  }
  const code = 'code' in error ? String((error as { code: unknown }).code) : ''
  if (code === 'ENOENT') return 'executable_not_found'
  if (code === 'EACCES' || code === 'EPERM') return 'permission_denied'
  if (code === 'EISDIR') return 'is_directory'
  return 'other'
}

function toHostDirent(value: Dirent): HostDirent {
  return {
    name: value.name,
    isFile: value.isFile(),
    isDirectory: value.isDirectory(),
    isSymbolicLink: value.isSymbolicLink(),
  }
}

async function* streamBytes(stream: Readable | null): AsyncIterable<Uint8Array> {
  if (!stream) return
  for await (const chunk of stream) {
    if (chunk instanceof Uint8Array) {
      yield chunk
    } else {
      yield Buffer.from(String(chunk))
    }
  }
}
