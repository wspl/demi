import { homedir, hostname } from 'node:os'
import process from 'node:process'
import { LocalHost } from '@demicodes/host-local'
import {
  HostRpcServer,
  RUNNER_PROTOCOL_VERSION,
  decodeRunnerMessage,
  encodeRunnerMessage,
  type BackendToRunnerMessage,
  type RunnerToBackendMessage,
} from '@demicodes/runner-protocol'
import type { Host } from '@demicodes/shell'
import { RunnerState } from './state'

export type RunnerStatus = 'connecting' | 'claim_pending' | 'online' | 'rejected' | 'stopped'

export interface RunnerClientOptions {
  backendUrl: string
  /** Machine-local state directory (default `~/.demi`). */
  stateDir?: string
  /** Device display name (default the machine hostname). */
  name?: string
  version?: string
  /** The Host served to the backend (default a `LocalHost` rooted at the home directory). */
  host?: Pick<Host, 'fs' | 'process' | 'identity'>
  /** Injectable for tests. */
  createWebSocket?: (url: string) => WebSocket
  reconnect?: { initialDelayMs?: number; maxDelayMs?: number }
  onStatus?: (status: RunnerStatus, detail?: string) => void
  /** First-start claim: show this token to the user (the CLI prints it). */
  onClaimPending?: (claimToken: string) => void
}

/**
 * The runner's single outbound connection: authenticates (or waits for the
 * claim), answers Host RPC, responds to liveness pings, and reconnects with
 * exponential backoff. Runner presence equals the state of this socket.
 */
export class RunnerClient {
  private readonly state: RunnerState
  private readonly host: Pick<Host, 'fs' | 'process' | 'identity'>
  private readonly createWebSocket: (url: string) => WebSocket
  private ws: WebSocket | null = null
  private rpc: HostRpcServer | null = null
  private stopped = false
  private rejected = false
  private reconnectDelayMs: number
  private readonly initialDelayMs: number
  private readonly maxDelayMs: number
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null

  constructor(private readonly options: RunnerClientOptions) {
    this.state = new RunnerState(options.stateDir)
    this.host = options.host ?? new LocalHost(homedir())
    this.createWebSocket = options.createWebSocket ?? ((url) => new WebSocket(url))
    this.initialDelayMs = options.reconnect?.initialDelayMs ?? 1_000
    this.maxDelayMs = options.reconnect?.maxDelayMs ?? 30_000
    this.reconnectDelayMs = this.initialDelayMs
  }

  start(): void {
    this.stopped = false
    void this.connect()
  }

  async stop(): Promise<void> {
    this.stopped = true
    if (this.reconnectTimer !== null) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    const ws = this.ws
    this.ws = null
    ws?.close()
    await this.rpc?.close()
    this.rpc = null
    this.options.onStatus?.('stopped')
  }

  private async connect(): Promise<void> {
    if (this.stopped || this.rejected) return
    this.options.onStatus?.('connecting')
    const deviceToken = await this.state.readToken()
    let ws: WebSocket
    try {
      ws = this.createWebSocket(this.options.backendUrl)
    } catch (error) {
      this.scheduleReconnect()
      return
    }
    this.ws = ws
    const rpc = new HostRpcServer(this.host, (message) => this.send(ws, message))
    this.rpc = rpc

    ws.addEventListener('open', () => {
      this.send(ws, {
        type: 'hello',
        protocol: RUNNER_PROTOCOL_VERSION,
        ...(deviceToken ? { deviceToken } : {}),
        runner: {
          name: this.options.name ?? hostname(),
          platform: process.platform,
          version: this.options.version ?? '0.0.0',
          identity: this.host.identity,
        },
      })
    })
    ws.addEventListener('message', (event) => {
      void this.handleFrame(ws, rpc, typeof event.data === 'string' ? event.data : '')
    })
    ws.addEventListener('close', () => {
      if (this.ws !== ws) return
      this.ws = null
      this.rpc = null
      void rpc.close()
      this.scheduleReconnect()
    })
    ws.addEventListener('error', () => {
      // The close event follows and owns reconnection.
    })
  }

  private async handleFrame(ws: WebSocket, rpc: HostRpcServer, frame: string): Promise<void> {
    let message: BackendToRunnerMessage
    try {
      message = decodeRunnerMessage(frame) as BackendToRunnerMessage
    } catch {
      return
    }
    switch (message.type) {
      case 'hello_ok': {
        this.reconnectDelayMs = this.initialDelayMs
        const config = (await this.state.readConfig()) ?? { backendUrl: this.options.backendUrl }
        await this.state.writeConfig({ ...config, backendUrl: this.options.backendUrl, deviceId: message.deviceId })
        this.options.onStatus?.('online')
        return
      }
      case 'claim_pending':
        this.options.onStatus?.('claim_pending')
        this.options.onClaimPending?.(message.claimToken)
        return
      case 'claimed':
        await this.state.writeToken(message.deviceToken)
        this.reconnectDelayMs = this.initialDelayMs
        this.options.onStatus?.('online')
        return
      case 'hello_error':
        // Revoked device or bad token: retrying cannot help without operator action.
        this.rejected = true
        this.options.onStatus?.('rejected', message.reason)
        ws.close()
        return
      case 'ping':
        this.send(ws, { type: 'pong' })
        return
      default:
        await rpc.handleMessage(message)
    }
  }

  private send(ws: WebSocket, message: RunnerToBackendMessage): void {
    try {
      ws.send(encodeRunnerMessage(message))
    } catch {
      // A racing close drops the frame; the close handler owns recovery.
    }
  }

  private scheduleReconnect(): void {
    if (this.stopped || this.rejected || this.reconnectTimer !== null) return
    const delay = this.reconnectDelayMs
    this.reconnectDelayMs = Math.min(this.reconnectDelayMs * 2, this.maxDelayMs)
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      void this.connect()
    }, delay)
  }
}
