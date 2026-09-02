import { RemoteHost } from '@demicodes/host-remote'
import {
  RUNNER_PROTOCOL_VERSION,
  createRunnerWire,
  type BackendToRunnerMessage,
  type HelloErrorCode,
  type RpcCallMessage,
  type RunnerInfo,
  type RunnerProtocolMessage,
  type RunnerToBackendMessage,
} from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import type { HostFileSystem, HostIdentity, HostStore } from '@demicodes/shell'
import { ByteQueue, deferred, errorMessage, type Deferred } from '@demicodes/utils'
import type { ControlService, DeviceRecord, WorkspaceRecord } from '../storage/control'
import { generateClaimCode, generateDeviceToken, hashDeviceToken, normalizeClaimCode } from './claim-codes'
import type { TransferBroker } from './transfers'

export interface RunnerRegistryOptions {
  control: ControlService
  /** Pairing-code lifetime; an expired code is re-issued over the waiting socket. */
  claimTtlMs?: number
  /** Backend-driven liveness interval (0 disables — for tests). */
  pingIntervalMs?: number
  /** Claim attempts allowed per user per minute. */
  claimAttemptsPerMinute?: number
  /** Where connection-level refusals are logged (default `console.warn`). */
  log?: (line: string) => void
  /** The command manifest pushed to every runner on connect (`commands.md`). */
  manifest?: () => Promise<unknown>
  /** Runs an `rpc` command a runner relayed; output streams back through `io`, the result is its exit. */
  rpc?: RpcRelayHandler
  /** Brokered transfers; the registry fails a device's transfers when its connection drops. */
  transfers?: TransferBroker
  /** Every message on every authenticated socket, by device — the wire audit tests run. */
  trace?: (deviceId: string, direction: 'in' | 'out', message: RunnerProtocolMessage) => void
}

/**
 * Where a relayed rpc call's stdout can come from besides `rpc_output`
 * frames: a brokered transfer the calling device `GET`s itself, so bulk
 * bytes never cross the runner socket (`runner.md` § Transfers).
 */
export interface RpcTransferDestination {
  deviceId: string
  /** Tells the runner to stream the transfer at `url` as this call's stdout. */
  receive(url: string): void
}

export interface RpcRelayIO {
  stdinStream: AsyncIterable<Uint8Array>
  stdout(bytes: Uint8Array): Promise<void>
  stderr(bytes: Uint8Array): Promise<void>
  transferDestination: RpcTransferDestination
}

/** One relayed `rpc` invocation: the call as it arrived, its live stdin, and where its output goes. */
export type RpcRelayHandler = (call: Omit<RpcCallMessage, 'type'>, io: RpcRelayIO) => Promise<number>

export interface RunnerSocketHandle {
  /** One MessagePack frame from the runner. */
  handleMessage(frame: Uint8Array): void
  handleClose(): void
}

export type ClaimResult = { ok: true; device: DeviceRecord } | { ok: false; code: 'rate_limited' | 'invalid_code' }

interface RunnerConnection {
  send(message: BackendToRunnerMessage): void
  close(): void
  runner: RunnerInfo | null
  /** Set once the connection is authenticated (claimed or token hello). */
  deviceId: string | null
  /** Set while the connection waits unclaimed. */
  claimCode: string | null
  claimTimer: ReturnType<typeof setTimeout> | null
  pingTimer: ReturnType<typeof setInterval> | null
  pongPending: boolean
  /** Running jobs as of the last `pong`; the idle rule reads it. */
  jobs: number
  /** Live stdin queues of the rpc calls relayed on this connection. */
  rpcStdin: Map<string, ByteQueue>
  /** Transfers this device was told to run, settled by `transfer_done`. */
  transfers: Map<string, Deferred<void>>
}

const wire = createRunnerWire(msgpackCodec)

/**
 * The runner-management module's live state: unclaimed sockets keyed by their
 * pending pairing code (in memory only — a restart just reprints), one
 * authenticated socket per device, and the stable per-execution-target
 * `RemoteHost`s that survive reconnects. Device online status IS socket
 * state; `last_seen_at` is display-only.
 */
export class RunnerRegistry {
  private readonly control: ControlService
  private readonly claimTtlMs: number
  private readonly pingIntervalMs: number
  private readonly claimAttemptsPerMinute: number
  private readonly log: (line: string) => void
  private readonly manifest: (() => Promise<unknown>) | null
  private readonly rpc: RpcRelayHandler | null
  private readonly transfers: TransferBroker | null
  private readonly trace: ((deviceId: string, direction: 'in' | 'out', message: RunnerProtocolMessage) => void) | null
  private readonly pendingClaims = new Map<string, RunnerConnection>()
  private readonly connections = new Map<string, RunnerConnection>()
  private readonly hosts = new Map<string, Map<string, RemoteHost>>()
  /** Last-known identity per device, so a Host can exist while its runner is offline. */
  private readonly identities = new Map<string, HostIdentity>()
  private readonly claimAttempts = new Map<string, number[]>()
  private closed = false

  constructor(options: RunnerRegistryOptions) {
    this.control = options.control
    this.claimTtlMs = options.claimTtlMs ?? 10 * 60_000
    this.pingIntervalMs = options.pingIntervalMs ?? 30_000
    this.claimAttemptsPerMinute = options.claimAttemptsPerMinute ?? 10
    this.log = options.log ?? ((line) => console.warn(line))
    this.manifest = options.manifest ?? null
    this.rpc = options.rpc ?? null
    this.transfers = options.transfers ?? null
    this.trace = options.trace ?? null
  }

  /** Binds one runner WebSocket; the route feeds frames and the close event in. */
  openSocket(io: { send(frame: Uint8Array): void; close(): void }): RunnerSocketHandle {
    const connection: RunnerConnection = {
      send: (message) => {
        if (connection.deviceId !== null) this.trace?.(connection.deviceId, 'out', message)
        try {
          io.send(wire.encode(message))
        } catch {
          // A racing close drops the frame; the close event owns cleanup.
        }
      },
      close: () => io.close(),
      runner: null,
      deviceId: null,
      claimCode: null,
      claimTimer: null,
      pingTimer: null,
      pongPending: false,
      jobs: 0,
      rpcStdin: new Map(),
      transfers: new Map(),
    }
    return {
      handleMessage: (frame) => {
        let message: RunnerToBackendMessage
        try {
          message = wire.decodeRunnerToBackend(frame)
        } catch {
          // A malformed frame is a connection-level error, never ignored.
          connection.close()
          return
        }
        void this.handleMessage(connection, message)
      },
      handleClose: () => this.handleSocketClose(connection),
    }
  }

  /** Claims a pending runner for `userId` — the `POST /api/devices/claim` core. */
  async claim(userId: string, rawCode: string): Promise<ClaimResult> {
    if (!this.recordClaimAttempt(userId)) return { ok: false, code: 'rate_limited' }
    const connection = this.pendingClaims.get(normalizeClaimCode(rawCode))
    if (!connection || !connection.runner) return { ok: false, code: 'invalid_code' }

    this.clearPendingClaim(connection)
    const deviceToken = generateDeviceToken()
    const device = await this.control.createDevice({
      userId,
      name: connection.runner.name,
      platform: connection.runner.platform,
      tokenHash: hashDeviceToken(deviceToken),
    })
    this.bindDevice(connection, device.id)
    connection.send({ type: 'claimed', deviceToken })
    await this.pushManifest(connection)
    return { ok: true, device }
  }

  /** Deletes the device row and drops its live connection (the reconnect is refused). */
  async revoke(deviceId: string): Promise<void> {
    await this.control.deleteDevice(deviceId)
    const connection = this.connections.get(deviceId)
    if (connection) {
      connection.send({ type: 'hello_error', code: 'revoked', reason: 'device revoked' })
      connection.close()
    }
  }

  deviceOnline(deviceId: string): boolean {
    return this.connections.has(deviceId)
  }

  /** Jobs running on the device as of its last `pong`; 0 while offline. */
  runningJobs(deviceId: string): number {
    return this.connections.get(deviceId)?.jobs ?? 0
  }

  /**
   * The stable `Host` for one execution target: same (device, conversation,
   * path) ⇒ same object, so per-Host shell state survives reconnects. Offline
   * devices still resolve — operations fail as ordinary tool errors until the
   * runner reattaches.
   */
  hostFor(workspace: Pick<WorkspaceRecord, 'deviceId' | 'path'>, conversationId: string, store: HostStore): RemoteHost {
    const key = `${conversationId}\0${workspace.path}`
    let deviceHosts = this.hosts.get(workspace.deviceId)
    if (!deviceHosts) {
      deviceHosts = new Map()
      this.hosts.set(workspace.deviceId, deviceHosts)
    }
    let host = deviceHosts.get(key)
    if (!host) {
      host = new RemoteHost({
        defaultCwd: workspace.path,
        commandArtifactsDir: `${workspace.path}/.demi-artifacts`,
        identity: this.identities.get(workspace.deviceId) ?? { uid: 0, gid: 0, hostname: 'offline' },
        store,
      })
      deviceHosts.set(key, host)
      const connection = this.connections.get(workspace.deviceId)
      if (connection) host.attach((message) => connection.send(message))
    }
    return host
  }

  /** Tells the device to `PUT` the file at `path` to `url`; resolves when the destination drained it. */
  transferSend(deviceId: string, transferId: string, path: string, url: string): Promise<void> {
    return this.transfer(deviceId, { type: 'transfer_send', transferId, path, url })
  }

  /** Tells the device to `GET` `url` into the file at `path`. */
  transferReceive(deviceId: string, transferId: string, path: string, url: string): Promise<void> {
    return this.transfer(deviceId, { type: 'transfer_receive', transferId, path, url })
  }

  private transfer(deviceId: string, message: Extract<BackendToRunnerMessage, { type: 'transfer_send' | 'transfer_receive' }>): Promise<void> {
    const connection = this.connections.get(deviceId)
    if (!connection) return Promise.reject(new Error(`device ${deviceId} is offline`))
    const done = deferred<void>()
    connection.transfers.set(message.transferId, done)
    connection.send(message)
    return done.promise
  }

  /** The device's filesystem for web-UI directory browse/create — `null` while offline. */
  deviceFs(deviceId: string): HostFileSystem | null {
    const connection = this.connections.get(deviceId)
    if (!connection) return null
    return this.hostFor({ deviceId, path: '/' }, BROWSE_CONVERSATION, INERT_STORE).fs
  }

  async close(): Promise<void> {
    this.closed = true
    for (const connection of [...this.pendingClaims.values(), ...this.connections.values()]) {
      this.teardown(connection)
      connection.close()
    }
    this.pendingClaims.clear()
    this.connections.clear()
    for (const deviceHosts of this.hosts.values()) {
      for (const host of deviceHosts.values()) host.detach('backend shutting down')
    }
    this.hosts.clear()
  }

  private async handleMessage(connection: RunnerConnection, message: RunnerToBackendMessage): Promise<void> {
    if (message.type === 'hello') {
      await this.handleHello(connection, message)
      return
    }
    if (message.type === 'pong') {
      connection.pongPending = false
      connection.jobs = message.jobs
      return
    }
    if (connection.deviceId === null) return
    this.trace?.(connection.deviceId, 'in', message)
    if (message.type === 'rpc_call') {
      void this.relayRpc(connection, connection.deviceId, message)
      return
    }
    if (message.type === 'rpc_stdin') {
      connection.rpcStdin.get(message.callId)?.push(message.bytes)
      return
    }
    if (message.type === 'rpc_stdin_end') {
      connection.rpcStdin.get(message.callId)?.close()
      return
    }
    if (message.type === 'transfer_done') {
      const done = connection.transfers.get(message.transferId)
      connection.transfers.delete(message.transferId)
      if (message.ok) done?.resolve()
      else done?.reject(new Error(message.error ?? 'transfer failed'))
      return
    }
    // fs results, spawn and job streams: each per-target host claims its own ids.
    const deviceHosts = this.hosts.get(connection.deviceId)
    if (!deviceHosts) return
    for (const host of deviceHosts.values()) host.handleMessage(message)
  }

  /** An `rpc` command invoked on the device: run here, its output streamed back to the command-mode process. */
  private async relayRpc(connection: RunnerConnection, deviceId: string, message: RpcCallMessage): Promise<void> {
    const { type: _type, ...call } = message
    const stdin = new ByteQueue()
    connection.rpcStdin.set(call.callId, stdin)
    let exitCode: number
    try {
      if (!this.rpc) throw new Error('this backend serves no rpc commands to runners')
      exitCode = await this.rpc(call, {
        stdinStream: stdin.stream(),
        stdout: async (bytes) => connection.send({ type: 'rpc_output', callId: call.callId, stream: 'stdout', bytes }),
        stderr: async (bytes) => connection.send({ type: 'rpc_output', callId: call.callId, stream: 'stderr', bytes }),
        transferDestination: {
          deviceId,
          receive: (url) => connection.send({ type: 'rpc_transfer', callId: call.callId, url }),
        },
      })
    } catch (error) {
      connection.send({ type: 'rpc_output', callId: call.callId, stream: 'stderr', bytes: new TextEncoder().encode(`${call.root}: ${errorMessage(error)}\n`) })
      exitCode = 1
    } finally {
      connection.rpcStdin.delete(call.callId)
      stdin.close()
    }
    connection.send({ type: 'rpc_exit', callId: call.callId, exitCode })
  }

  private async handleHello(
    connection: RunnerConnection,
    message: Extract<RunnerToBackendMessage, { type: 'hello' }>,
  ): Promise<void> {
    if (connection.deviceId !== null || connection.claimCode !== null) return
    connection.runner = message.runner
    const refuse = (code: HelloErrorCode, reason: string) => {
      this.log(`runner hello refused (${code}): ${reason} [${message.runner.name}, ${message.runner.platform}]`)
      connection.send({ type: 'hello_error', code, reason })
      connection.close()
    }
    if (message.protocol !== RUNNER_PROTOCOL_VERSION) {
      refuse('unsupported_protocol', `unsupported protocol ${message.protocol}; this backend speaks ${RUNNER_PROTOCOL_VERSION}`)
      return
    }
    if (message.deviceToken === undefined) {
      this.issueClaimCode(connection)
      return
    }
    let device: DeviceRecord | null
    try {
      device = await this.control.getDeviceByTokenHash(hashDeviceToken(message.deviceToken))
    } catch (error) {
      refuse('internal', errorMessage(error))
      return
    }
    if (!device) {
      refuse('unknown_device', 'unknown device')
      return
    }
    // A token holds at most one live connection; the newcomer is refused
    // and retries once the old socket has closed or timed out.
    if (this.connections.has(device.id)) {
      refuse('already_connected', `device ${device.id} already has a live connection`)
      return
    }
    this.bindDevice(connection, device.id)
    connection.send({ type: 'hello_ok', deviceId: device.id })
    await this.pushManifest(connection)
  }

  private async pushManifest(connection: RunnerConnection): Promise<void> {
    if (!this.manifest) return
    try {
      connection.send({ type: 'manifest', manifest: await this.manifest() })
    } catch (error) {
      this.log(`manifest not sent: ${errorMessage(error)}`)
    }
  }

  private issueClaimCode(connection: RunnerConnection): void {
    if (this.closed) return
    if (connection.claimCode) this.pendingClaims.delete(connection.claimCode)
    const code = generateClaimCode()
    connection.claimCode = normalizeClaimCode(code)
    this.pendingClaims.set(connection.claimCode, connection)
    connection.send({ type: 'claim_pending', claimToken: code })
    // Single-use and expiring: an expired code rotates on the waiting socket.
    if (connection.claimTimer) clearTimeout(connection.claimTimer)
    connection.claimTimer = setTimeout(() => this.issueClaimCode(connection), this.claimTtlMs)
  }

  private bindDevice(connection: RunnerConnection, deviceId: string): void {
    connection.deviceId = deviceId
    this.connections.set(deviceId, connection)
    if (connection.runner) this.identities.set(deviceId, connection.runner.identity)
    void this.control.touchDeviceSeen(deviceId).catch(() => {})
    const deviceHosts = this.hosts.get(deviceId)
    if (deviceHosts) {
      for (const host of deviceHosts.values()) host.attach((message) => connection.send(message))
    }
    if (this.pingIntervalMs > 0 && connection.pingTimer === null) {
      connection.pingTimer = setInterval(() => {
        if (connection.pongPending) {
          connection.close()
          return
        }
        connection.pongPending = true
        connection.send({ type: 'ping' })
      }, this.pingIntervalMs)
    }
  }

  private handleSocketClose(connection: RunnerConnection): void {
    this.teardown(connection)
    if (connection.deviceId !== null && this.connections.get(connection.deviceId) === connection) {
      this.connections.delete(connection.deviceId)
      void this.control.touchDeviceSeen(connection.deviceId).catch(() => {})
      this.detachHosts(connection.deviceId, 'runner disconnected')
      this.transfers?.deviceGone(connection.deviceId)
    }
  }

  private detachHosts(deviceId: string, reason: string): void {
    const deviceHosts = this.hosts.get(deviceId)
    if (!deviceHosts) return
    for (const host of deviceHosts.values()) host.detach(reason)
  }

  private teardown(connection: RunnerConnection): void {
    this.clearPendingClaim(connection)
    for (const done of connection.transfers.values()) done.reject(new Error('runner disconnected'))
    connection.transfers.clear()
    if (connection.pingTimer) {
      clearInterval(connection.pingTimer)
      connection.pingTimer = null
    }
  }

  private clearPendingClaim(connection: RunnerConnection): void {
    if (connection.claimCode) {
      this.pendingClaims.delete(connection.claimCode)
      connection.claimCode = null
    }
    if (connection.claimTimer) {
      clearTimeout(connection.claimTimer)
      connection.claimTimer = null
    }
  }

  private recordClaimAttempt(userId: string): boolean {
    const now = Date.now()
    const attempts = (this.claimAttempts.get(userId) ?? []).filter((at) => now - at < 60_000)
    if (attempts.length >= this.claimAttemptsPerMinute) return false
    attempts.push(now)
    this.claimAttempts.set(userId, attempts)
    return true
  }
}

/** Directory browse is fs-only; the shared browse host needs no real identity or store. */
const BROWSE_CONVERSATION = '\0browse'
const INERT_STORE: HostStore = {
  readJson: async () => null,
  writeJson: async () => {},
  delete: async () => {},
  list: async () => [],
}
