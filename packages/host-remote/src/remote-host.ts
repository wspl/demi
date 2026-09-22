import {
  artifactLocationSchema,
  type ArtifactResolver,
  type CommandContext,
  type NativeArtifact,
  type NativePackage,
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
import {
  collectBytes,
  createId,
  deferred,
  errorMessage,
  noop,
  withTimeout,
  type Deferred
} from '@demicodes/utils'
import { fsOps, gitOps, logPageSchema, STDIN_CHUNK_BYTES } from '@demicodes/runner-protocol'
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
  LogCursor,
  LogPage,
  JobExitMessage,
  NetErrorCode,
  PipeRef,
  RunnerToBackendMessage,
  ServiceErrorCode
} from '@demicodes/runner-protocol'
import type { HostPipes, Pipe } from './pipes'

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
 * The Host's log (`runner.md` § Host log): up to `limit` lines after `since`,
 * oldest first, of one `source` when named; without `since` the page ends at
 * the newest line. The runner answers from its files, so a read starts
 * nothing on the Host. A log the runner cannot read is a `RemoteLogError`.
 */
export interface RemoteLog {
  read(params: { since?: LogCursor; limit: number; source?: string }): Promise<LogPage>
}

export class RemoteLogError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'RemoteLogError'
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
 * The runner's service facet (`runner.md` § Service streams): one user stream
 * as an invocation of a resident native service, its input and output the
 * two pipes. `open` resolves when the runner answers `service_opened`; a
 * refusal is a `RemoteServiceError` with the runner's code. Until the stream
 * is closed, it answers the runner's requests for its service's executable.
 */
export interface RemoteServices {
  open(params: {
    context: CommandContext
    package: NativePackage
    operation: string
    /** The operation's arguments and whether it answers in JSON, as a command's invocation carries them. */
    args?: Record<string, unknown>
    json?: boolean
    cwd: string
    input: PipeRef
    output: PipeRef
    resolveArtifact: ArtifactResolver
  }): Promise<RemoteServiceStream>
  /**
   * One question, one answer: opens a stream, sends `input` as its whole
   * input and returns its whole output, which may not exceed `maxBytes`. An
   * invocation that exits nonzero rejects with a `RemoteServiceExit`.
   */
  call(params: {
    context: CommandContext
    package: NativePackage
    operation: string
    args?: Record<string, unknown>
    json?: boolean
    cwd: string
    input: Uint8Array
    resolveArtifact: ArtifactResolver
    maxBytes: number
    signal?: AbortSignal
  }): Promise<Uint8Array>
}

export interface RemoteServiceStream {
  /** How the invocation completed; rejects when the runner goes before it says. */
  done: Promise<{ exitCode: number; stderr: string }>
  /** The stream is over: its artifact requests are no longer answered. */
  close(): void
}

/** A one-shot call's invocation exited nonzero; `stderr` is the tail of what it wrote there. */
export class RemoteServiceExit extends Error {
  constructor(
    readonly exitCode: number,
    readonly stderr: string,
    readonly stdout: Uint8Array,
  ) {
    super(`Service call exited with ${exitCode}${stderr.trim() ? `: ${stderr.trim()}` : ''}`)
    this.name = 'RemoteServiceExit'
  }
}

export class RemoteServiceError extends Error {
  readonly code: ServiceErrorCode

  constructor(code: ServiceErrorCode, message: string) {
    super(message)
    this.name = 'RemoteServiceError'
    this.code = code
  }
}

/** An artifact request's owner: the live job or user stream it serves. */
type ArtifactOwner = Extract<RunnerToBackendMessage, { type: 'artifact_resolve' }>['owner']

/** What the live work an artifact request names may run. */
interface ArtifactGrant {
  packages: NativePackage[]
  resolve: ArtifactResolver
  signal: AbortSignal
  live(): boolean
}

interface ServiceStreamState {
  package: NativePackage
  resolveArtifact: ArtifactResolver
  controller: AbortController
  /** The runner's `service_done`. */
  done: Deferred<{ exitCode: number; stderr: string }>
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
  /** The pipes the Host's file contents travel through (`runner.md` § File contents). */
  pipes: HostPipes
}

/**
 * The backend-side `Host` over a connected runner: every `fs` method is one
 * `fs_call` round trip, with a file's contents in a pipe beside it,
 * `process.spawn` streams over `spawn_*` messages, and
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
  readonly log: RemoteLog
  readonly net: RemoteNet
  readonly services: RemoteServices

  private send: ((message: BackendToRunnerMessage) => void) | null = null
  /** The last manifest queued on this connection, not across reconnects. */
  private sentManifestHash: string | null = null
  private currentIdentity: HostIdentity
  private readonly pendingCalls = new Map<string, Deferred<unknown>>()
  private readonly pendingNet = new Map<string, Deferred<void>>()
  private readonly pendingServices = new Map<string, Deferred<void>>()
  private readonly serviceStreams = new Map<string, ServiceStreamState>()
  private readonly activeSpawns = new Map<string, RemoteSpawn>()
  /** The active spawns that are retained, which the activity count leaves out. */
  private readonly retainedSpawns = new Set<string>()
  private readonly activeJobs = new Map<string, RemoteJobState>()
  private readonly artifactRequests = new Set<string>()

  constructor(private readonly options: RemoteHostOptions) {
    this.defaultCwd = options.defaultCwd
    this.currentIdentity = options.identity
    this.store = options.store
    this.fs = createRemoteFs((op, params) => this.call(op, params), {
      readFile: async (path, options) => collectBytes((await this.openRead(path, options)).stream()),
      readStream: async (path, options) => {
        options?.signal?.throwIfAborted()
        return pipeBytes(await this.openRead(path, options), options?.signal)
      },
      writeFile: (path, data, options) => this.sendFile(path, data, options),
    })
    this.git = {
      changes: (root) => this.callGit('changes', { root }),
      show: async (root, path) => {
        const pipe = await this.receive((output) => this.callGit('show', { root, path, output }))
        return collectBytes(pipe.stream())
      },
    }
    this.log = { read: (params) => this.readLog(params) }
    this.net = { open: (params) => this.openNet(params) }
    this.services = {
      open: (params) => this.openService(params),
      call: (params) => this.callService(params),
    }
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
    this.sentManifestHash = null
    if (identity)
      this.currentIdentity = identity
  }

  /**
   * Marks the runner offline: pending fs calls reject and in-flight spawns die.
   */
  detach(reason = 'runner disconnected'): void {
    this.send = null
    this.sentManifestHash = null
    const pending = [...this.pendingCalls.values()]
    this.pendingCalls.clear()
    for (const call of pending) {
      call.reject(offlineError(reason))
    }
    const streams = [...this.pendingNet.values(), ...this.pendingServices.values()]
    this.pendingNet.clear()
    this.pendingServices.clear()
    for (const stream of streams) {
      stream.reject(offlineError(reason))
    }
    for (const stream of this.serviceStreams.values()) {
      stream.controller.abort(offlineError(reason))
      stream.done.reject(offlineError(reason))
    }
    this.serviceStreams.clear()
    const spawns = [...this.activeSpawns.values()]
    this.activeSpawns.clear()
    this.retainedSpawns.clear()
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
    return this.activeSpawns.size - this.retainedSpawns.size
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
   * The command context of a live job, recorded before dispatch; absent after
   * exit/disconnect (`native-runtime.md` § Command context).
   */
  jobContext(jobId: string): CommandContext | null {
    return this.activeJobs.get(jobId)?.context ?? null
  }

  /** Whether the live job or user stream an artifact request names is this Host's. */
  serves(owner: ArtifactOwner): boolean {
    return 'jobId' in owner
      ? this.activeJobs.has(owner.jobId)
      : this.serviceStreams.has(owner.streamId)
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
    /** What the job's declared commands receive; the backend built it. */
    context: CommandContext
  }): RemoteJob {
    const release = this.options.admit?.()
    const jobId = createId()
    const job = new RemoteJobState(
      jobId,
      (message) => this.dispatch(message),
      params.context,
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
      if (params.commands && this.sentManifestHash !== params.commands.manifest.hash) {
        this.send({ type: 'manifest', manifest: params.commands.manifest })
        this.sentManifestHash = params.commands.manifest.hash
      }
      this.send({
        type: 'job_start',
        ...(params.commands ? { manifestHash: params.commands.manifest.hash } : {}),
        context: params.context,
        jobId,
        script: params.script,
        cwd: params.cwd,
        env: params.env,
        ...(params.stdin ? { stdin: params.stdin } : {}),
        ...(params.stdout ? { stdout: params.stdout } : {}),
      })
    } catch (error) {
      // A throwing transport may have queued a partial send; select again next time.
      this.sentManifestHash = null
      this.activeJobs.delete(jobId)
      job.finish({ files: [], filesTruncated: false, exitCode: null, signal: errorMessage(error), spawnError: { kind: 'other' } })
      throw error
    }
    return job.handle()
  }

  /**
   * Answers the runner's request for an executable's location, for the live
   * job or user stream that runs it and only while it does.
   */
  private async resolveArtifact(message: Extract<RunnerToBackendMessage, { type: 'artifact_resolve' }>): Promise<void> {
    const send = this.send
    if (!send)
      return
    const grant = this.artifactGrant(message.owner)
    if (!grant) {
      send({ type: 'artifact_location', id: message.id, error: 'No matching live job or stream' })
      return
    }
    if (this.artifactRequests.has(message.id) || this.artifactRequests.size >= 32) {
      send({ type: 'artifact_location', id: message.id, error: 'Artifact resolution request limit or duplicate id' })
      return
    }
    this.artifactRequests.add(message.id)
    try {
      const artifact = grant.packages
        .map(descriptor => descriptor.targets[message.target])
        .find((artifact): artifact is NativeArtifact => artifact?.sha256 === message.sha256)
      if (!artifact)
        throw new Error('Artifact does not belong to the live work\'s packages')
      const location = artifactLocationSchema.parse(await grant.resolve(artifact, grant.signal))
      if (this.send === send && grant.live())
        send({ type: 'artifact_location', id: message.id, location })
    } catch (error) {
      if (this.send === send && grant.live())
        send({ type: 'artifact_location', id: message.id, error: errorMessage(error) })
    } finally {
      this.artifactRequests.delete(message.id)
    }
  }

  private artifactGrant(owner: ArtifactOwner): ArtifactGrant | null {
    if ('jobId' in owner) {
      const job = this.activeJobs.get(owner.jobId)
      const commands = job?.commands
      if (!job || !commands || commands.manifest.hash !== owner.manifestHash)
        return null
      return {
        packages: Object.values(commands.manifest.packages),
        resolve: commands.resolveArtifact,
        signal: job.signal,
        live: () => this.activeJobs.get(owner.jobId) === job,
      }
    }
    const stream = this.serviceStreams.get(owner.streamId)
    if (!stream)
      return null
    return {
      packages: [stream.package],
      resolve: stream.resolveArtifact,
      signal: stream.controller.signal,
      live: () => this.serviceStreams.get(owner.streamId) === stream,
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
    if (message.type === 'log_lines' || message.type === 'log_error') {
      const pending = this.pendingCalls.get(message.id)
      if (!pending)
        return
      this.pendingCalls.delete(message.id)
      if (message.type === 'log_lines') pending.resolve({ lines: message.lines, next: message.next })
      else pending.reject(new RemoteLogError(message.message))
      return
    }
    if (message.type === 'service_opened' || message.type === 'service_error') {
      const pending = this.pendingServices.get(message.streamId)
      if (!pending)
        return
      this.pendingServices.delete(message.streamId)
      if (message.type === 'service_opened') pending.resolve()
      else pending.reject(new RemoteServiceError(message.code, message.message))
      return
    }
    if (message.type === 'service_done') {
      this.serviceStreams.get(message.streamId)?.done.resolve({
        exitCode: message.exitCode,
        stderr: message.stderr,
      })
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
      this.retainedSpawns.delete(message.spawnId)
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

  /**
   * A pipe the runner fills once `request` succeeds (`runner.md` § File
   * contents). A refused request fails the pipe, since its bytes will never
   * come.
   */
  private async receive(request: (output: PipeRef) => Promise<unknown>): Promise<Pipe> {
    const pipe = this.options.pipes.fromRunner()
    // The pipe's outcome reaches whoever reads it; nothing else awaits it.
    pipe.done.catch(noop)
    try {
      await request(pipe.ref())
    } catch (error) {
      pipe.fail(errorMessage(error))
      throw error
    }
    return pipe
  }

  /**
   * `fs_readFile`: the pipe the runner streams `length` bytes from `offset`
   * into, once the runner has the file open.
   */
  private openRead(
    path: string,
    options?: { cwd?: string; offset?: number; length?: number },
  ): Promise<Pipe> {
    return this.receive((output) => this.call('readFile', {
      path,
      cwd: options?.cwd,
      offset: options?.offset,
      length: options?.length,
      output,
    }))
  }

  /**
   * `fs_writeFile`: this process fills the pipe while the runner writes it
   * into place, taking the next chunk of `data` only once the pipe took the
   * last. The runner's reply says how the write went, and it comes only
   * after the runner read the whole pipe. A stream that fails, or `signal`,
   * fails the pipe instead of ending it, so the runner drops what it has
   * and the file stays as it was; that failure, not the runner's echo of
   * it, is the one thrown.
   */
  private async sendFile(
    path: string,
    data: Uint8Array | AsyncIterable<Uint8Array>,
    options?: { cwd?: string; createParents?: boolean; signal?: AbortSignal },
  ): Promise<void> {
    const signal = options?.signal
    signal?.throwIfAborted()
    const pipe = this.options.pipes.toRunner()
    pipe.done.catch(noop)
    const writer = pipe.writer()
    let cause: unknown = null
    const stop = (reason: unknown) => {
      cause ??= reason
      pipe.fail(errorMessage(reason))
    }
    const abort = () => stop(signal?.reason)
    signal?.addEventListener('abort', abort, { once: true })
    // A write into a failed pipe rejects and returns the stream; whoever
    // failed the pipe reported why, the stream itself through `stop`.
    const upload = (async () => {
      for await (const chunk of chunksOf(data, stop))
        await writer.write(chunk)
      writer.end()
    })().catch(noop)
    try {
      await this.call('writeFile', {
        path,
        cwd: options?.cwd,
        createParents: options?.createParents,
        input: pipe.ref(),
      })
    } catch (error) {
      pipe.fail(errorMessage(error))
      throw cause ?? error
    } finally {
      signal?.removeEventListener('abort', abort)
    }
    await upload
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

  /** One `log_read` request, answered by `log_lines` or `log_error` under its id. */
  private async readLog(params: {
    since?: LogCursor
    limit: number
    source?: string
  }): Promise<LogPage> {
    if (!this.send)
      throw offlineError('runner disconnected')
    const id = createId()
    const pending = deferred<unknown>()
    this.pendingCalls.set(id, pending)
    try {
      // An absent field must stay off the wire: MessagePack would carry
      // `undefined` as nil, which the runner's contract refuses.
      this.send({
        type: 'log_read',
        id,
        limit: params.limit,
        ...definedFields({ since: params.since, source: params.source }),
      })
      return logPageSchema.parse(await pending.promise)
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

  /**
   * One `service_open` stream, answered by `service_opened` under the stream
   * id; a refusal rejects with the runner's code. The pipes' own outcomes
   * arrive later as `pipe_done`. It reserves no machine activity: a user
   * stream is retention (`resource-lifecycle.md` § Activity).
   */
  private async openService(params: Parameters<RemoteServices['open']>[0]): Promise<RemoteServiceStream> {
    const send = this.send
    if (!send)
      throw offlineError('runner disconnected')
    const streamId = createId()
    const pending = deferred<void>()
    const stream: ServiceStreamState = {
      package: params.package,
      resolveArtifact: params.resolveArtifact,
      controller: new AbortController(),
      done: deferred(),
    }
    // A long stream's owner follows its pipes, not this; a settled outcome must not reject unheard.
    stream.done.promise.catch(() => {})
    const close = () => {
      stream.controller.abort(new Error('Service stream closed'))
      stream.done.reject(new Error('Service stream closed'))
      if (this.serviceStreams.get(streamId) === stream)
        this.serviceStreams.delete(streamId)
    }
    this.pendingServices.set(streamId, pending)
    this.serviceStreams.set(streamId, stream)
    try {
      send({
        type: 'service_open',
        streamId,
        context: params.context,
        package: params.package,
        operation: params.operation,
        ...(params.args ? { args: params.args } : {}),
        ...(params.json === undefined ? {} : { json: params.json }),
        cwd: params.cwd,
        input: params.input,
        output: params.output,
      })
      await pending.promise
    } catch (error) {
      this.pendingServices.delete(streamId)
      close()
      throw error
    }
    return { done: stream.done.promise, close }
  }

  private async callService(
    params: Parameters<RemoteServices['call']>[0]
  ): Promise<Uint8Array> {
    const { input: bytes, maxBytes, signal, ...open } = params
    signal?.throwIfAborted()
    const input = this.options.pipes.toRunner()
    const output = this.options.pipes.fromRunner()
    // Each end is observed below; a settled pipe must not reject unheard.
    input.done.catch(() => {})
    output.done.catch(() => {})
    const abandon = (reason: string) => {
      input.fail(reason)
      output.fail(reason)
    }
    const aborted = () => abandon('service call cancelled')
    signal?.addEventListener('abort', aborted, { once: true })
    let stream: RemoteServiceStream | null = null
    try {
      stream = await this.openService({
        ...open,
        input: input.ref(),
        output: output.ref(),
      })
      const writer = input.writer()
      await writer.write(bytes)
      writer.end()
      const chunks: Uint8Array[] = []
      let size = 0
      for await (const chunk of output.stream()) {
        size += chunk.length
        if (size > maxBytes)
          throw new Error(`Service answer exceeds ${maxBytes} bytes`)
        chunks.push(chunk)
      }
      signal?.throwIfAborted()
      const outcome = await stream.done
      const answer = new Uint8Array(size)
      let offset = 0
      for (const chunk of chunks) {
        answer.set(chunk, offset)
        offset += chunk.length
      }
      if (outcome.exitCode !== 0)
        throw new RemoteServiceExit(outcome.exitCode, outcome.stderr, answer)
      return answer
    } catch (error) {
      abandon('service call failed')
      throw signal?.aborted ? signal.reason : error
    } finally {
      signal?.removeEventListener('abort', aborted)
      stream?.close()
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
    // A retained process is retention, not activity
    // (`resource-lifecycle.md` § Activity): it holds no admission and is not
    // counted, so the machine can go idle and stop with it running.
    const release = params.retained ? undefined : this.options.admit?.()
    const spawnId = createId()
    const spawn = new RemoteSpawn(spawnId, (message) => this.dispatch(message))
    void spawn.handle().wait().finally(() => release?.())
    if (params.retained)
      this.retainedSpawns.add(spawnId)
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
      this.retainedSpawns.delete(spawnId)
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
    readonly context: CommandContext,
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

/**
 * The bytes the runner puts in `pipe`. Ending the iteration early stops the
 * runner's read; so does aborting `signal`, which fails the pipe, whether or
 * not the iteration has begun.
 */
function pipeBytes(pipe: Pipe, signal?: AbortSignal): AsyncIterable<Uint8Array> {
  const abort = () => pipe.fail(signal?.reason === undefined ? 'the reader went away' : errorMessage(signal.reason))
  signal?.addEventListener('abort', abort, { once: true })
  if (signal?.aborted)
    abort()
  return (async function* () {
    try {
      yield* pipe.stream()
    } finally {
      signal?.removeEventListener('abort', abort)
    }
  })()
}

type RemoteCall = <Op extends FsOp>(
  op: Op,
  params: FsParams<Op>
) => Promise<FsResult<Op>>

/**
 * `data` as the chunks a write sends: bytes at once as one. A failure of the
 * stream itself goes to `failed` before it ends the loop; a loop that stops
 * early returns the stream.
 */
async function* chunksOf(
  data: Uint8Array | AsyncIterable<Uint8Array>,
  failed: (error: unknown) => void,
): AsyncGenerator<Uint8Array> {
  if (data instanceof Uint8Array) {
    yield data
    return
  }
  try {
    yield* data
  } catch (error) {
    failed(error)
    throw error
  }
}

/**
 * The `fs` facet: metadata operations are one call each; `contents` moves a
 * file's bytes through pipes.
 */
function createRemoteFs(
  call: RemoteCall,
  contents: Pick<HostFileSystem, 'readFile' | 'readStream' | 'writeFile'>,
): HostFileSystem {
  return {
    ...contents,
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
