import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { tmpdir } from 'node:os'
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
import type {
  Host,
  HostDirent,
  HostFileStat,
  HostFileSystem,
  HostProcess,
  HostProcessOutputChunk,
  HostSpawnHandle,
  HostSpawnParams,
  HostStore,
} from '@demi/shell'
import { LocalHostStore } from './local-store'

export interface LocalHostOptions {
  storeRoot?: string
}

export class LocalHost implements Host {
  readonly defaultCwd: string
  readonly fs: HostFileSystem
  readonly process: HostProcess
  readonly store: HostStore

  constructor(defaultCwd: string, options: LocalHostOptions = {}) {
    this.defaultCwd = resolve(defaultCwd)
    this.fs = new LocalHostFileSystem(this.defaultCwd)
    this.process = new LocalHostProcess(this.defaultCwd)
    this.store = new LocalHostStore(options.storeRoot ?? defaultStoreRoot(this.defaultCwd))
  }
}

class LocalHostProcess implements HostProcess {
  constructor(private readonly defaultCwd: string) {}

  async spawn(params: HostSpawnParams): Promise<HostSpawnHandle> {
    const child = spawn(params.command, params.args ?? [], {
      cwd: params.cwd ?? this.defaultCwd,
      env: { ...process.env, ...params.env },
      detached: params.killProcessGroup === true,
      stdio: ['pipe', 'pipe', 'pipe'],
    })

    let settled = false
    const waitPromise = new Promise<{ exitCode: number | null; signal?: string }>((resolve) => {
      child.once('error', (error) => {
        if (settled) return
        settled = true
        resolve({ exitCode: null, signal: error.message })
      })
      child.once('close', (exitCode, signal) => {
        if (settled) return
        settled = true
        resolve({ exitCode, signal: signal ?? undefined })
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
      if (isNotFound(error)) return false
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
    await mkdir(this.resolvePath(path, options?.cwd), { recursive: options?.recursive })
  }

  async rm(path: string, options?: { cwd?: string; recursive?: boolean; force?: boolean }): Promise<void> {
    await rm(this.resolvePath(path, options?.cwd), { recursive: options?.recursive, force: options?.force })
  }

  async cp(path: string, destination: string, options?: { cwd?: string; recursive?: boolean }): Promise<void> {
    await cp(this.resolvePath(path, options?.cwd), this.resolvePath(destination, options?.cwd), { recursive: options?.recursive })
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

function defaultStoreRoot(defaultCwd: string): string {
  const key = createHash('sha256').update(defaultCwd).digest('hex').slice(0, 16)
  return join(tmpdir(), 'demi-host-local-store', key)
}

function toHostFileStat(value: Stats): HostFileStat {
  return {
    isFile: value.isFile(),
    isDirectory: value.isDirectory(),
    isSymbolicLink: value.isSymbolicLink(),
    mode: value.mode,
    size: value.size,
    mtime: value.mtime,
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

function isNotFound(error: unknown): boolean {
  return typeof error === 'object' && error !== null && 'code' in error && (error as { code?: unknown }).code === 'ENOENT'
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
