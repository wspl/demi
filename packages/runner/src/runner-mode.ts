// Runner mode (`runner.md`): the one outbound WebSocket to the backend with
// its handshake and reconnect, the Host served over it, the job table, the
// manifest cache, and the local relay for command-mode processes.
import {
  connectWebSocket,
  createRunnerHost,
  identity,
  msgpackDecode,
  msgpackEncode,
  readTail,
  spawnTeed,
  version as tinyjsVersion,
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
import { delay, errorMessage } from '@demicodes/utils'
import { ManifestCache } from './manifest-cache'
import { RelayServer } from './relay/server'
import { RunnerState } from './state'
import { TransferClient } from './transfers'

export interface RunnerModeOptions {
  backendUrl: string
  /** Machine-local state directory (`~/.demi`). */
  stateDir: string
  /** Device display name (default the hostname). */
  name?: string
  /** The packed binary the root-command symlinks point at. */
  executable: string
  /** Device facts jobs fall back to: `PATH`, `HOME`. */
  deviceEnv: Record<string, string>
  /** Booted as a managed host: the hello says so, and a missing token is a refusal, not a pairing. */
  managed?: boolean
  reconnect?: { initialDelayMs?: number; maxDelayMs?: number }
  log?: (line: string) => void
}

export type RunnerStatus = 'connecting' | 'claim_pending' | 'online' | 'rejected' | 'stopped'

/** The runner program: `start` runs until `stop`, reconnecting with backoff. */
export class RunnerMode {
  private readonly host: Host
  private readonly state: RunnerState
  private readonly cache: ManifestCache
  private readonly transfers: TransferClient
  private readonly wire = createRunnerWire({ encode: msgpackEncode, decode: msgpackDecode })
  private readonly log: (line: string) => void
  private stopped = false
  private link: WebSocketLink | null = null
  private relay: RelayServer | null = null

  constructor(private readonly options: RunnerModeOptions) {
    this.host = createRunnerHost({ defaultCwd: identity.homeDir, storeDir: `${options.stateDir}/store` })
    this.state = new RunnerState(this.host.fs, options.stateDir)
    this.cache = new ManifestCache(this.host.fs, this.state.commandsDir, this.state.binDir, options.executable)
    this.transfers = new TransferClient(options.backendUrl, () => this.state.readToken())
    this.log = options.log ?? ((line) => console.error(line))
  }

  /** Runs until stopped; resolves when the runner was told to stop or the backend refused it for good. */
  async run(): Promise<'stopped' | 'rejected'> {
    const initial = this.options.reconnect?.initialDelayMs ?? 1_000
    const max = this.options.reconnect?.maxDelayMs ?? 30_000
    let backoff = initial
    await this.host.fs.mkdir(this.options.stateDir, { recursive: true })
    await this.host.fs.rm(this.state.socketPath, { force: true })
    this.relay = await RelayServer.listen(this.state.socketPath, {
      send: (message) => this.sendToBackend(message),
      manifest: () => this.cache.current(),
      download: (url) => this.transfers.download(url),
    })
    try {
      while (!this.stopped) {
        this.log('connecting…')
        const outcome = await this.connectOnce()
        if (outcome === 'rejected') return 'rejected'
        if (outcome === 'online') backoff = initial
        if (this.stopped) break
        await delay(backoff)
        backoff = Math.min(backoff * 2, max)
      }
      return 'stopped'
    } finally {
      this.relay.close()
      this.relay = null
    }
  }

  async stop(): Promise<void> {
    this.stopped = true
    await this.link?.close()
  }

  private sendToBackend(message: RunnerToBackendMessage): void {
    const link = this.link
    if (!link) throw new Error('runner: not connected')
    void link.send(this.wire.encode(message)).catch(() => {})
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
    this.link = link
    const rpc = new HostRpcServer(this.host, (message) => this.sendToBackend(message), this.options.deviceEnv)
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
      pathPrefix: [this.state.binDir],
      // Command-mode processes find the relay socket and the manifest cache here.
      fixedEnv: { DEMI_HOME: this.options.stateDir },
      send: (message) => this.sendToBackend(message),
    })
    let outcome: 'closed' | 'online' | 'rejected' = 'closed'
    try {
      const deviceToken = await this.state.readToken()
      await link.send(
        this.wire.encode({
          type: 'hello',
          protocol: RUNNER_PROTOCOL_VERSION,
          ...(deviceToken ? { deviceToken } : {}),
          runner: { name: this.options.name ?? identity.hostname, platform: `tinyjs/${tinyjsVersion}`, version: RUNNER_VERSION, identity: { uid: identity.uid, gid: identity.gid, hostname: identity.hostname, homeDir: identity.homeDir }, ...(this.options.managed ? { managed: true } : {}) },
        }),
      )
      for (;;) {
        const frame = await link.receive()
        if (frame === null) break
        let message: BackendToRunnerMessage
        try {
          message = this.wire.decodeBackendToRunner(frame)
        } catch (error) {
          this.log(`malformed frame from the backend: ${errorMessage(error)}`)
          continue
        }
        const handled = await this.handle(message, { rpc, jobs })
        if (handled === 'rejected') {
          outcome = 'rejected'
          break
        }
        if (handled === 'online') outcome = 'online'
      }
    } catch (error) {
      this.log(`connection lost: ${errorMessage(error)}`)
    } finally {
      this.link = null
      await link.close().catch(() => {})
      await Promise.all([jobs.close(), rpc.close()])
      this.relay?.connectionLost()
    }
    return outcome
  }

  private async handle(message: BackendToRunnerMessage, ends: { rpc: HostRpcServer; jobs: JobTable }): Promise<'online' | 'rejected' | undefined> {
    switch (message.type) {
      case 'hello_ok': {
        const config = (await this.state.readConfig()) ?? { backendUrl: this.options.backendUrl }
        await this.state.writeConfig({ ...config, backendUrl: this.options.backendUrl, deviceId: message.deviceId })
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
      case 'manifest':
        try {
          const manifest = await this.cache.install(message.manifest)
          this.log(`manifest ${manifest.hash.slice(0, 12)} installed: ${Object.keys(manifest.roots).join(', ')}`)
        } catch (error) {
          this.log(`manifest refused: ${errorMessage(error)}`)
        }
        return undefined
      case 'rpc_output':
      case 'rpc_transfer':
      case 'rpc_exit':
        this.relay?.handleReply(message)
        return undefined
      case 'transfer_send':
      case 'transfer_receive':
        void this.transfer(message)
        return undefined
      case 'job_start':
      case 'job_stdin':
      case 'job_stdin_end':
      case 'job_kill':
        await ends.jobs.handleMessage(message)
        return undefined
      default:
        await ends.rpc.handleMessage(message)
        return undefined
    }
  }

  /** One brokered copy: the HTTP exchange runs to its end, then `transfer_done` reports it. */
  private async transfer(message: Extract<BackendToRunnerMessage, { type: 'transfer_send' | 'transfer_receive' }>): Promise<void> {
    const { transferId } = message
    try {
      if (message.type === 'transfer_send') await this.transfers.send(message.path, message.url)
      else await this.transfers.receive(message.path, message.url)
      this.sendToBackend({ type: 'transfer_done', transferId, ok: true })
    } catch (error) {
      this.log(`transfer ${transferId} failed: ${errorMessage(error)}`)
      try {
        this.sendToBackend({ type: 'transfer_done', transferId, ok: false, error: errorMessage(error) })
      } catch {
        // Offline: the backend already failed the transfer with the connection.
      }
    }
  }
}

/** Reported in `hello`; bumped with the runner program. */
export const RUNNER_VERSION = '0.20.0'

/** `--backend https://demi.example.com` ⇒ `wss://demi.example.com/api/runner`; an explicit path is kept as-is. */
export function runnerSocketUrl(backendUrl: string): string {
  const url = new URL(backendUrl)
  if (url.protocol === 'http:') url.protocol = 'ws:'
  else if (url.protocol === 'https:') url.protocol = 'wss:'
  if (url.pathname === '' || url.pathname === '/') url.pathname = '/api/runner'
  return url.toString()
}
