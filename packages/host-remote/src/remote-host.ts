import {
  artifactLocationSchema,
} from '@demicodes/command-protocol'
import type { RemoteCommandCatalog } from './shell-environment-factory'
import type {
  CommandStorage,
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
import { createId, deferred, errorMessage, withTimeout, type Deferred } from '@demicodes/utils'
import { fsOps, gitOps, STDIN_CHUNK_BYTES } from '@demicodes/runner-protocol'
import type {
  BackendToRunnerMessage,
  FsOp,
  FsParams,
  FsResult,
  GitChanges,
  GitErrorCode,
  GitOp,
  GitParams,
  GitResult,
  JobExitMessage,
  NetErrorCode,
  PipeRef,
  RunnerToBackendMessage
} from '@demicodes/runner-protocol'

/**
 * The runner's working-tree facet (`runner.md` § Working tree): the
 * uncommitted changes under a directory, and a file as the last commit has
 * it. Paths are the runner's; `path` is relative to `root`. A refusal is a
 * `RemoteGitError` with the runner's code.
 */
export interface RemoteGit {
  changes(root: string): Promise<GitChanges>
  show(root: string, path: string): Promise<Uint8Array>
}

export class RemoteGitError extends Error {
  readonly code: GitErrorCode

  constructor(code: GitErrorCode, message: string) {
    super(message)
    this.name = 'RemoteGitError'
    this.code = code
  }
}

/**
 * The runner's network facet (`runner.md` § Network streams): one TCP stream
 * on the device's network as two pipes whose other ends are elsewhere. `open`
 * resolves when the runner answers `net_opened`; a refusal is a
 * `RemoteNetError` with the runner's code.
 */
export interface RemoteNet {
  open(params: {
    host: string
    port: number
    input: PipeRef
    output: PipeRef
  }): Promise<void>
}

export class RemoteNetError extends Error {
  readonly code: NetErrorCode

  constructor(code: NetErrorCode, message: string) {
    super(message)
    this.name = 'RemoteNetError'
    this.code = code
  }
}

/**
 * One job on the runner as the backend drives it (`runner.md` § Jobs and the
 * tee).
 */
export interface RemoteJob {
  /**
   * The latest active registered invocation's hint; absent when none declares
   * one.
   */
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
  /** Reserves machine activity until the dispatched operation completes. */
  admit?: () => () => void
  /** From the runner `hello` — read synchronously at shell creation. */
  identity: HostIdentity
  /**
   * Backend-composed store; conversation state never crosses the runner
   * protocol.
   */
  store: HostStore
}

/**
 * The backend-side `Host` over a connected runner: every `fs` method is one
 * `fs_call` round trip, `process.spawn` streams over `spawn_*` messages, and
 * `openCwd` uses the contract's own logical path fallback (directory fds
 * cannot cross the wire). `git` is the runner's working-tree facet, one
 * `git_*` round trip per call, outside the `Host` contract.
 *
 * The object is stable across reconnects — `AgentHarness.host` must return
 * the same Host for the same execution target, so per-Host shell state
 * survives a runner going offline. `attach` binds the current connection;
 * `detach` fails everything in flight (surfacing as ordinary tool errors)
 * and leaves the Host offline until the next `attach`.
 */
export class RemoteHost implements Host {
  readonly defaultCwd: string
  readonly store: HostStore
  readonly fs: HostFileSystem
  readonly process: HostProcess
  readonly git: RemoteGit
  readonly net: RemoteNet

  private send: ((message: BackendToRunnerMessage) => void) | null = null
  private currentIdentity: HostIdentity
  private readonly pendingCalls = new Map<string, Deferred<unknown>>()
  private readonly pendingNet = new Map<string, Deferred<void>>()
  private readonly activeSpawns = new Map<string, RemoteSpawn>()
  private readonly activeJobs = new Map<string, RemoteJobState>()
  private readonly artifactRequests = new Set<string>()

  constructor(private readonly options: RemoteHostOptions) {
    this.defaultCwd = options.defaultCwd
    this.currentIdentity = options.identity
    this.store = options.store
    this.fs = createRemoteFs((op, params) => this.call(op, params))
    this.git = {
      changes: (root) => this.callGit('changes', { root }),
      show: (root, path) => this.callGit('show', { root, path }),
    }
    this.net = { open: (params) => this.openNet(params) }
    this.process = {
      spawn: (params) => this.spawn(params),
      openCwd: async (path) => this.openCwd(path),
    }
  }

  /**
   * The runner's identity from its hello; a Host made while its runner was
   * offline carries a placeholder until the first attach.
   */
  get identity(): HostIdentity {
    return this.currentIdentity
  }

  /**
   * Binds the current connection, and the runner's identity when the caller has
   * it. In-flight work from a previous connection must already be detached.
   */
  attach(
    send: (message: BackendToRunnerMessage) => void,
    identity?: HostIdentity,
  ): void {
    this.send = send
    if (identity)
      this.currentIdentity = identity
  }

  /**
   * Marks the runner offline: pending fs calls reject and in-flight spawns die.
   */
  detach(reason = 'runner disconnected'): void {
    this.send = null
    const pending = [...this.pendingCalls.values()]
    this.pendingCalls.clear()
    for (const call of pending) {
      call.reject(offlineError(reason))
    }
    const streams = [...this.pendingNet.values()]
    this.pendingNet.clear()
    for (const stream of streams) {
      stream.reject(offlineError(reason))
    }
    const spawns = [...this.activeSpawns.values()]
    this.activeSpawns.clear()
    for (const spawn of spawns) {
      spawn.finish({
        exitCode: null,
        signal: reason,
        spawnError: { kind: 'other' }
      })
    }
    const jobs = [...this.activeJobs.values()]
    this.activeJobs.clear()
    for (const job of jobs) {
      job.finish({ files: [], filesTruncated: false,
        exitCode: null,
        signal: reason,
        spawnError: { kind: 'other' }
      })
    }
  }

  get online(): boolean {
    return this.send !== null
  }

  /** Release state on the current connection without admitting work or waking a Host. */
  async releaseConversation(conversationId: string): Promise<void> {
    const send = this.send
    if (!send)
      return
    const id = createId()
    const pending = deferred<unknown>()
    this.pendingCalls.set(id, pending)
    try {
      send({ type: 'conversation_release', id, conversationId })
      await withTimeout(pending.promise, 360_000, 'Conversation release timed out')
    } finally {
      this.pendingCalls.delete(id)
    }
  }

  /** Remote processes currently in flight (diagnostics). */
  get activeSpawnCount(): number {
    return this.activeSpawns.size
  }

  /** Jobs this Host started that have not exited (diagnostics). */
  get activeJobCount(): number {
    return this.activeJobs.size
  }

  /** The agent-supplied storage binding recorded locally before dispatch. */
  jobCommandStorage(jobId: string): CommandStorage | undefined {
    return this.activeJobs.get(jobId)?.commandStorage
  }

  /**
   * The immutable environment of a live job, recorded before dispatch; absent
   * after exit/disconnect.
   */
  jobEnvironment(jobId: string): Readonly<Record<string, string>> | null {
    return this.activeJobs.get(jobId)?.env ?? null
  }

  /**
   * Runs the script on the runner as one job; offline, the job fails
   * at once. `stdin` / `stdout` attach the job's fd 0 / fd 1 to pipes whose
   * other ends are elsewhere (`runner.md` § Pipes).
   */
  startJob(params: {
    script: string;
    cwd: string;
    env: Record<string, string>;
    stdin?: PipeRef;
    stdout?: PipeRef;
    commandStorage?: CommandStorage;
    commands?: RemoteCommandCatalog
    conversation: string
    node: string
  }): RemoteJob {
    const release = this.options.admit?.()
    const jobId = createId()
    const job = new RemoteJobState(
      jobId,
      (message) => this.dispatch(message),
      Object.freeze({
        ...params.env,
        DEMI_CONVERSATION_ID: params.conversation,
        DEMI_AGENT_NODE_ID: params.node,
      }),
      params.commandStorage,
      params.commands ? { ...params.commands } : undefined
    )
    void job.handle().wait().finally(() => release?.())
    if (!this.send) {
      job.finish({ files: [], filesTruncated: false,
        exitCode: null,
        signal: 'runner disconnected',
        spawnError: { kind: 'other' }
      })
      return job.handle()
    }
    this.activeJobs.set(jobId, job)
    try {
      if (params.commands)
        this.send({ type: 'manifest', manifest: params.commands.manifest })
      this.send({
        type: 'job_start',
        ...(params.commands ? { manifestHash: params.commands.manifest.hash } : {}),
        conversation: params.conversation,
        node: params.node,
        jobId,
        script: params.script,
        cwd: params.cwd,
        env: params.env,
        ...(params.stdin ? { stdin: params.stdin } : {}),
        ...(params.stdout ? { stdout: params.stdout } : {}),
      })
    } catch (error) {
      this.activeJobs.delete(jobId)
      job.finish({ files: [], filesTruncated: false, exitCode: null, signal: errorMessage(error), spawnError: { kind: 'other' } })
      throw error
    }
    return job.handle()
  }

  private async resolveArtifact(message: Extract<RunnerToBackendMessage, { type: 'artifact_resolve' }>): Promise<void> {
    const send = this.send
    if (!send)
      return
    const job = this.activeJobs.get(message.jobId)
    if (!job?.commands || job.commands.manifest.hash !== message.manifestHash) {
      send({ type: 'artifact_location', id: message.id, error: 'No matching active job command catalog' })
      return
    }
    if (this.artifactRequests.has(message.id) || this.artifactRequests.size >= 32) {
      send({ type: 'artifact_location', id: message.id, error: 'Artifact resolution request limit or duplicate id' })
      return
    }
    this.artifactRequests.add(message.id)
    try {
      const artifact = Object.values(job.commands.manifest.packages)
        .map(descriptor => descriptor.targets[message.target])
        .find(artifact => artifact.sha256 === message.sha256)
      if (!artifact)
        throw new Error('Artifact does not belong to the active job catalog')
      const location = artifactLocationSchema.parse(await job.commands.resolveArtifact(artifact, job.signal))
      if (this.send === send && this.activeJobs.get(message.jobId) === job)
        send({ type: 'artifact_location', id: message.id, location })
    } catch (error) {
      if (this.send === send && this.activeJobs.get(message.jobId) === job)
        send({ type: 'artifact_location', id: message.id, error: errorMessage(error) })
    } finally {
      this.artifactRequests.delete(message.id)
    }
  }

  /**
   * Routes runner messages that belong to this Host (fs results, spawn
   * streams).
   */
  handleMessage(message: RunnerToBackendMessage): void {
    if (message.type === 'conversation_released') {
      const pending = this.pendingCalls.get(message.id)
      this.pendingCalls.delete(message.id)
      if (message.error !== undefined)
        pending?.reject(new Error(message.error))
      else
        pending?.resolve(undefined)
      return
    }
    if (message.type === 'artifact_resolve') {
      void this.resolveArtifact(message).catch(error => {
        // A transport failure belongs to the registry's connection lifecycle.
        console.error(`Artifact response delivery failed: ${errorMessage(error)}`)
      })
      return
    }
    if (message.type === 'fs_ok' || message.type === 'fs_error') {
      const pending = this.pendingCalls.get(message.id)
      if (!pending)
        return
      this.pendingCalls.delete(message.id)
      if (message.type === 'fs_ok') pending.resolve(message.result)
      else pending.reject(fsError(message.code, message.message))
      return
    }
    if (message.type === 'git_ok' || message.type === 'git_error') {
      const pending = this.pendingCalls.get(message.id)
      if (!pending)
        return
      this.pendingCalls.delete(message.id)
      if (message.type === 'git_ok') pending.resolve(message.result)
      else pending.reject(new RemoteGitError(message.code, message.message))
      return
    }
    if (message.type === 'net_opened' || message.type === 'net_error') {
      const pending = this.pendingNet.get(message.streamId)
      if (!pending)
        return
      this.pendingNet.delete(message.streamId)
      if (message.type === 'net_opened') pending.resolve()
      else pending.reject(new RemoteNetError(message.code, message.message))
      return
    }
    if (message.type === 'spawn_output') {
      this.activeSpawns.get(message.spawnId)?.pushChunk({
        stream: message.stream,
        chunk: message.bytes
      })
      return
    }
    if (message.type === 'spawn_exit') {
      const spawn = this.activeSpawns.get(message.spawnId)
      if (!spawn)
        return
      this.activeSpawns.delete(message.spawnId)
      spawn.finish({
        exitCode: message.exitCode,
        signal: message.signal,
        ...(message.spawnError ? { spawnError: message.spawnError } : {}),
      })
      return
    }
    if (message.type === 'job_output') {
      this.activeJobs.get(message.jobId)?.pushChunk({
        stream: message.stream,
        chunk: message.bytes
      })
      return
    }
    if (message.type === 'job_running_hint') {
      this.activeJobs.get(message.jobId)?.setRunningHint(
        message.invocationId,
        message.hint
      )
      return
    }
    if (message.type === 'job_exit') {
      const job = this.activeJobs.get(message.jobId)
      if (!job)
        return
      this.activeJobs.delete(message.jobId)
      const { type: _type, jobId: _jobId, ...exit } = message
      job.finish(exit)
    }
  }

  private dispatch(message: BackendToRunnerMessage): void {
    if (!this.send)
      throw offlineError('runner disconnected')
    this.send(message)
  }

  /**
   * One `fs_<op>` request, answered by `fs_ok` or `fs_error` under its id. The
   * reply is checked against the schema of the operation this caller asked
   * for, not the `op` the reply names itself: only the local one says what
   * shape the caller is waiting for.
   */
  private async call<Op extends FsOp>(
    op: Op,
    params: FsParams<Op>
  ): Promise<FsResult<Op>> {
    if (!this.send)
      throw offlineError('runner disconnected')
    const release = this.options.admit?.()
    const id = createId()
    const pending = deferred<unknown>()
    this.pendingCalls.set(id, pending)
    try {
      this.send(
        { type: `fs_${op}`, id, ...definedFields(params) } as BackendToRunnerMessage
      )
      // TypeScript widens `fsOps[op].result` to every operation's result
      // schema; the value is the one `op` named. Same step as
      // `MachineClient.call`.
      return fsOps[op].result.parse(await pending.promise) as FsResult<Op>
    } catch (error) {
      this.pendingCalls.delete(id)
      throw error
    } finally {
      release?.()
    }
  }

  private async callGit<Op extends GitOp>(
    op: Op,
    params: GitParams<Op>
  ): Promise<GitResult<Op>> {
    if (!this.send)
      throw offlineError('runner disconnected')
    const id = createId()
    const pending = deferred<unknown>()
    this.pendingCalls.set(id, pending)
    try {
      this.send({ type: `git_${op}`, id, ...params } as BackendToRunnerMessage)
      // Same widening as `call`: `gitOps[op].result` is every operation's
      // result schema; the value is the one `op` named.
      return gitOps[op].result.parse(await pending.promise) as GitResult<Op>
    } catch (error) {
      this.pendingCalls.delete(id)
      throw error
    }
  }

  /**
   * One `net_open` stream, answered by `net_opened` under the stream id; a
   * refusal rejects with the runner's code. The pipes' own outcomes arrive
   * later as `pipe_done`.
   */
  private async openNet(params: {
    host: string
    port: number
    input: PipeRef
    output: PipeRef
  }): Promise<void> {
    if (!this.send)
      throw offlineError('runner disconnected')
    const release = this.options.admit?.()
    const streamId = createId()
    const pending = deferred<void>()
    this.pendingNet.set(streamId, pending)
    try {
      this.send({
        type: 'net_open',
        streamId,
        host: params.host,
        port: params.port,
        input: params.input,
        output: params.output,
      })
      await pending.promise
    } catch (error) {
      this.pendingNet.delete(streamId)
      throw error
    } finally {
      release?.()
    }
  }

  private async spawn(params: HostSpawnParams): Promise<HostSpawnHandle> {
    if (!this.send) {
      return failedSpawnHandle({
        exitCode: null,
        signal: 'runner disconnected',
        spawnError: { kind: 'other' }
      })
    }
    const release = this.options.admit?.()
    const spawnId = createId()
    const spawn = new RemoteSpawn(spawnId, (message) => this.dispatch(message))
    void spawn.handle().wait().finally(() => release?.())
    this.activeSpawns.set(spawnId, spawn)
    try {
      this.send({
        type: 'spawn',
        spawnId,
        command: params.command,
        ...(params.args ? { args: params.args } : {}),
        ...(params.cwd !== undefined ? { cwd: params.cwd } : {}),
        ...(params.env ? { env: params.env } : {}),
        ...(params.inheritEnv !== undefined ? { inheritEnv: params.inheritEnv } : {}),
        ...(params.killProcessGroup !== undefined ? {
          killProcessGroup: params.killProcessGroup
        } : {}),
      })
    } catch (error) {
      this.activeSpawns.delete(spawnId)
      release?.()
      throw error
    }
    return spawn.handle()
  }

  private async openCwd(path: string): Promise<HostCwd> {
    const validate = async (candidate: string) => {
      const stat = await this.fs.stat(candidate)
      if (!stat.isDirectory)
        throw fsError('ENOTDIR', `Not a directory: ${candidate}`)
    }
    await validate(path)
    return createLogicalHostCwd(path, validate)
  }
}

function offlineError(reason: string): Error {
  return Object.assign(new Error(reason), { code: 'ERUNNEROFFLINE' })
}

function fsError(code: string | undefined, message: string): Error {
  const rebuilt = new Error(message)
  if (code)
    Object.assign(rebuilt, { code })
  return rebuilt
}

/** Splits a remote stdin write into ordered, bounded protocol frames. */
function sendStdin(
  send: (message: BackendToRunnerMessage) => void,
  message: Extract<BackendToRunnerMessage, { type: 'spawn_stdin' | 'job_stdin' }>,
): void {
  for (let offset = 0; offset < message.bytes.byteLength; offset += STDIN_CHUNK_BYTES) {
    send({
      ...message,
      bytes: message.bytes.subarray(offset, offset + STDIN_CHUNK_BYTES),
    })
  }
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
    if (this.done)
      return
    this.chunks.push(chunk)
    this.wake()
  }

  finish(exit: HostSpawnExit): void {
    if (this.done)
      return
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
        if (this.done)
          return
        sendStdin(this.send, { type: 'spawn_stdin', spawnId: this.spawnId, bytes: data })
      },
      closeStdin: async () => {
        if (this.done)
          return
        this.send({ type: 'spawn_stdin_end', spawnId: this.spawnId })
      },
      kill: async (signal) => {
        if (this.done)
          return
        this.send({
          type: 'spawn_kill',
          spawnId: this.spawnId,
          ...(signal ? { signal } : {})
        })
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
      if (this.done)
        return
      await new Promise<void>((resolve) => {
        this.waiters.add(resolve)
      })
    }
  }

  private async *bytesView(
    stream: 'stdout' | 'stderr'
  ): AsyncIterable<Uint8Array> {
    for await (const item of this.entries()) {
      if (item.stream === stream)
        yield item.chunk
    }
  }

  private async *mergedView(): AsyncIterable<HostProcessOutputChunk> {
    yield* this.entries()
  }
}

/** One job's chunk log and exit, the same cursor model as a spawn. */
class RemoteJobState {
  private readonly abort = new AbortController()
  get signal(): AbortSignal { return this.abort.signal }
  private readonly chunks: HostProcessOutputChunk[] = []
  private readonly runningHints = new Map<string, string>()
  private done = false
  private readonly exitPromise = deferred<RemoteJobExit>()
  private readonly waiters = new Set<() => void>()

  constructor(
    private readonly jobId: string,
    private readonly send: (message: BackendToRunnerMessage) => void,
    readonly env: Readonly<Record<string, string>>,
    readonly commandStorage?: CommandStorage,
    readonly commands?: RemoteCommandCatalog,
  ) {}

  pushChunk(chunk: HostProcessOutputChunk): void {
    if (this.done)
      return
    this.chunks.push(chunk)
    this.wake()
  }

  setRunningHint(invocationId: string, hint: string | null): void {
    if (this.done)
      return
    if (hint === null) this.runningHints.delete(invocationId)
    else this.runningHints.set(invocationId, hint)
  }

  finish(exit: RemoteJobExit): void {
    if (this.done)
      return
    this.done = true
    this.abort.abort(new Error('Runner job finished'))
    this.runningHints.clear()
    this.exitPromise.resolve(exit)
    this.wake()
  }

  handle(): RemoteJob {
    const hints = this.runningHints
    return {
      get runningHint() {
        return [...hints.values()].at(-1)
      },
      output: this.entries(),
      writeStdin: async (data) => {
        if (this.done)
          return
        sendStdin(this.send, { type: 'job_stdin', jobId: this.jobId, bytes: data })
      },
      closeStdin: async () => {
        if (this.done)
          return
        this.send({ type: 'job_stdin_end', jobId: this.jobId })
      },
      kill: async (signal) => {
        if (this.done)
          return
        this.send({
          type: 'job_kill',
          jobId: this.jobId,
          ...(signal ? { signal } : {})
        })
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
      if (this.done)
        return
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

type RemoteCall = <Op extends FsOp>(
  op: Op,
  params: FsParams<Op>
) => Promise<FsResult<Op>>

function createRemoteFs(call: RemoteCall): HostFileSystem {
  return {
    readFile: (path, options) => call('readFile', { path, cwd: options?.cwd }),
    writeFile: async (path, data, options) => void (await call(
      'writeFile',
      { path, data, cwd: options?.cwd, createParents: options?.createParents }
    )),
    appendFile: async (path, data, options) => void (await call(
      'appendFile',
      { path, data, cwd: options?.cwd, createParents: options?.createParents }
    )),
    exists: (path, options) => call('exists', { path, cwd: options?.cwd }),
    stat: (path, options) => call('stat', { path, cwd: options?.cwd }),
    lstat: (path, options) => call('lstat', { path, cwd: options?.cwd }),
    readdir: ((path: string, options?: {
      cwd?: string;
      withFileTypes?: boolean
    }) =>
      call(
        'readdir',
        { path, cwd: options?.cwd, withFileTypes: options?.withFileTypes }
      )) as HostFileSystem['readdir'],
    mkdir: async (path, options) => void (await call(
      'mkdir',
      { path, cwd: options?.cwd, recursive: options?.recursive }
    )),
    rm: async (path, options) => void (await call(
      'rm',
      {
        path,
        cwd: options?.cwd,
        recursive: options?.recursive,
        force: options?.force
      }
    )),
    cp: async (path, destination, options) => void (await call(
      'cp',
      { path, destination, cwd: options?.cwd, recursive: options?.recursive }
    )),
    mv: async (path, destination, options) => void (await call(
      'mv',
      { path, destination, cwd: options?.cwd }
    )),
    chmod: async (path, mode, options) => void (await call(
      'chmod',
      { path, mode, cwd: options?.cwd }
    )),
    symlink: async (target, path, options) => void (await call(
      'symlink',
      { target, path, cwd: options?.cwd }
    )),
    link: async (existingPath, path, options) => void (await call(
      'link',
      { existingPath, path, cwd: options?.cwd }
    )),
    readlink: (path, options) => call('readlink', { path, cwd: options?.cwd }),
    realpath: (path, options) => call('realpath', { path, cwd: options?.cwd }),
    utimes: async (path, atime, mtime, options) => void (await call(
      'utimes',
      { path, atime, mtime, cwd: options?.cwd }
    )),
  }
}

/**
 * Undefined options are left off the frame; the wire carries what the caller
 * set.
 */
function definedFields<T extends object>(params: T): Partial<T> {
  const defined: Partial<T> = {}
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined)
      (defined as Record<string, unknown>)[key] = value
  }
  return defined
}
