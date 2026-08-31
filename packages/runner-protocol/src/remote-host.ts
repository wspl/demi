import type {
  Host,
  HostCwd,
  HostFileStat,
  HostFileSystem,
  HostIdentity,
  HostProcess,
  HostProcessOutputChunk,
  HostSpawnExit,
  HostSpawnHandle,
  HostSpawnParams,
  HostStore,
} from '@demicodes/shell'
import { createLogicalHostCwd } from '@demicodes/shell'
import { createId, deferred, type Deferred } from '@demicodes/utils'
import type { BackendToRunnerMessage, HostFsOp, RunnerToBackendMessage, WireCallError } from './messages'

export interface RemoteHostOptions {
  defaultCwd: string
  /** Artifact directory on the execution target (part of the remote fs namespace). */
  commandArtifactsDir: string
  /** From the runner `hello` — read synchronously at shell creation. */
  identity: HostIdentity
  /** Backend-composed store; conversation state never crosses the runner protocol. */
  store: HostStore
}

/**
 * The backend-side `Host` over a connected runner: every `fs` method is one
 * `fs_call` round trip, `process.spawn` streams over `spawn_*` messages, and
 * `openCwd` uses the contract's own logical path fallback (directory fds
 * cannot cross the wire).
 *
 * The object is stable across reconnects — `AgentHarness.host` must return
 * the same Host for the same execution target, so per-Host shell state
 * survives a runner going offline. `attach` binds the current connection;
 * `detach` fails everything in flight (surfacing as ordinary tool errors)
 * and leaves the Host offline until the next `attach`.
 */
export class RemoteHost implements Host {
  readonly defaultCwd: string
  readonly commandArtifactsDir: string
  readonly identity: HostIdentity
  readonly store: HostStore
  readonly fs: HostFileSystem
  readonly process: HostProcess

  private send: ((message: BackendToRunnerMessage) => void) | null = null
  private readonly pendingCalls = new Map<string, Deferred<unknown>>()
  private readonly activeSpawns = new Map<string, RemoteSpawn>()

  constructor(options: RemoteHostOptions) {
    this.defaultCwd = options.defaultCwd
    this.commandArtifactsDir = options.commandArtifactsDir
    this.identity = options.identity
    this.store = options.store
    this.fs = createRemoteFs((op, args) => this.call(op, args))
    this.process = {
      spawn: (params) => this.spawn(params),
      openCwd: async (path) => this.openCwd(path),
    }
  }

  /** Binds the current connection. In-flight work from a previous connection must already be detached. */
  attach(send: (message: BackendToRunnerMessage) => void): void {
    this.send = send
  }

  /** Marks the runner offline: pending fs calls reject and in-flight spawns die. */
  detach(reason = 'runner disconnected'): void {
    this.send = null
    const pending = [...this.pendingCalls.values()]
    this.pendingCalls.clear()
    for (const call of pending) {
      call.reject(offlineError(reason))
    }
    const spawns = [...this.activeSpawns.values()]
    this.activeSpawns.clear()
    for (const spawn of spawns) {
      spawn.finish({ exitCode: null, signal: reason, spawnError: { kind: 'other' } })
    }
  }

  get online(): boolean {
    return this.send !== null
  }

  /** Remote processes currently in flight (diagnostics). */
  get activeSpawnCount(): number {
    return this.activeSpawns.size
  }

  /** Routes runner messages that belong to this Host (fs results, spawn streams). */
  handleMessage(message: RunnerToBackendMessage): void {
    if (message.type === 'fs_result') {
      const pending = this.pendingCalls.get(message.id)
      if (!pending) return
      this.pendingCalls.delete(message.id)
      if (message.ok) pending.resolve(message.result)
      else pending.reject(wireCallError(message.error))
      return
    }
    if (message.type === 'spawn_output') {
      this.activeSpawns.get(message.spawnId)?.pushChunk({ stream: message.stream, chunk: message.bytes })
      return
    }
    if (message.type === 'spawn_exit') {
      const spawn = this.activeSpawns.get(message.spawnId)
      if (!spawn) return
      this.activeSpawns.delete(message.spawnId)
      spawn.finish({
        exitCode: message.exitCode,
        signal: message.signal,
        ...(message.spawnError ? { spawnError: message.spawnError } : {}),
      })
    }
  }

  private dispatch(message: BackendToRunnerMessage): void {
    if (!this.send) throw offlineError('runner disconnected')
    this.send(message)
  }

  private async call(op: HostFsOp, args: unknown[]): Promise<unknown> {
    if (!this.send) throw offlineError('runner disconnected')
    const id = createId()
    const pending = deferred<unknown>()
    this.pendingCalls.set(id, pending)
    try {
      this.send({ type: 'fs_call', id, op, args })
    } catch (error) {
      this.pendingCalls.delete(id)
      throw error
    }
    return pending.promise
  }

  private async spawn(params: HostSpawnParams): Promise<HostSpawnHandle> {
    if (!this.send) {
      return failedSpawnHandle({ exitCode: null, signal: 'runner disconnected', spawnError: { kind: 'other' } })
    }
    const spawnId = createId()
    const spawn = new RemoteSpawn(spawnId, (message) => this.dispatch(message))
    this.activeSpawns.set(spawnId, spawn)
    try {
      this.send({
        type: 'spawn',
        spawnId,
        command: params.command,
        ...(params.args ? { args: params.args } : {}),
        ...(params.cwd !== undefined ? { cwd: params.cwd } : {}),
        ...(params.env ? { env: params.env } : {}),
        ...(params.killProcessGroup !== undefined ? { killProcessGroup: params.killProcessGroup } : {}),
      })
    } catch (error) {
      this.activeSpawns.delete(spawnId)
      throw error
    }
    return spawn.handle()
  }

  private openCwd(path: string): HostCwd {
    return createLogicalHostCwd(path)
  }
}

function offlineError(reason: string): Error {
  return Object.assign(new Error(reason), { code: 'ERUNNEROFFLINE' })
}

function wireCallError(error: WireCallError): Error {
  const rebuilt = new Error(error.message)
  if (error.code) Object.assign(rebuilt, { code: error.code })
  return rebuilt
}

/**
 * One remote process: buffers the ordered, stream-tagged chunk sequence from
 * the runner and derives the handle's `stdout` / `stderr` / merged `output`
 * views from it — three independent cursors over one log, so no view
 * double-counts and each sees the full history regardless of when iteration
 * starts.
 */
class RemoteSpawn {
  private readonly chunks: HostProcessOutputChunk[] = []
  private done = false
  private exit: HostSpawnExit | null = null
  private readonly exitPromise = deferred<HostSpawnExit>()
  private readonly waiters = new Set<() => void>()

  constructor(
    private readonly spawnId: string,
    private readonly send: (message: BackendToRunnerMessage) => void,
  ) {}

  pushChunk(chunk: HostProcessOutputChunk): void {
    if (this.done) return
    this.chunks.push(chunk)
    this.wake()
  }

  finish(exit: HostSpawnExit): void {
    if (this.done) return
    this.done = true
    this.exit = exit
    this.exitPromise.resolve(exit)
    this.wake()
  }

  handle(): HostSpawnHandle {
    return {
      stdout: this.bytesView('stdout'),
      stderr: this.bytesView('stderr'),
      output: this.mergedView(),
      writeStdin: async (data) => {
        if (this.done) return
        this.send({ type: 'spawn_stdin', spawnId: this.spawnId, bytes: data })
      },
      closeStdin: async () => {
        if (this.done) return
        this.send({ type: 'spawn_stdin_end', spawnId: this.spawnId })
      },
      kill: async (signal) => {
        if (this.done) return
        this.send({ type: 'spawn_kill', spawnId: this.spawnId, ...(signal ? { signal } : {}) })
      },
      wait: () => this.exitPromise.promise,
    }
  }

  private wake(): void {
    const waiters = [...this.waiters]
    this.waiters.clear()
    for (const waiter of waiters) waiter()
  }

  /** Each call is an independent cursor over the full chunk log. */
  private async *entries(): AsyncIterable<HostProcessOutputChunk> {
    let cursor = 0
    while (true) {
      if (cursor < this.chunks.length) {
        const item = this.chunks[cursor]
        cursor += 1
        yield item
        continue
      }
      if (this.done) return
      await new Promise<void>((resolve) => {
        this.waiters.add(resolve)
      })
    }
  }

  private async *bytesView(stream: 'stdout' | 'stderr'): AsyncIterable<Uint8Array> {
    for await (const item of this.entries()) {
      if (item.stream === stream) yield item.chunk
    }
  }

  private async *mergedView(): AsyncIterable<HostProcessOutputChunk> {
    yield* this.entries()
  }
}

function failedSpawnHandle(exit: HostSpawnExit): HostSpawnHandle {
  return {
    stdout: emptyStream(),
    stderr: emptyStream(),
    output: emptyStream(),
    writeStdin: async () => {},
    closeStdin: async () => {},
    kill: async () => {},
    wait: async () => exit,
  }
}

async function* emptyStream(): AsyncIterable<never> {}

function createRemoteFs(call: (op: HostFsOp, args: unknown[]) => Promise<unknown>): HostFileSystem {
  return {
    readFile: (path, options) => call('readFile', dropUndefined([path, options])) as Promise<Uint8Array>,
    writeFile: (path, data, options) => call('writeFile', dropUndefined([path, data, options])) as Promise<void>,
    appendFile: (path, data, options) => call('appendFile', dropUndefined([path, data, options])) as Promise<void>,
    exists: (path, options) => call('exists', dropUndefined([path, options])) as Promise<boolean>,
    stat: (path, options) => call('stat', dropUndefined([path, options])) as Promise<HostFileStat>,
    lstat: (path, options) => call('lstat', dropUndefined([path, options])) as Promise<HostFileStat>,
    readdir: ((path: string, options?: { cwd?: string; withFileTypes?: boolean }) =>
      call('readdir', dropUndefined([path, options]))) as HostFileSystem['readdir'],
    mkdir: (path, options) => call('mkdir', dropUndefined([path, options])) as Promise<void>,
    rm: (path, options) => call('rm', dropUndefined([path, options])) as Promise<void>,
    cp: (path, destination, options) => call('cp', dropUndefined([path, destination, options])) as Promise<void>,
    mv: (path, destination, options) => call('mv', dropUndefined([path, destination, options])) as Promise<void>,
    chmod: (path, mode, options) => call('chmod', dropUndefined([path, mode, options])) as Promise<void>,
    symlink: (target, path, options) => call('symlink', dropUndefined([target, path, options])) as Promise<void>,
    link: (existingPath, path, options) => call('link', dropUndefined([existingPath, path, options])) as Promise<void>,
    readlink: (path, options) => call('readlink', dropUndefined([path, options])) as Promise<string>,
    realpath: (path, options) => call('realpath', dropUndefined([path, options])) as Promise<string>,
    utimes: (path, atime, mtime, options) => call('utimes', dropUndefined([path, atime, mtime, options])) as Promise<void>,
  }
}

/** Trailing undefined options are dropped so the wire carries the caller's actual arity. */
function dropUndefined(args: unknown[]): unknown[] {
  const trimmed = [...args]
  while (trimmed.length > 0 && trimmed[trimmed.length - 1] === undefined) trimmed.pop()
  return trimmed
}
