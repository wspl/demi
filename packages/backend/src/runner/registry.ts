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
import type { CommandIO, Host, HostFileSystem, HostIdentity, HostStore } from '@demicodes/shell'
import { ByteQueue, createId, deferred, errorMessage, noop, toBytes, type Deferred } from '@demicodes/utils'
import type { ControlService, DeviceRecord, WorkspaceRecord } from '../storage/control'
import { generateClaimCode, generateDeviceToken, hashDeviceToken, normalizeClaimCode } from './claim-codes'
import { withRelayedPipes, type Pipe, type PipeBroker } from './pipes'

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
  /** The pipe broker; the registry mints a relayed call's pipes and fails a device's pipes when its connection drops. */
  pipes?: PipeBroker
  /** Every message on every authenticated socket, by device — the wire audit tests run. */
  trace?: (deviceId: string, direction: 'in' | 'out', message: RunnerProtocolMessage) => void
  /** A guest's `home_grow`: grow the device's home image to `bytes`; `home_grown` is sent once this resolves. */
  homeGrow?: (deviceId: string, bytes: number) => Promise<void>
}

/**
 * A relayed rpc call's streams (`runner.md` § Pipes): its stdin and stdout
 * are pipes whose device end is the caller — the handler reads and writes
 * them here, or names their far ends — and its stderr view rides the
 * socket; the live stdin is what the command is steered with.
 */
export interface RpcRelayIO {
  signal: AbortSignal
  stdin: Pipe | null
  stdout: Pipe
  stderr(bytes: Uint8Array): Promise<void>
  stdinStream: AsyncIterable<Uint8Array>
  /** The `CommandIO` a handler runs with: stdout into the pipe, stderr to the view, the pipes attached for a handler that forwards them. */
  commandIO(): CommandIO
}

/** One relayed `rpc` invocation: the call as it arrived, its live stdin, and where its output goes. */
export interface RpcExecution {
  conversationId: string
  host: Host
  env: Readonly<Record<string, string>>
}

export type RpcRelayHandler = (call: Omit<RpcCallMessage, 'type'>, io: RpcRelayIO, execution: RpcExecution) => Promise<number>

export interface RunnerSocketHandle {
  /** One MessagePack frame from the runner. */
  handleMessage(frame: Uint8Array): void
  handleClose(): void
}

export type ClaimResult = { ok: true; device: DeviceRecord } | { ok: false; code: 'rate_limited' | 'invalid_code' }

interface RunnerConnection {
  closed: boolean
  helloStarted: boolean
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
  /** A backend-initiated pause (the checkpoint copy): a missed pong is not a death while set. */
  livenessPaused: boolean
  /** Running jobs as of the last `pong`; the idle rule reads it. */
  jobs: number
  /** Live stdin queues of the rpc calls relayed on this connection. */
  rpcCalls: Map<string, { jobId: string; live: ByteQueue; controller: AbortController; pipes: Pipe[] }>
  /** `sync` requests in flight, settled by `sync_done`. */
  syncs: Map<string, Deferred<{ untouched: boolean }>>
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
  private readonly pipes: PipeBroker | null
  private readonly trace: ((deviceId: string, direction: 'in' | 'out', message: RunnerProtocolMessage) => void) | null
  private readonly homeGrow: ((deviceId: string, bytes: number) => Promise<void>) | null
  private readonly pendingClaims = new Map<string, RunnerConnection>()
  private readonly sockets = new Set<RunnerConnection>()
  private readonly connections = new Map<string, RunnerConnection>()
  private readonly hosts = new Map<string, Map<string, RemoteHost>>()
  private readonly conversationOfHost = new WeakMap<RemoteHost, string>()
  /** Last-known identity per device, so a Host can exist while its runner is offline. */
  private readonly identities = new Map<string, HostIdentity>()
  private readonly claimAttempts = new Map<string, number[]>()
  private readonly onlineWaiters = new Map<string, Deferred<void>>()
  private closed = false

  constructor(options: RunnerRegistryOptions) {
    this.control = options.control
    this.claimTtlMs = options.claimTtlMs ?? 10 * 60_000
    this.pingIntervalMs = options.pingIntervalMs ?? 30_000
    this.claimAttemptsPerMinute = options.claimAttemptsPerMinute ?? 10
    this.log = options.log ?? ((line) => console.warn(line))
    this.manifest = options.manifest ?? null
    this.rpc = options.rpc ?? null
    this.pipes = options.pipes ?? null
    this.trace = options.trace ?? null
    this.homeGrow = options.homeGrow ?? null
  }

  /** Binds one runner WebSocket; the route feeds frames and the close event in. */
  openSocket(io: { send(frame: Uint8Array): void; close(): void }): RunnerSocketHandle {
    const connection: RunnerConnection = {
      closed: false,
      helloStarted: false,
      send: (message) => {
        if (connection.closed) return
        if (connection.deviceId !== null) this.trace?.(connection.deviceId, 'out', message)
        try {
          io.send(wire.encode(message))
        } catch {
          // A racing close drops the frame; the close event owns cleanup.
        }
      },
      close: () => {
        this.handleSocketClose(connection)
        io.close()
      },
      runner: null,
      deviceId: null,
      claimCode: null,
      claimTimer: null,
      pingTimer: null,
      pongPending: false,
      livenessPaused: false,
      jobs: 0,
      rpcCalls: new Map(),
      syncs: new Map(),
    }
    this.sockets.add(connection)
    return {
      handleMessage: (frame) => {
        if (connection.closed || this.closed) return
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
    const manifest = await this.manifestFrame()
    if (connection.closed || this.closed) {
      await this.control.deleteDevice(device.id)
      return { ok: false, code: 'invalid_code' }
    }
    this.bindDevice(connection, device.id)
    connection.send({ type: 'claimed', deviceToken })
    if (manifest) connection.send(manifest)
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

  /** Resolves once the device has an authenticated socket; at once if it has one now. */
  whenOnline(deviceId: string): Promise<void> {
    if (this.connections.has(deviceId)) return Promise.resolve()
    let waiter = this.onlineWaiters.get(deviceId)
    if (!waiter) {
      waiter = deferred<void>()
      this.onlineWaiters.set(deviceId, waiter)
    }
    return waiter.promise
  }

  /**
   * Liveness detection exempts a host in a backend-initiated pause
   * (`managed-hosts.md` § Home persistence): the checkpoint copy holds the
   * guest paused for the copy time, and the ping loop must not read that as a death.
   */
  pauseLiveness(deviceId: string): void {
    const connection = this.connections.get(deviceId)
    if (connection) connection.livenessPaused = true
  }

  resumeLiveness(deviceId: string): void {
    const connection = this.connections.get(deviceId)
    if (!connection) return
    connection.livenessPaused = false
    connection.pongPending = false
  }

  /** The device's last-known identity (its home directory among it), from any hello it has sent; null before the first. */
  deviceIdentity(deviceId: string): HostIdentity | null {
    return this.identities.get(deviceId) ?? null
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
        identity: this.identities.get(workspace.deviceId) ?? { uid: 0, gid: 0, hostname: 'offline', homeDir: workspace.path },
        store,
      })
      deviceHosts.set(key, host)
      this.conversationOfHost.set(host, conversationId)
      const connection = this.connections.get(workspace.deviceId)
      if (connection) host.attach((message) => connection.send(message))
    }
    return host
  }

  /**
   * Drops the device's socket now. A killed guest leaves its TCP side
   * half-open — no FIN ever comes — and the liveness ping would take a
   * whole interval to notice; a wake must not find the dead connection
   * still counted as online.
   */
  disconnect(deviceId: string): void {
    const connection = this.connections.get(deviceId)
    if (!connection) return
    this.handleSocketClose(connection)
    connection.close()
  }

  /**
   * Flushes the device's home to disk before its guest is killed
   * (`managed-hosts.md` § Lifecycle) and learns whether the home was
   * touched since boot. Offline, or silent past `timeoutMs`, counts as
   * touched: the save then happens in full, which is always correct.
   */
  async sync(deviceId: string, timeoutMs: number): Promise<{ untouched: boolean }> {
    const connection = this.connections.get(deviceId)
    if (!connection) return { untouched: false }
    const id = createId()
    const done = deferred<{ untouched: boolean }>()
    connection.syncs.set(id, done)
    connection.send({ type: 'sync', id })
    const timer = setTimeout(() => {
      connection.syncs.delete(id)
      done.resolve({ untouched: false })
    }, timeoutMs)
    try {
      return await done.promise
    } finally {
      clearTimeout(timer)
    }
  }

  /** The device's filesystem for web-UI directory browse/create — `null` while offline. */
  deviceFs(deviceId: string): HostFileSystem | null {
    const connection = this.connections.get(deviceId)
    if (!connection) return null
    return this.hostFor({ deviceId, path: '/' }, BROWSE_CONVERSATION, INERT_STORE).fs
  }

  async close(): Promise<void> {
    this.closed = true
    for (const waiter of this.onlineWaiters.values()) waiter.reject(new Error('registry closed'))
    this.onlineWaiters.clear()
    for (const connection of [...this.sockets]) {
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
      connection.rpcCalls.get(message.callId)?.live.push(message.bytes)
      return
    }
    if (message.type === 'rpc_stdin_end') {
      connection.rpcCalls.get(message.callId)?.live.close()
      return
    }
    if (message.type === 'rpc_cancel') {
      this.cancelRpc(connection, message.callId)
      return
    }
    if (message.type === 'pipe_done') {
      // The broker settles a pipe by its HTTP exchange; the runner's report is for the log.
      if (!message.ok) this.log(`pipe ${message.pipeId} on device ${connection.deviceId}: ${message.error ?? 'failed'}`)
      return
    }
    if (message.type === 'sync_done') {
      const done = connection.syncs.get(message.id)
      connection.syncs.delete(message.id)
      done?.resolve({ untouched: message.untouched })
      return
    }
    if (message.type === 'home_grow') {
      void this.growHome(connection, connection.deviceId, message.bytes)
      return
    }
    if (message.type === 'job_exit') {
      for (const [callId, call] of connection.rpcCalls) if (call.jobId === message.jobId) this.cancelRpc(connection, callId)
    }
    // fs results, spawn and job streams: each per-target host claims its own ids.
    const deviceHosts = this.hosts.get(connection.deviceId)
    if (!deviceHosts) return
    for (const host of deviceHosts.values()) host.handleMessage(message)
  }

  /** The guest's home is nearly full: grow its image, then tell the guest the new size so it grows the filesystem. */
  private async growHome(connection: RunnerConnection, deviceId: string, bytes: number): Promise<void> {
    if (!this.homeGrow) {
      this.log(`device ${deviceId} asked for a ${bytes}-byte home; this backend grows none`)
      return
    }
    try {
      await this.homeGrow(deviceId, bytes)
      connection.send({ type: 'home_grown', bytes })
    } catch (error) {
      this.log(`home growth of device ${deviceId} failed: ${errorMessage(error)}`)
    }
  }

  /**
   * An `rpc` command invoked on the device: run here, its pipes minted with
   * the device as the caller's end and named to the runner before anything
   * else, its stderr view streamed back, and its exit sent once the stdout
   * pipe drained (`runner.md` § Pipes).
   */
  private executionFor(deviceId: string, call: Omit<RpcCallMessage, 'type'>): RpcExecution {
    for (const host of this.hosts.get(deviceId)?.values() ?? []) {
      const env = host.jobEnvironment(call.jobId)
      if (!env) continue
      if (env.DEMI_SESSION_ID !== call.agentSessionId || env.DEMI_SHELL_ID !== call.shellId) {
        throw new Error('rpc identity does not match the dispatched job')
      }
      const conversationId = this.conversationOfHost.get(host)!
      return { conversationId, host, env }
    }
    throw new Error('rpc requires a live job dispatched to this device')
  }

  private async relayRpc(connection: RunnerConnection, deviceId: string, message: RpcCallMessage): Promise<void> {
    const { type: _type, ...call } = message
    if (connection.rpcCalls.has(call.callId)) {
      connection.close()
      return
    }
    const live = new ByteQueue()
    const controller = new AbortController()
    const pipes: Pipe[] = []
    connection.rpcCalls.set(call.callId, { jobId: call.jobId, live, controller, pipes })
    const stderr = async (bytes: Uint8Array) => connection.send({ type: 'rpc_output', callId: call.callId, bytes })
    let exitCode: number
    try {
      const execution = this.executionFor(deviceId, call)
      if (!this.rpc) throw new Error('this backend serves no rpc commands to runners')
      if (!this.pipes) throw new Error('this backend brokers no pipes')
      const stdin = call.stdin ? this.pipes.open({ deviceId }) : null
      const stdout = this.pipes.open(undefined, { deviceId })
      pipes.push(stdout, ...(stdin ? [stdin] : []))
      connection.send({ type: 'rpc_pipes', callId: call.callId, ...(stdin ? { stdin: stdin.ref() } : {}), stdout: stdout.ref() })
      const writer = stdout.writer()
      const io: RpcRelayIO = {
        signal: controller.signal,
        stdin,
        stdout,
        stderr,
        stdinStream: live.stream(),
        commandIO: () => withRelayedPipes({ stdout: (data) => writer.write(toBytes(data)), stderr: (data) => stderr(toBytes(data)) }, { stdin, stdout }),
      }
      try {
        exitCode = await this.rpc(call, io, execution)
        writer.end()
      } catch (error) {
        writer.fail(error)
        throw error
      }
      // The process has read everything before it exits with the code.
      await stdout.done.catch(noop)
      stdin?.done.catch(noop)
    } catch (error) {
      await stderr(new TextEncoder().encode(`${call.root}: ${errorMessage(error)}\n`))
      exitCode = 1
    } finally {
      connection.rpcCalls.delete(call.callId)
      live.close()
      for (const pipe of pipes) this.pipes?.fail(pipe.id, 'rpc call ended')
    }
    connection.send({ type: 'rpc_exit', callId: call.callId, exitCode: controller.signal.aborted ? 130 : exitCode })
  }

  private cancelRpc(connection: RunnerConnection, callId: string): void {
    const call = connection.rpcCalls.get(callId)
    if (!call) return
    call.controller.abort()
    call.live.close()
    for (const pipe of call.pipes) this.pipes?.fail(pipe.id, 'rpc call cancelled')
  }

  private async handleHello(
    connection: RunnerConnection,
    message: Extract<RunnerToBackendMessage, { type: 'hello' }>,
  ): Promise<void> {
    if (connection.helloStarted || connection.closed || this.closed) return
    connection.helloStarted = true
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
      // A managed host is born with its token; one without it is misbooted,
      // never a device waiting to be paired (`managed-hosts.md` § Joining).
      if (message.runner.managed) refuse('unknown_device', 'a managed host presents its device token; it is never paired')
      else this.issueClaimCode(connection)
      return
    }
    let device: DeviceRecord | null
    try {
      device = await this.control.getDeviceByTokenHash(hashDeviceToken(message.deviceToken))
    } catch (error) {
      refuse('internal', errorMessage(error))
      return
    }
    if (connection.closed || this.closed) return
    if (!device) {
      refuse('unknown_device', 'unknown device')
      return
    }
    // Finish asynchronous preparation before atomically checking and binding
    // the device. A socket closed during preparation cannot become online.
    const manifest = await this.manifestFrame()
    if (connection.closed || this.closed) return
    if (this.connections.has(device.id)) {
      refuse('already_connected', `device ${device.id} already has a live connection`)
      return
    }
    this.bindDevice(connection, device.id)
    connection.send({ type: 'hello_ok', deviceId: device.id })
    if (manifest) connection.send(manifest)
  }

  private async manifestFrame(): Promise<Extract<BackendToRunnerMessage, { type: 'manifest' }> | null> {
    if (!this.manifest) return null
    try {
      return { type: 'manifest', manifest: await this.manifest() }
    } catch (error) {
      this.log(`manifest not sent: ${errorMessage(error)}`)
      return null
    }
  }

  private issueClaimCode(connection: RunnerConnection): void {
    if (this.closed || connection.closed) return
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
    this.onlineWaiters.get(deviceId)?.resolve()
    this.onlineWaiters.delete(deviceId)
    if (connection.runner) this.identities.set(deviceId, connection.runner.identity)
    void this.control.touchDeviceSeen(deviceId).catch(() => {})
    const deviceHosts = this.hosts.get(deviceId)
    if (deviceHosts) {
      for (const host of deviceHosts.values()) host.attach((message) => connection.send(message), connection.runner?.identity)
    }
    if (this.pingIntervalMs > 0 && connection.pingTimer === null) {
      connection.pingTimer = setInterval(() => {
        if (connection.livenessPaused) return
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
    if (connection.closed) return
    connection.closed = true
    this.sockets.delete(connection)
    this.teardown(connection)
    if (connection.deviceId !== null && this.connections.get(connection.deviceId) === connection) {
      this.connections.delete(connection.deviceId)
      void this.control.touchDeviceSeen(connection.deviceId).catch(() => {})
      this.detachHosts(connection.deviceId, 'runner disconnected')
      this.pipes?.deviceGone(connection.deviceId)
    }
  }

  private detachHosts(deviceId: string, reason: string): void {
    const deviceHosts = this.hosts.get(deviceId)
    if (!deviceHosts) return
    for (const host of deviceHosts.values()) host.detach(reason)
  }

  private teardown(connection: RunnerConnection): void {
    this.clearPendingClaim(connection)
    for (const callId of connection.rpcCalls.keys()) this.cancelRpc(connection, callId)
    for (const done of connection.syncs.values()) done.resolve({ untouched: false })
    connection.syncs.clear()
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
