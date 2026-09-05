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
import type { BackendToRunnerMessage, FsOp, FsParams, FsResult, JobExitMessage, PipeRef, RunnerToBackendMessage } from '@demicodes/runner-protocol'

/** One job on the runner as the backend drives it (`runner.md` § Jobs and the tee). */
export interface RemoteJob {
  /** The latest active registered invocation's hint; absent when none declares one. */
  readonly runningHint?: string
  /** The view: ordered, stream-tagged chunks while the job runs. */
  output: AsyncIterable<HostProcessOutputChunk>
  writeStdin(data: Uint8Array): Promise<void>
  closeStdin(): Promise<void>
  kill(signal?: string): Promise<void>
  wait(): Promise<RemoteJobExit>
}

export type RemoteJobExit = Omit<JobExitMessage, 'type' | 'jobId'>

export interface RemoteHostOptions {
  defaultCwd: string
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
  readonly identity: HostIdentity
  readonly store: HostStore
  readonly fs: HostFileSystem
  readonly process: HostProcess

  private send: ((message: BackendToRunnerMessage) => void) | null = null
  private readonly pendingCalls = new Map<string, Deferred<unknown>>()
  private readonly activeSpawns = new Map<string, RemoteSpawn>()
  private readonly activeJobs = new Map<string, RemoteJobState>()

  constructor(options: RemoteHostOptions) {
    this.defaultCwd = options.defaultCwd
    this.identity = options.identity
    this.store = options.store
    this.fs = createRemoteFs((op, params) => this.call(op, params))
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
    const jobs = [...this.activeJobs.values()]
    this.activeJobs.clear()
    for (const job of jobs) {
      job.finish({ exitCode: null, signal: reason, spawnError: { kind: 'other' } })
    }
  }

  get online(): boolean {
    return this.send !== null
  }

  /** Remote processes currently in flight (diagnostics). */
  get activeSpawnCount(): number {
    return this.activeSpawns.size
  }

  /** Jobs this Host started that have not exited (diagnostics). */
  get activeJobCount(): number {
    return this.activeJobs.size
  }

  /**
   * Starts `bash -c script` on the runner as one job; offline, the job fails
   * at once. `stdin` / `stdout` attach the job's fd 0 / fd 1 to pipes whose
   * other ends are elsewhere (`runner.md` § Pipes).
   */
  startJob(params: { script: string; cwd: string; env: Record<string, string>; stdin?: PipeRef; stdout?: PipeRef }): RemoteJob {
    const jobId = createId()
    const job = new RemoteJobState(jobId, (message) => this.dispatch(message))
    if (!this.send) {
      job.finish({ exitCode: null, signal: 'runner disconnected', spawnError: { kind: 'other' } })
      return job.handle()
    }
    this.activeJobs.set(jobId, job)
    try {
      this.send({
        type: 'job_start',
        jobId,
        script: params.script,
        cwd: params.cwd,
        env: params.env,
        ...(params.stdin ? { stdin: params.stdin } : {}),
        ...(params.stdout ? { stdout: params.stdout } : {}),
      })
    } catch (error) {
      this.activeJobs.delete(jobId)
      throw error
    }
    return job.handle()
  }

  /** Routes runner messages that belong to this Host (fs results, spawn streams). */
  handleMessage(message: RunnerToBackendMessage): void {
    if (message.type === 'fs_ok' || message.type === 'fs_error') {
      const pending = this.pendingCalls.get(message.id)
      if (!pending) return
      this.pendingCalls.delete(message.id)
      if (message.type === 'fs_ok') pending.resolve(message.result)
      else pending.reject(fsError(message.code, message.message))
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
      return
    }
    if (message.type === 'job_output') {
      this.activeJobs.get(message.jobId)?.pushChunk({ stream: message.stream, chunk: message.bytes })
      return
    }
    if (message.type === 'job_running_hint') {
      this.activeJobs.get(message.jobId)?.setRunningHint(message.invocationId, message.hint)
      return
    }
    if (message.type === 'job_exit') {
      const job = this.activeJobs.get(message.jobId)
      if (!job) return
      this.activeJobs.delete(message.jobId)
      const { type: _type, jobId: _jobId, ...exit } = message
      job.finish(exit)
    }
  }

  private dispatch(message: BackendToRunnerMessage): void {
    if (!this.send) throw offlineError('runner disconnected')
    this.send(message)
  }

  /** One `fs_<op>` request, answered by `fs_ok` or `fs_error` under its id. */
  private async call<Op extends FsOp>(op: Op, params: FsParams<Op>): Promise<FsResult<Op>> {
    if (!this.send) throw offlineError('runner disconnected')
    const id = createId()
    const pending = deferred<unknown>()
    this.pendingCalls.set(id, pending)
    try {
      this.send({ type: `fs_${op}`, id, ...definedFields(params) } as BackendToRunnerMessage)
    } catch (error) {
      this.pendingCalls.delete(id)
      throw error
    }
    return (await pending.promise) as FsResult<Op>
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

function fsError(code: string | undefined, message: string): Error {
  const rebuilt = new Error(message)
  if (code) Object.assign(rebuilt, { code })
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

/** One job's chunk log and exit, the same cursor model as a spawn. */
class RemoteJobState {
  private readonly chunks: HostProcessOutputChunk[] = []
  private readonly runningHints = new Map<string, string>()
  private done = false
  private readonly exitPromise = deferred<RemoteJobExit>()
  private readonly waiters = new Set<() => void>()

  constructor(
    private readonly jobId: string,
    private readonly send: (message: BackendToRunnerMessage) => void,
  ) {}

  pushChunk(chunk: HostProcessOutputChunk): void {
    if (this.done) return
    this.chunks.push(chunk)
    this.wake()
  }

  setRunningHint(invocationId: string, hint: string | null): void {
    if (this.done) return
    if (hint === null) this.runningHints.delete(invocationId)
    else this.runningHints.set(invocationId, hint)
  }

  finish(exit: RemoteJobExit): void {
    if (this.done) return
    this.done = true
    this.runningHints.clear()
    this.exitPromise.resolve(exit)
    this.wake()
  }

  handle(): RemoteJob {
    const hints = this.runningHints
    return {
      get runningHint() { return [...hints.values()].at(-1) },
      output: this.entries(),
      writeStdin: async (data) => {
        if (this.done) return
        this.send({ type: 'job_stdin', jobId: this.jobId, bytes: data })
      },
      closeStdin: async () => {
        if (this.done) return
        this.send({ type: 'job_stdin_end', jobId: this.jobId })
      },
      kill: async (signal) => {
        if (this.done) return
        this.send({ type: 'job_kill', jobId: this.jobId, ...(signal ? { signal } : {}) })
      },
      wait: () => this.exitPromise.promise,
    }
  }

  private wake(): void {
    const waiters = [...this.waiters]
    this.waiters.clear()
    for (const waiter of waiters) waiter()
  }

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

type RemoteCall = <Op extends FsOp>(op: Op, params: FsParams<Op>) => Promise<FsResult<Op>>

function createRemoteFs(call: RemoteCall): HostFileSystem {
  return {
    readFile: (path, options) => call('readFile', { path, cwd: options?.cwd }),
    writeFile: async (path, data, options) => void (await call('writeFile', { path, data, cwd: options?.cwd, createParents: options?.createParents })),
    appendFile: async (path, data, options) => void (await call('appendFile', { path, data, cwd: options?.cwd, createParents: options?.createParents })),
    exists: (path, options) => call('exists', { path, cwd: options?.cwd }),
    stat: (path, options) => call('stat', { path, cwd: options?.cwd }),
    lstat: (path, options) => call('lstat', { path, cwd: options?.cwd }),
    readdir: ((path: string, options?: { cwd?: string; withFileTypes?: boolean }) =>
      call('readdir', { path, cwd: options?.cwd, withFileTypes: options?.withFileTypes })) as HostFileSystem['readdir'],
    mkdir: async (path, options) => void (await call('mkdir', { path, cwd: options?.cwd, recursive: options?.recursive })),
    rm: async (path, options) => void (await call('rm', { path, cwd: options?.cwd, recursive: options?.recursive, force: options?.force })),
    cp: async (path, destination, options) => void (await call('cp', { path, destination, cwd: options?.cwd, recursive: options?.recursive })),
    mv: async (path, destination, options) => void (await call('mv', { path, destination, cwd: options?.cwd })),
    chmod: async (path, mode, options) => void (await call('chmod', { path, mode, cwd: options?.cwd })),
    symlink: async (target, path, options) => void (await call('symlink', { target, path, cwd: options?.cwd })),
    link: async (existingPath, path, options) => void (await call('link', { existingPath, path, cwd: options?.cwd })),
    readlink: (path, options) => call('readlink', { path, cwd: options?.cwd }),
    realpath: (path, options) => call('realpath', { path, cwd: options?.cwd }),
    utimes: async (path, atime, mtime, options) => void (await call('utimes', { path, atime, mtime, cwd: options?.cwd })),
  }
}

/** Undefined options are left off the frame; the wire carries what the caller set. */
function definedFields<T extends object>(params: T): Partial<T> {
  const defined: Partial<T> = {}
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined) (defined as Record<string, unknown>)[key] = value
  }
  return defined
}
