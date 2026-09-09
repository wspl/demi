import { ExecutionContexts, type ExecutionContext } from './commands/contexts'
import { directorySource } from '@demicodes/command-loader'
// Runner mode (`runner.md`): the one outbound WebSocket to the backend with
// its handshake and reconnect, the Host served over it, the job table, the
// manifest cache, and the local relay for native clients.
import {
  connectWebSocket,
  createRunnerHost,
  identity,
  msgpackDecode,
  msgpackEncode,
  readTail,
  spawnTeed,
  version as runtimeVersion,
  type WebSocketLink,
} from './machine'
import {
  RUNNER_PROTOCOL_VERSION,
  createRunnerWire,
  type BackendToRunnerMessage,
  type RunnerToBackendMessage,
} from '@demicodes/runner-protocol'
import { HostRpcServer } from './serve/host-rpc-server'
import { JobTable } from './serve/jobs'
import type { Host } from '@demicodes/shell'
import {
  collectBytes,
  createId,
  delay,
  dirnamePath,
  errorMessage,
  noop,
  SerialQueue
} from '@demicodes/utils'
import { ManifestCache } from './manifest-cache'
import { RelayServer } from './relay/server'
import { DirectoryVolume, type Volume } from './init/volume'
import { RunnerState } from './state'
import { PipeClient } from './pipes'

export interface RunnerModeOptions {
  backendUrl: string
  /** Machine-local state directory (`~/.demi`). */
  stateDir: string
  /** Device display name (default the hostname). */
  name?: string
  /** The packed binary the root-command symlinks point at. */
  clientExecutable: string
  /** Device facts jobs fall back to: `PATH`, `HOME`. */
  deviceEnv: Record<string, string>
  /**
   * Booted as a managed host: the hello says so, and a missing token is a
   * refusal, not a pairing.
   */
  managed?: boolean
  /**
   * A token held in memory only — PID 1's, off the kernel command line — taking
   * precedence over the state directory's.
   */
  deviceToken?: string
  /**
   * The identity reported and the user every job and spawn runs as; PID 1 names
   * the guest user here.
   */
  identity?: Host['identity']
  /** The home as the guest sees it (default: a directory that only syncs). */
  volumes?: Partial<Record<'home' | 'system', Volume>>
  /**
   * How often the home's room is checked between jobs (default a minute; 0
   * disables).
   */
  volumeCheckMs?: number
  reconnect?: {
    initialDelayMs?: number;
    maxDelayMs?: number
  }
  log?: (line: string) => void
}

export type RunnerStatus = 'connecting'
  | 'claim_pending'
  | 'online'
  | 'rejected'
  | 'stopped'

/** The runner program: `start` runs until `stop`, reconnecting with backoff. */
export class RunnerMode {
  private readonly host: Host
  private readonly state: RunnerState
  private readonly cache: ManifestCache
  private readonly pipes: PipeClient
  private readonly wire = createRunnerWire({
    encode: msgpackEncode,
    decode: msgpackDecode
  })
  private readonly log: (line: string) => void
  private readonly volumes: Partial<Record<'home' | 'system', Volume>>
  private readonly contexts = new ExecutionContexts()
  private endpoint = ''
  private draining = false
  private stopped = false
  private link: WebSocketLink | null = null
  private relay: RelayServer | null = null
  private volumeCheckTimer: ReturnType<typeof setInterval> | null = null
  private readonly growthPending = new Map<'home' | 'system', string>()

  constructor(private readonly options: RunnerModeOptions) {
    const runnerIdentity = options.identity ?? identity
    this.host = createRunnerHost({
      defaultCwd: runnerIdentity.homeDir,
      storeDir: `${options.stateDir}/store`,
      identity: runnerIdentity,
    })
    this.state = new RunnerState(this.host.fs, options.stateDir)
    this.cache = new ManifestCache(
      this.host.fs,
      this.state.commandsDir,
      options.clientExecutable
    )
    this.pipes = new PipeClient(options.backendUrl, () => this.token())
    this.log = options.log ?? ((line) => console.error(line))
    this.volumes = options.volumes ?? {
      home: new DirectoryVolume({
        run: (command, args) => this.command(command, args)
      })
    }
  }

  private executionEnvironment(
    context: ExecutionContext,
    env: Record<string, string | undefined>
  ): Record<string, string> {
    const bin = context.manifest
      ? this.cache.binDirectory(context.manifest)
      : dirnamePath(this.options.clientExecutable)
    return {
      ...this.contexts.environment(context, this.endpoint),
      PATH: `${bin}:${env.PATH ?? this.options.deviceEnv.PATH ?? '/usr/bin:/bin'}`,
    }
  }

  /** The device token: the one held in memory, else the state directory's. */
  private async token(): Promise<string | null> {
    return this.options.deviceToken ?? this.state.readToken()
  }

  /**
   * A command from the runner's own machine, run to its end — the home's
   * `sync`, `df`, `resize2fs`.
   */
  private async command(
    command: string,
    args: string[]
  ): Promise<{
    code: number | null;
    stdout: Uint8Array
  }> {
    const child = await this.host.process.spawn({ command, args })
    await child.closeStdin()
    const [stdout, exit] = await Promise.all([
      collectBytes(child.stdout),
      child.wait()
    ])
    return { code: exit.exitCode, stdout }
  }

  /**
   * Runs until stopped; resolves when the runner was told to stop or the
   * backend refused it for good.
   */
  async run(): Promise<'stopped' | 'rejected'> {
    const initial = this.options.reconnect?.initialDelayMs ?? 1_000
    const max = this.options.reconnect?.maxDelayMs ?? 30_000
    let backoff = initial
    await this.host.fs.mkdir(this.options.stateDir, { recursive: true })
    await this.host.fs.chmod(this.options.stateDir, 0o700)
    const lease = await tjs.open(
      `${this.options.stateDir}/runner.lock`,
      'a',
      0o600
    )
    if (!lease.lock()) {
      await lease.close()
      throw new Error('runner already active for this installation')
    }
    let runtimeDir: string | null = null
    let published = false
    try {
      const config = await this.state.readConfig()
      if (config && config.backendUrl !== this.options.backendUrl)
        throw new Error('state directory belongs to another backend')
      await this.state.writeConfig(config ?? {
        backendUrl: this.options.backendUrl
      })
      const startId = crypto.randomUUID().replaceAll('-', '')
      const windows = navigator.platform.startsWith('Win')
      runtimeDir = windows
        ? null
        : await tjs.makeTempDir(`${tjs.tmpDir}/demi-XXXXXX`)
      if (runtimeDir)
        await tjs.chmod(runtimeDir, 0o700)
      this.endpoint = windows
        ? String.raw`\\.\pipe\demi-${startId}`
        : `${runtimeDir}/ipc.sock`
      const secret = crypto.randomUUID().replaceAll('-', '')
      this.relay = await RelayServer.listen(this.endpoint, {
        send: message => this.sendToBackend(message),
        host: this.host,
        contexts: this.contexts,
        source: context => {
          if (!context.manifest)
            throw new Error('no command manifest for this execution context')
          return directorySource(
            `${this.state.commandsDir}/${context.manifest.hash}`,
            this.host.fs
          )
        },
        pipes: this.pipes,
        manageSecret: secret,
        drain: async () => {
          this.draining = true
          while (this.contexts.count) await delay(50)
        },
        stop: () => this.stop(),
      })
      const activeRunner = {
        endpoint: this.endpoint,
        secret,
        release: tjs.env.DEMI_RELEASE_ID ?? RUNNER_VERSION,
      }
      await this.host.fs.writeFile(
        this.state.activePath,
        new TextEncoder().encode(JSON.stringify(activeRunner))
      )
      published = true
      await this.host.fs.chmod(this.state.activePath, 0o600)
      const checkMs = this.options.volumeCheckMs ?? 60_000
      if (checkMs > 0)
        this.volumeCheckTimer = setInterval(
          () => void this.checkVolumes(),
          checkMs
        )
      while (!this.stopped) {
        this.log('connecting…')
        const outcome = await this.connectOnce()
        if (outcome === 'rejected')
          return 'rejected'
        if (outcome === 'online')
          backoff = initial
        if (this.stopped)
          break
        await delay(backoff)
        backoff = Math.min(backoff * 2, max)
      }
      return 'stopped'
    } finally {
      if (this.volumeCheckTimer)
        clearInterval(this.volumeCheckTimer)
      this.volumeCheckTimer = null
      this.relay?.close()
      this.relay = null
      this.contexts.clear()
      if (published)
        await this.host.fs.rm(
          this.state.activePath,
          { force: true }
        )
      if (runtimeDir)
        await this.host.fs.rm(
          runtimeDir,
          { recursive: true, force: true }
        )
      await lease.close()
    }
  }

  /**
   * The writable-volume growth request (`managed-hosts.md` § Home persistence): when
   * the filesystem nears its cap, ask once; `volume_grown` closes the
   * request by growing the filesystem into the enlarged image.
   */
  private async checkVolumes(): Promise<void> {
    if (!this.link)
      return
    for (const volume of ['home', 'system'] as const) {
      const image = this.volumes[volume]
      if (!image || this.growthPending.has(volume))
        continue
      const id = createId()
      this.growthPending.set(volume, id)
      try {
        const bytes = await image.wanted()
        if (bytes === null || !this.link) {
          this.growthPending.delete(volume)
          continue
        }
        this.sendToBackend({ type: 'volume_grow', id, volume, bytes })
      } catch (error) {
        this.growthPending.delete(volume)
        this.log(`${volume} check failed: ${errorMessage(error)}`)
      }
    }
  }

  async stop(): Promise<void> {
    this.stopped = true
    await this.link?.close()
  }

  private sendToBackend(message: RunnerToBackendMessage): void {
    const link = this.link
    if (!link)
      throw new Error('runner: not connected')
    // The receive loop owns disconnection and tears down all jobs and calls.
    void link.send(this.wire.encode(message)).catch(noop)
    // A job that just ended may have filled the home.
    if (message.type === 'spawn_exit') {
      this.relay?.cancelOwner(`spawn:${message.spawnId}`)
      this.contexts.remove(`spawn:${message.spawnId}`)
    }
    if (message.type === 'job_exit') {
      this.contexts.remove(`job:${message.jobId}`)
      this.relay?.cancelJob(message.jobId)
      void this.checkVolumes()
    }
  }

  /** One connection: hello, then the message loop until the socket closes. */
  private async connectOnce(): Promise<'closed' | 'online' | 'rejected'> {
    let link: WebSocketLink
    try {
      link = await connectWebSocket(runnerSocketUrl(this.options.backendUrl))
    } catch (error) {
      this.log(`connect failed: ${errorMessage(error)}`)
      return 'closed'
    }
    this.growthPending.clear()
    this.link = link
    const rpc = new HostRpcServer(
      this.host,
      (message) => this.sendToBackend(message),
      this.options.deviceEnv,
      async message => {
        const context = this.contexts.create(
          `spawn:${message.spawnId}`,
          {},
          await this.cache.current()
        )
        return this.executionEnvironment(context, message.env ?? {})
      }
    )
    const jobs = new JobTable({
      spawn: spawnTeed,
      outputDir: this.state.outputDir,
      fs: {
        mkdir: (path) => this.host.fs.mkdir(path, { recursive: true }),
        readTail,
        readFile: (path) => this.host.fs.readFile(path),
        rm: (path) => this.host.fs.rm(path, { force: true }),
      },
      deviceEnv: this.options.deviceEnv,
      // The state location is fixed; per-job context and manifest PATH are injected below.
      fixedEnv: { DEMI_HOME: this.options.stateDir },
      executionEnv: async message => {
        const context = this.contexts.create(
          `job:${message.jobId}`,
          message.env,
          await this.cache.current(),
          message.jobId
        )
        return this.executionEnvironment(context, message.env)
      },
      pipes: this.pipes,
      send: (message) => this.sendToBackend(message),
    })
    let outcome: 'closed' | 'online' | 'rejected' = 'closed'
    const inputQueues = new Map<string, SerialQueue>()
    try {
      const deviceToken = await this.token()
      await link.send(
        this.wire.encode({
          type: 'hello',
          protocol: RUNNER_PROTOCOL_VERSION,
          ...(deviceToken ? { deviceToken } : {}),
          runner: {
            name: this.options.name ?? identity.hostname,
            platform: `txiki.js/${runtimeVersion}`,
            version: RUNNER_VERSION,
            identity: { ...this.host.identity },
            ...(this.options.managed ? { managed: true } : {}),
          },
        }),
      )
      for (;;) {
        const frame = await link.receive()
        if (frame === null)
          break
        let message: BackendToRunnerMessage
        try {
          message = this.wire.decodeBackendToRunner(frame)
        } catch (error) {
          this.log(`malformed frame from the backend: ${errorMessage(error)}`)
          continue
        }
        if (message.type === 'job_stdin' || message.type === 'job_stdin_end'
          || message.type === 'spawn_stdin'
          || message.type === 'spawn_stdin_end') {
          this.enqueueInput(message, inputQueues, { rpc, jobs })
          continue
        }
        const handled = await this.handle(message, { rpc, jobs })
        if (handled === 'rejected') {
          outcome = 'rejected'
          break
        }
        if (handled === 'online')
          outcome = 'online'
      }
    } catch (error) {
      this.log(`connection lost: ${errorMessage(error)}`)
    } finally {
      this.link = null
      await link.close().catch(() => {})
      await Promise.all([jobs.close(), rpc.close()])
      this.relay?.connectionLost()
      this.contexts.clear()
    }
    return outcome
  }

  private enqueueInput(
    message: Extract<BackendToRunnerMessage, { type: 'job_stdin'
      | 'job_stdin_end'
      | 'spawn_stdin'
      | 'spawn_stdin_end' }>,
    queues: Map<string, SerialQueue>,
    ends: {
      rpc: HostRpcServer;
      jobs: JobTable
    },
  ): void {
    const key = 'jobId' in message
      ? `job:${message.jobId}`
      : `spawn:${message.spawnId}`
    let queue = queues.get(key)
    if (!queue) {
      queue = new SerialQueue()
      queues.set(key, queue)
    }
    const current = queue
    void current.run(() => this.handle(message, ends))
      // A child can close stdin before queued writes finish; its exit is reported separately.
      .catch(noop)
      .finally(() => {
        if (current.idle && queues.get(key) === current)
          queues.delete(key)
      })
  }

  private async handle(
    message: BackendToRunnerMessage,
    ends: {
      rpc: HostRpcServer;
      jobs: JobTable
    }
  ): Promise<'online' | 'rejected' | undefined> {
    if (this.draining) {
      if (message.type === 'job_start') {
        this.sendToBackend({
          type: 'job_exit',
          jobId: message.jobId,
          exitCode: null,
          spawnError: { kind: 'other' },
          signal: 'runner is draining for upgrade',
        })
        return
      }
      if (message.type === 'spawn') {
        this.sendToBackend({
          type: 'spawn_exit',
          spawnId: message.spawnId,
          exitCode: null,
          spawnError: { kind: 'other' },
          signal: 'runner is draining for upgrade',
        })
        return
      }
    }
    switch (message.type) {
      case 'hello_ok': {
        const config = (await this.state.readConfig()) ?? {
          backendUrl: this.options.backendUrl
        }
        await this.state.writeConfig({
          ...config,
          backendUrl: this.options.backendUrl,
          deviceId: message.deviceId
        })
        this.log('runner online')
        return 'online'
      }
      case 'claim_pending':
        this.log(`Pairing code: ${message.claimToken}`)
        this.log('Enter it in the web UI to link this device.')
        return undefined
      case 'claimed':
        await this.state.writeToken(message.deviceToken)
        this.log('runner online')
        return 'online'
      case 'hello_error':
        this.log(`refused by the backend (${message.code}): ${message.reason}`)
        return message.code === 'already_connected' ? undefined : 'rejected'
      case 'ping':
        this.sendToBackend({ type: 'pong', jobs: ends.jobs.count })
        return undefined
      case 'sync': {
        let syncError: string | undefined
        try {
          await Promise.all(Object.values(this.volumes)
            .map(image => image.sync()))
        } catch (error) {
          syncError = errorMessage(error)
          this.log(`sync failed: ${syncError}`)
        }
        this.sendToBackend({
          type: 'sync_done',
          id: message.id,
          ...(syncError ? { error: syncError } : {})
        })
        return undefined
      }
      case 'volume_grown':
        if (this.growthPending.get(message.volume) !== message.id)
          return undefined
        try {
          if (message.error)
            throw new Error(message.error)
          await this.volumes[message.volume]?.grown(message.bytes)
        } catch (error) {
          this.log(`${message.volume} growth failed: ${errorMessage(error)}`)
        } finally {
          this.growthPending.delete(message.volume)
        }
        return undefined
      case 'manifest':
        try {
          const manifest = await this.cache.install(message.manifest)
          this.log(
            `manifest ${manifest.hash.slice(0, 12)} installed: ${Object.keys(manifest.roots).join(', ')}`
          )
        } catch (error) {
          this.log(`manifest refused: ${errorMessage(error)}`)
        }
        return undefined
      case 'rpc_pipes':
      case 'rpc_output':
      case 'rpc_exit':
        this.relay?.handleReply(message)
        return undefined
      case 'job_start':
      case 'job_stdin':
      case 'job_stdin_end':
      case 'job_kill':
        if (message.type === 'job_kill')
          this.relay?.cancelJob(message.jobId)
        await ends.jobs.handleMessage(message)
        return undefined
      default:
        await ends.rpc.handleMessage(message)
        return undefined
    }
  }

}

/** Reported in `hello`; bumped with the runner program. */
export const RUNNER_VERSION = '0.22.0'

/**
 * `--backend https://demi.example.com` ⇒ `wss://demi.example.com/api/runner`;
 * an explicit path is kept as-is.
 */
export function runnerSocketUrl(backendUrl: string): string {
  const url = new URL(backendUrl)
  if (url.protocol === 'http:') url.protocol = 'ws:'
  else if (url.protocol === 'https:')
    url.protocol = 'wss:'
  if (url.pathname === '' || url.pathname === '/')
    url.pathname = '/api/runner'
  return url.toString()
}
