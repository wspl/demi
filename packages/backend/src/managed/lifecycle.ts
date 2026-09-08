import { ActivityGate, errorMessage, noop, withTimeout } from '@demicodes/utils'
import { generateDeviceToken, hashDeviceToken } from '../runner/claim-codes'
import type { RunnerRegistry } from '../runner/registry'
import type { ControlService, DeviceRecord, ManagedOperation } from '../storage/control'
import type { ManagedHostProvisioner, ManagedVolume } from './provisioner'

export interface ManagedHostsConfig {
  idleMs: number
  hardCapMs: number
  checkpointIntervalMs: number
  crashLoop: { deaths: number; windowMs: number }
  bootTimeoutMs: number
  sweepMs: number
  syncTimeoutMs: number
  systemQuotaBytes: number
  homeQuotaBytes: number
  maxRunning: number
}
export const DEFAULT_MANAGED_HOSTS_CONFIG: ManagedHostsConfig = {
  idleMs: 10 * 60_000, hardCapMs: 24 * 60 * 60_000,
  checkpointIntervalMs: 15 * 60_000, crashLoop: { deaths: 3, windowMs: 10 * 60_000 },
  bootTimeoutMs: 60_000, sweepMs: 30_000, syncTimeoutMs: 5_000,
  systemQuotaBytes: 16 * 1024 ** 3, homeQuotaBytes: 32 * 1024 ** 3, maxRunning: 16,
}
export interface ManagedHostsOptions {
  control: ControlService
  registry: RunnerRegistry
  provisioner: ManagedHostProvisioner
  backendUrl: () => string
  turnInFlight: (userId: string) => Promise<boolean>
  reserveIdle: (userId: string) => Promise<(() => void) | null>
  interrupt: (userId: string) => Promise<() => void>
  config?: Partial<ManagedHostsConfig>
  log?: (line: string) => void
  now?: () => number
}
export class ManagedHostError extends Error {
  constructor(readonly code: string, message: string) { super(message); this.name = 'ManagedHostError' }
}
interface Machine {
  device: DeviceRecord
  activity: ActivityGate
  state: 'off' | 'booting' | 'running' | 'saving' | 'resetting'
  pending: Promise<void> | null
  resetTask: Promise<void> | null
  reset: ManagedOperation | null
  deaths: number[]
  startedAt: number
  idleSince: number | null
  checkpointAt: number
  error: string | null
}

/** User-owned machines. Admission and every VM transition share one device identity. */
export class ManagedHosts {
  private readonly config: ManagedHostsConfig
  private readonly volumeLimits: { systemBytes: number; homeBytes: number }
  private readonly machines = new Map<string, Machine>()
  private readonly now: () => number
  private readonly log: (line: string) => void
  private readonly timer: ReturnType<typeof setInterval>
  private sweeping = false
  private closed = false

  constructor(private readonly options: ManagedHostsOptions) {
    this.config = { ...DEFAULT_MANAGED_HOSTS_CONFIG, ...options.config }
    this.volumeLimits = { systemBytes: this.config.systemQuotaBytes, homeBytes: this.config.homeQuotaBytes }
    this.now = options.now ?? Date.now
    this.log = options.log ?? console.warn
    this.timer = setInterval(() => { void this.sweep().catch(error => this.log(errorMessage(error))) }, this.config.sweepMs)
    options.provisioner.onDeath(id => {
      const machine = this.machines.get(id)
      if (!machine || machine.state === 'saving' || machine.state === 'resetting') return
      machine.deaths.push(this.now()); machine.state = 'off'
      this.options.registry.disconnect(id)
    })
  }

  private machine(device: DeviceRecord): Machine {
    if (device.kind !== 'managed') throw new Error('Expected a managed device')
    let machine = this.machines.get(device.id)
    if (!machine) {
      machine = { device, activity: new ActivityGate(), state: 'off', pending: null, resetTask: null, reset: null, deaths: [], startedAt: 0, idleSince: null, checkpointAt: 0, error: null }
      this.machines.set(device.id, machine)
    }
    return machine
  }

  async ensureRunning(device: DeviceRecord): Promise<void> {
    const machine = this.machine(device)
    if (this.closed) throw new ManagedHostError('closed', 'Cloud is shutting down')
    if (machine.state === 'resetting') throw new ManagedHostError('resetting', 'Cloud environment is resetting')
    while (machine.pending) await machine.pending
    if (machine.resetTask) throw new ManagedHostError('resetting', 'Cloud environment is resetting')
    if (machine.state === 'running') return
    if (machine.deaths.filter(at => this.now() - at < this.config.crashLoop.windowMs).length >= this.config.crashLoop.deaths) throw new ManagedHostError('crash_loop', 'Cloud repeatedly failed; reset the environment to recover')
    // Mark booting synchronously before any await: reservations include boots in flight.
    this.assertCapacity(machine)
    machine.state = 'booting'
    const boot = this.boot(machine)
    machine.pending = boot
    try { await boot } finally { if (machine.pending === boot) machine.pending = null }
  }

  private assertCapacity(machine: Machine): void {
    const reserved = [...this.machines.values()].filter(other => other !== machine && other.state !== 'off').length
    if (reserved >= this.config.maxRunning) throw new ManagedHostError('capacity', 'Cloud capacity is currently full; retry later')
  }

  private async boot(machine: Machine, resetting = false): Promise<void> {
    try {
      const token = generateDeviceToken()
      await this.options.control.rotateDeviceToken(machine.device.id, hashDeviceToken(token))
      await this.options.provisioner.wake(machine.device.id, { backendUrl: this.options.backendUrl(), deviceToken: token })
      await withTimeout(this.options.registry.whenOnline(machine.device.id), this.config.bootTimeoutMs, 'Cloud boot timeout')
      if (!resetting) machine.state = 'running'
      machine.error = null
      machine.startedAt = machine.checkpointAt = this.now(); machine.idleSince = null
    } catch (error) {
      await this.options.provisioner.hibernate(machine.device.id).catch(noop)
      this.options.registry.disconnect(machine.device.id)
      if (!resetting) machine.state = 'off'
      machine.error = errorMessage(error)
      throw error
    }
  }

  async enter(device: DeviceRecord, signal?: AbortSignal): Promise<() => void> {
    const machine = this.machine(device)
    if (machine.state === 'resetting') throw new ManagedHostError('resetting', 'Cloud environment is resetting')
    const release = await machine.activity.enter(signal)
    try { await this.ensureRunning(device); return release }
    catch (error) { release(); throw error }
  }

  /** Synchronous admission for every RPC, including calls through an already cached Host. */
  admit(deviceId: string): () => void {
    const machine = this.machines.get(deviceId)
    if (!machine) return noop
    if (this.closed || machine.state !== 'running' || machine.resetTask) throw new ManagedHostError('unavailable', 'Cloud is not accepting operations')
    const release = machine.activity.tryEnter()
    if (!release) throw new ManagedHostError('unavailable', 'Cloud is changing state')
    machine.idleSince = null
    return release
  }

  async hibernate(deviceId: string): Promise<void> {
    const machine = this.machines.get(deviceId)
    if (!machine) return
    if (machine.pending) await machine.pending
    if (machine.state !== 'running') return
    machine.state = 'saving'
    const save = (async () => {
      try {
        await this.options.registry.sync(deviceId, this.config.syncTimeoutMs).catch(error => this.log(errorMessage(error)))
        await this.options.provisioner.hibernate(deviceId)
      } finally { this.options.registry.disconnect(deviceId); machine.state = 'off' }
    })()
    machine.pending = save
    try { await save } finally { if (machine.pending === save) machine.pending = null }
  }

  async growVolume(deviceId: string, volume: ManagedVolume, bytes: number): Promise<void> {
    const quota = volume === 'system' ? this.config.systemQuotaBytes : this.config.homeQuotaBytes
    if (!Number.isSafeInteger(bytes) || bytes <= 0 || bytes > quota) throw new ManagedHostError('quota', `${volume} volume quota exceeded`)
    const device = await this.options.control.getDevice(deviceId)
    if (!device || device.kind !== 'managed') throw new Error('No managed device')
    await this.options.provisioner.growVolume(deviceId, volume, bytes)
  }

  async status(userId: string) {
    const device = await this.options.control.getManagedDevice(userId)
    if (!device) return { device: null, state: 'unallocated' as const, operation: null, error: null, image: null, limits: this.volumeLimits }
    const machine = this.machine(device)
    return { device, state: machine.state, operation: machine.reset, error: machine.error, image: await this.options.provisioner.imageState(device.id), limits: this.volumeLimits }
  }

  async reset(userId: string, operationId: string): Promise<ManagedOperation> {
    if (this.closed) throw new ManagedHostError('closed', 'Cloud is shutting down')
    const device = await this.options.control.getOrCreateCloudDevice(userId)
    const machine = this.machine(device)
    const existing = await this.options.control.getManagedOperation(device.id, operationId)
    if (machine.state === 'resetting') {
      if (machine.reset?.id === operationId) return machine.reset
      throw new ManagedHostError('resetting', 'Another reset is in progress')
    }
    if (existing?.phase === 'ready') return existing
    const operation: ManagedOperation = { id: operationId, baseVersion: existing?.baseVersion ?? await this.options.provisioner.currentBaseVersion(), phase: 'stopping', error: null }
    // Recheck after asynchronous lookups before closing admission.
    if (machine.resetTask) {
      if (machine.reset?.id === operationId) return machine.reset
      throw new ManagedHostError('resetting', 'Another reset is in progress')
    }
    if (this.closed) throw new ManagedHostError('closed', 'Cloud is shutting down')
    this.assertCapacity(machine)
    machine.state = 'resetting'; machine.reset = operation
    const task = (async () => {
      try {
        await this.options.control.putManagedOperation(device.id, operation)
        await this.runReset(machine, operation)
      } catch (error) {
        machine.state = 'off'; machine.error = errorMessage(error)
        machine.reset = { ...operation, phase: 'failed', error: machine.error }
        throw error
      }
    })()
    machine.resetTask = task
    void task.catch(error => this.log(errorMessage(error))).finally(() => { machine.resetTask = null })
    return operation
  }

  private async runReset(machine: Machine, operation: ManagedOperation): Promise<void> {
    let release: (() => void) | undefined
    let releaseTrees: (() => void) | undefined
    const phase = async (value: ManagedOperation['phase'], error: string | null = null) => {
      operation = { ...operation, phase: value, error }; machine.reset = operation
      await this.options.control.putManagedOperation(machine.device.id, operation)
    }
    try {
      if (machine.pending) await machine.pending.catch(noop)
      machine.state = 'resetting'
      releaseTrees = await this.options.interrupt(machine.device.userId)
      await this.options.registry.sync(machine.device.id, this.config.syncTimeoutMs).catch(error => this.log(errorMessage(error)))
      this.options.registry.disconnect(machine.device.id)
      release = await machine.activity.reserve(AbortSignal.timeout(30_000))
      await phase('saving')
      await this.options.provisioner.hibernate(machine.device.id)
      await phase('rebuilding')
      await this.options.provisioner.reset(machine.device.id, operation.id, operation.baseVersion)
      await this.options.control.announceCloudReset(machine.device.userId, operation.id)
      machine.deaths = []
      await phase('booting')
      await this.boot(machine, true)
      await phase('ready')
      machine.state = 'running'
    } catch (error) {
      await this.options.provisioner.hibernate(machine.device.id).catch(noop)
      this.options.registry.disconnect(machine.device.id)
      machine.state = 'off'; machine.error = errorMessage(error)
      await phase('failed', machine.error)
    } finally { release?.(); releaseTrees?.() }
  }

  async reconcile(): Promise<void> {
    await this.options.provisioner.reconcile()
    for (const { deviceId, operation } of await this.options.control.listManagedOperations()) {
      const device = await this.options.control.getDevice(deviceId)
      if (!device) throw new Error('Reset references a missing device')
      const machine = this.machine(device); machine.reset = operation
      // Recovery makes the disk commit determinate before accepting traffic; boot remains lazy.
      if (operation.phase !== 'ready' && operation.phase !== 'failed') {
        await this.options.provisioner.reset(deviceId, operation.id, operation.baseVersion)
        await this.options.control.announceCloudReset(device.userId, operation.id)
        machine.reset = { ...operation, phase: 'failed', error: 'Reset disks recovered; retry to start Cloud' }
        await this.options.control.putManagedOperation(deviceId, machine.reset)
      }
    }
  }

  async sweep(): Promise<void> {
    if (this.sweeping || this.closed) return
    this.sweeping = true
    try {
      for (const machine of this.machines.values()) {
        if (machine.state !== 'running' || machine.resetTask) continue
        const now = this.now()
        const turn = await this.options.turnInFlight(machine.device.userId)
        const jobs = this.options.registry.runningJobs(machine.device.id)
        if (turn || jobs || machine.activity.active) machine.idleSince = null
        else machine.idleSince ??= now
        const idle = machine.idleSince !== null && now - machine.idleSince >= this.config.idleMs
        const capped = !turn && now - machine.startedAt >= this.config.hardCapMs
        if (idle || capped) {
          const releaseMachine = machine.activity.tryReserve()
          if (!releaseMachine) continue
          try {
            const releaseTrees = await this.options.reserveIdle(machine.device.userId)
            if (!releaseTrees) continue
            try { await this.hibernate(machine.device.id) } finally { releaseTrees() }
          } finally { releaseMachine() }
        } else if (now - machine.checkpointAt >= this.config.checkpointIntervalMs) {
          this.options.registry.pauseLiveness(machine.device.id)
          try { await this.options.provisioner.checkpoint(machine.device.id); machine.checkpointAt = now }
          finally { this.options.registry.resumeLiveness(machine.device.id) }
        }
      }
    } finally { this.sweeping = false }
  }

  async close(): Promise<void> {
    this.closed = true; clearInterval(this.timer)
    for (const machine of this.machines.values()) await machine.resetTask?.catch(noop)
    for (const machine of this.machines.values()) await this.hibernate(machine.device.id).catch(error => this.log(errorMessage(error)))
    await this.options.provisioner.close()
  }
}
