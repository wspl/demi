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
  idleMs: 10 * 60_000,
  hardCapMs: 24 * 60 * 60_000,
  checkpointIntervalMs: 15 * 60_000,
  crashLoop: { deaths: 3, windowMs: 10 * 60_000 },
  bootTimeoutMs: 60_000,
  sweepMs: 30_000,
  syncTimeoutMs: 5_000,
  systemQuotaBytes: 16 * 1024 ** 3,
  homeQuotaBytes: 32 * 1024 ** 3,
  maxRunning: 16,
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
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message)
    this.name = 'ManagedHostError'
  }
}
interface Machine {
  device: DeviceRecord
  activity: ActivityGate
  state: 'off' | 'booting' | 'running' | 'saving' | 'resetting'
  // Boots and saves can be joined; reset waits for either before taking ownership.
  transitionTask: Promise<void> | null
  // Reset closes admission across several VM transitions, including their failures.
  resetTask: Promise<void> | null
  // The latest durable reset result remains visible after its task has ended.
  resetOperation: ManagedOperation | null
  deaths: number[]
  startedAt: number
  idleSince: number | null
  checkpointAt: number
  error: string | null
}

/** User-owned machines. Admission and every VM transition share one device identity. */
export class ManagedHosts {
  private readonly config: ManagedHostsConfig
  private readonly machines = new Map<string, Machine>()
  private readonly now: () => number
  private readonly log: (line: string) => void
  private readonly timer: ReturnType<typeof setInterval>
  private sweeping = false
  private closed = false

  constructor(private readonly options: ManagedHostsOptions) {
    this.config = { ...DEFAULT_MANAGED_HOSTS_CONFIG, ...options.config }
    this.now = options.now ?? Date.now
    this.log = options.log ?? console.warn
    this.timer = setInterval(() => {
      void this.sweep().catch(error => this.log(errorMessage(error)))
    }, this.config.sweepMs)
    options.provisioner.onDeath(id => {
      const machine = this.machines.get(id)
      if (!machine || machine.state === 'saving' || machine.state === 'resetting') {
        return
      }
      machine.deaths.push(this.now())
      machine.state = 'off'
      this.options.registry.disconnect(id)
    })
  }

  private machine(device: DeviceRecord): Machine {
    if (device.kind !== 'managed') {
      throw new Error('Expected a managed device')
    }
    let machine = this.machines.get(device.id)
    if (!machine) {
      machine = {
        device,
        activity: new ActivityGate(),
        state: 'off',
        transitionTask: null,
        resetTask: null,
        resetOperation: null,
        deaths: [],
        startedAt: 0,
        idleSince: null,
        checkpointAt: 0,
        error: null,
      }
      this.machines.set(device.id, machine)
    }
    return machine
  }

  async ensureRunning(device: DeviceRecord): Promise<void> {
    const machine = this.machine(device)
    if (this.closed) {
      throw new ManagedHostError('closed', 'Cloud is shutting down')
    }
    if (machine.state === 'resetting') {
      throw new ManagedHostError('resetting', 'Cloud environment is resetting')
    }
    while (machine.transitionTask) {
      await machine.transitionTask
    }
    if (machine.resetTask) {
      throw new ManagedHostError('resetting', 'Cloud environment is resetting')
    }
    if (machine.state === 'running') {
      return
    }
    const recentDeaths = machine.deaths.filter(at => this.now() - at < this.config.crashLoop.windowMs)
    if (recentDeaths.length >= this.config.crashLoop.deaths) {
      throw new ManagedHostError('crash_loop', 'Cloud repeatedly failed; reset the environment to recover')
    }

    // Mark booting synchronously before any await: reservations include boots in flight.
    this.assertCapacity(machine)
    machine.state = 'booting'
    const boot = this.startMachine(machine)
    machine.transitionTask = boot
    try {
      await boot
    } finally {
      if (machine.transitionTask === boot) {
        machine.transitionTask = null
      }
    }
  }

  private assertCapacity(machine: Machine): void {
    const reserved = [...this.machines.values()].filter(
      other => other !== machine && other.state !== 'off',
    ).length
    if (reserved >= this.config.maxRunning) {
      throw new ManagedHostError('capacity', 'Cloud capacity is currently full; retry later')
    }
  }

  private async startMachine(machine: Machine): Promise<void> {
    try {
      await this.bootRunner(machine)
      machine.state = 'running'
    } catch (error) {
      machine.state = 'off'
      throw error
    }
  }

  private async bootRunner(machine: Machine): Promise<void> {
    try {
      const token = generateDeviceToken()
      await this.options.control.rotateDeviceToken(machine.device.id, hashDeviceToken(token))
      await this.options.provisioner.wake(machine.device.id, {
        backendUrl: this.options.backendUrl(),
        deviceToken: token,
      })
      await withTimeout(
        this.options.registry.whenOnline(machine.device.id),
        this.config.bootTimeoutMs,
        'Cloud boot timeout',
      )
      machine.error = null
      const startedAt = this.now()
      machine.startedAt = startedAt
      machine.checkpointAt = startedAt
      machine.idleSince = null
    } catch (error) {
      // Preserve the original failure if best-effort disk saving also fails.
      await this.options.provisioner
        .hibernate(machine.device.id)
        .catch(error => this.log(errorMessage(error)))
      this.options.registry.disconnect(machine.device.id)
      machine.error = errorMessage(error)
      throw error
    }
  }

  async enter(device: DeviceRecord, signal?: AbortSignal): Promise<() => void> {
    const machine = this.machine(device)
    if (machine.state === 'resetting') {
      throw new ManagedHostError('resetting', 'Cloud environment is resetting')
    }
    const release = await machine.activity.enter(signal)
    try {
      await this.ensureRunning(device)
      return release
    } catch (error) {
      release()
      throw error
    }
  }

  /** Synchronous admission for every RPC, including calls through an already cached Host. */
  admit(deviceId: string): () => void {
    const machine = this.machines.get(deviceId)
    if (!machine) {
      return noop
    }
    if (this.closed || machine.state !== 'running' || machine.resetTask) {
      throw new ManagedHostError('unavailable', 'Cloud is not accepting operations')
    }
    const release = machine.activity.tryEnter()
    if (!release) {
      throw new ManagedHostError('unavailable', 'Cloud is changing state')
    }
    machine.idleSince = null
    return release
  }

  async hibernate(deviceId: string): Promise<void> {
    const machine = this.machines.get(deviceId)
    if (!machine) {
      return
    }
    if (machine.transitionTask) {
      await machine.transitionTask
    }
    if (machine.state !== 'running') {
      return
    }
    machine.state = 'saving'
    const save = this.saveMachine(machine)
    machine.transitionTask = save
    try {
      await save
    } finally {
      if (machine.transitionTask === save) {
        machine.transitionTask = null
      }
    }
  }

  private async saveMachine(machine: Machine): Promise<void> {
    const deviceId = machine.device.id
    try {
      // Sync is best effort: a disconnected runner must not prevent disk persistence.
      await this.options.registry
        .sync(deviceId, this.config.syncTimeoutMs)
        .catch(error => this.log(errorMessage(error)))
      await this.options.provisioner.hibernate(deviceId)
    } finally {
      this.options.registry.disconnect(deviceId)
      machine.state = 'off'
    }
  }

  async growVolume(deviceId: string, volume: ManagedVolume, bytes: number): Promise<void> {
    const quota = volume === 'system' ? this.config.systemQuotaBytes : this.config.homeQuotaBytes
    if (!Number.isSafeInteger(bytes) || bytes <= 0 || bytes > quota) {
      throw new ManagedHostError('quota', `${volume} volume quota exceeded`)
    }
    const device = await this.options.control.getDevice(deviceId)
    if (!device || device.kind !== 'managed') {
      throw new Error('No managed device')
    }
    await this.options.provisioner.growVolume(deviceId, volume, bytes)
  }

  async status(userId: string) {
    const limits = {
      systemBytes: this.config.systemQuotaBytes,
      homeBytes: this.config.homeQuotaBytes,
    }
    const device = await this.options.control.getManagedDevice(userId)
    if (!device) {
      return {
        device: null,
        state: 'unallocated' as const,
        operation: null,
        error: null,
        image: null,
        limits,
      }
    }
    const machine = this.machine(device)
    return {
      device,
      state: machine.state,
      operation: machine.resetOperation,
      error: machine.error,
      image: await this.options.provisioner.imageState(device.id),
      limits,
    }
  }

  async reset(userId: string, operationId: string): Promise<ManagedOperation> {
    if (this.closed) {
      throw new ManagedHostError('closed', 'Cloud is shutting down')
    }
    const device = await this.options.control.getOrCreateCloudDevice(userId)
    const machine = this.machine(device)
    const existing = await this.options.control.getManagedOperation(device.id, operationId)
    if (machine.state === 'resetting') {
      if (machine.resetOperation?.id === operationId) {
        return machine.resetOperation
      }
      throw new ManagedHostError('resetting', 'Another reset is in progress')
    }
    if (existing?.phase === 'ready') {
      return existing
    }
    const operation: ManagedOperation = {
      id: operationId,
      baseVersion: existing?.baseVersion ?? (await this.options.provisioner.currentBaseVersion()),
      phase: 'stopping',
      error: null,
    }
    // Recheck after asynchronous lookups before closing admission.
    if (machine.resetTask) {
      if (machine.resetOperation?.id === operationId) {
        return machine.resetOperation
      }
      throw new ManagedHostError('resetting', 'Another reset is in progress')
    }
    if (this.closed) {
      throw new ManagedHostError('closed', 'Cloud is shutting down')
    }
    this.assertCapacity(machine)
    machine.state = 'resetting'
    machine.resetOperation = operation
    const task = this.persistAndRunReset(machine, operation)
    machine.resetTask = task
    void task
      .catch(error => this.log(errorMessage(error)))
      .finally(() => {
        machine.resetTask = null
      })
    return operation
  }

  private async persistAndRunReset(machine: Machine, operation: ManagedOperation): Promise<void> {
    try {
      await this.options.control.putManagedOperation(machine.device.id, operation)
      await this.runReset(machine, operation)
    } catch (error) {
      machine.state = 'off'
      machine.error = errorMessage(error)
      machine.resetOperation = { ...operation, phase: 'failed', error: machine.error }
      throw error
    }
  }

  private async runReset(machine: Machine, operation: ManagedOperation): Promise<void> {
    let releaseMachine: (() => void) | undefined
    let releaseTrees: (() => void) | undefined
    const recordPhase = async (phase: ManagedOperation['phase'], error: string | null = null) => {
      operation = { ...operation, phase, error }
      machine.resetOperation = operation
      await this.options.control.putManagedOperation(machine.device.id, operation)
    }
    try {
      // A failed boot/save still needs to finish before reset takes over the disks.
      if (machine.transitionTask) {
        await machine.transitionTask.catch(noop)
      }
      machine.state = 'resetting'
      releaseTrees = await this.options.interrupt(machine.device.userId)
      await this.options.registry
        .sync(machine.device.id, this.config.syncTimeoutMs)
        .catch(error => this.log(errorMessage(error)))
      this.options.registry.disconnect(machine.device.id)
      releaseMachine = await machine.activity.reserve(AbortSignal.timeout(30_000))

      await recordPhase('saving')
      await this.options.provisioner.hibernate(machine.device.id)

      await recordPhase('rebuilding')
      await this.options.provisioner.reset(machine.device.id, operation.id, operation.baseVersion)
      await this.options.control.announceCloudReset(machine.device.userId, operation.id)
      machine.deaths = []

      await recordPhase('booting')
      await this.bootRunner(machine)
      await recordPhase('ready')
      machine.state = 'running'
    } catch (error) {
      // Preserve the original failure if best-effort disk saving also fails.
      await this.options.provisioner
        .hibernate(machine.device.id)
        .catch(error => this.log(errorMessage(error)))
      this.options.registry.disconnect(machine.device.id)
      machine.state = 'off'
      machine.error = errorMessage(error)
      await recordPhase('failed', machine.error)
    } finally {
      releaseMachine?.()
      releaseTrees?.()
    }
  }

  async reconcile(): Promise<void> {
    await this.options.provisioner.reconcile()
    for (const { deviceId, operation } of await this.options.control.listManagedOperations()) {
      const device = await this.options.control.getDevice(deviceId)
      if (!device) {
        throw new Error('Reset references a missing device')
      }
      const machine = this.machine(device)
      machine.resetOperation = operation
      // Recovery makes the disk commit determinate before accepting traffic; boot remains lazy.
      if (operation.phase !== 'ready' && operation.phase !== 'failed') {
        await this.options.provisioner.reset(deviceId, operation.id, operation.baseVersion)
        await this.options.control.announceCloudReset(device.userId, operation.id)
        machine.resetOperation = {
          ...operation,
          phase: 'failed',
          error: 'Reset disks recovered; retry to start Cloud',
        }
        await this.options.control.putManagedOperation(deviceId, machine.resetOperation)
      }
    }
  }

  async sweep(): Promise<void> {
    if (this.sweeping || this.closed) {
      return
    }
    this.sweeping = true
    try {
      for (const machine of this.machines.values()) {
        if (machine.state !== 'running' || machine.resetTask) {
          continue
        }
        const now = this.now()
        const turnInFlight = await this.options.turnInFlight(machine.device.userId)
        const runningJobs = this.options.registry.runningJobs(machine.device.id)
        if (turnInFlight || runningJobs || machine.activity.active) {
          machine.idleSince = null
        } else {
          machine.idleSince ??= now
        }
        const idleTimeoutReached = machine.idleSince !== null && now - machine.idleSince >= this.config.idleMs
        const hardCapReached = !turnInFlight && now - machine.startedAt >= this.config.hardCapMs
        if (idleTimeoutReached || hardCapReached) {
          await this.hibernateIdleMachine(machine)
        } else if (now - machine.checkpointAt >= this.config.checkpointIntervalMs) {
          await this.checkpointMachine(machine, now)
        }
      }
    } finally {
      this.sweeping = false
    }
  }

  private async hibernateIdleMachine(machine: Machine): Promise<void> {
    const releaseMachine = machine.activity.tryReserve()
    if (!releaseMachine) {
      return
    }

    try {
      const releaseTrees = await this.options.reserveIdle(machine.device.userId)
      if (!releaseTrees) {
        return
      }

      try {
        await this.hibernate(machine.device.id)
      } finally {
        releaseTrees()
      }
    } finally {
      releaseMachine()
    }
  }

  private async checkpointMachine(machine: Machine, checkpointAt: number): Promise<void> {
    const deviceId = machine.device.id
    this.options.registry.pauseLiveness(deviceId)
    try {
      await this.options.provisioner.checkpoint(deviceId)
      machine.checkpointAt = checkpointAt
    } finally {
      this.options.registry.resumeLiveness(deviceId)
    }
  }

  async close(): Promise<void> {
    this.closed = true
    clearInterval(this.timer)
    // Reset failures are already recorded and logged by the reset task owner.
    for (const machine of this.machines.values()) {
      await machine.resetTask?.catch(noop)
    }
    for (const machine of this.machines.values()) {
      await this.hibernate(machine.device.id).catch(error => this.log(errorMessage(error)))
    }
    await this.options.provisioner.close()
  }
}
