import { deferred, errorMessage, noop, withTimeout, type Deferred } from '@demicodes/utils'
import { generateDeviceToken, hashDeviceToken } from '../runner/claim-codes'
import type { RunnerRegistry } from '../runner/registry'
import type { ControlService, DeviceRecord, ManagedHostOwner } from '../storage/control'
import { ownerKey, type ManagedHostProvisioner } from './provisioner'

/** Sizes are configurable, presence is not (`managed-hosts.md` § Security baseline). */
export interface ManagedHostsConfig {
  /** Reclaim after this long with no in-flight turn and no running jobs. */
  idleMs: number
  /** Reclaim a host that has jobs but no turns after this long. */
  hardCapMs: number
  /** Save a running host's home this often. */
  checkpointIntervalMs: number
  /** This many guest deaths within the window stop automatic re-provisioning for the owner. */
  crashLoop: { deaths: number; windowMs: number }
  hostsPerUser: number
  /** How long a booted guest may take to present itself. */
  bootTimeoutMs: number
  /** How often the idle rule and the checkpoint clock are evaluated. */
  sweepMs: number
  /** How long the guest may take to answer the `sync` before hibernate; silence counts as a touched home. */
  syncTimeoutMs: number
}

export const DEFAULT_MANAGED_HOSTS_CONFIG: ManagedHostsConfig = {
  idleMs: 10 * 60_000,
  hardCapMs: 24 * 60 * 60_000,
  checkpointIntervalMs: 15 * 60_000,
  crashLoop: { deaths: 3, windowMs: 10 * 60_000 },
  hostsPerUser: 10,
  bootTimeoutMs: 60_000,
  sweepMs: 30_000,
  syncTimeoutMs: 5_000,
}

export interface ManagedHostsOptions {
  control: ControlService
  registry: RunnerRegistry
  provisioner: ManagedHostProvisioner
  /** The URL guests dial — the backend's public one, like a user host (`managed-hosts.md` § Network). */
  backendUrl: () => string
  /** Whether any conversation of the owner has a turn in flight; the idle rule's first input. */
  turnInFlight: (owner: ManagedHostOwner) => Promise<boolean>
  config?: Partial<ManagedHostsConfig>
  log?: (line: string) => void
  now?: () => number
}

export type ManagedHostErrorCode = 'crash_loop' | 'host_limit' | 'boot_timeout' | 'guest_died' | 'not_owner'

export class ManagedHostError extends Error {
  constructor(
    readonly code: ManagedHostErrorCode,
    message: string,
  ) {
    super(message)
    this.name = 'ManagedHostError'
  }
}

/**
 * One owner's guest as the lifecycle sees it (`managed-hosts.md` §
 * Lifecycle). `booting` covers provision and wake; `off` covers hibernated
 * and dead alike — both take the wake path on the next need. State is in
 * memory: after a backend restart every managed device is `off`, which is
 * true, since its guest died with the process.
 */
interface OwnedHost {
  owner: ManagedHostOwner
  deviceId: string
  state: 'booting' | 'running' | 'saving' | 'off'
  /** The in-flight boot, joined by concurrent needs. */
  boot: Promise<void> | null
  /** Rejected by a death during the boot, so the boot fails then rather than at its timeout. */
  died: Deferred<never> | null
  /** The in-flight hibernate; a need arriving meanwhile waits for it and then wakes. */
  save: Promise<void> | null
  startedAt: number
  idleSince: number | null
  lastCheckpointAt: number
  deaths: number[]
}

/** The device row's owner (`devices.owner_*`, managed rows only). */
export function ownerOf(device: DeviceRecord): ManagedHostOwner {
  if (device.ownerConversationId !== null) return { kind: 'conversation', id: device.ownerConversationId }
  if (device.ownerWorkspaceId !== null) return { kind: 'workspace', id: device.ownerWorkspaceId }
  throw new Error(`device ${device.id} is managed but owns nothing`)
}

/**
 * The managed-host lifecycle: provision on first need, the idle rule and
 * hard cap driving hibernation, wake on the next need with concurrent
 * needs joined, the periodic checkpoint with the liveness exemption, the
 * crash-loop guard and the per-user cap. Drives the provisioner seam and
 * the device rows; never touches an image. Every way a guest ends — the
 * idle rule, the owner archived or deleted, the backend closing — goes
 * through `hibernate`, so the home is saved before the guest is gone.
 */
export class ManagedHosts {
  private readonly config: ManagedHostsConfig
  private readonly hosts = new Map<string, OwnedHost>()
  private readonly log: (line: string) => void
  private readonly now: () => number
  private sweepTimer: ReturnType<typeof setInterval> | null = null
  private sweeping = false
  private closed = false

  constructor(private readonly options: ManagedHostsOptions) {
    this.config = { ...DEFAULT_MANAGED_HOSTS_CONFIG, ...options.config }
    this.log = options.log ?? ((line) => console.warn(line))
    this.now = options.now ?? (() => Date.now())
    options.provisioner.onDeath((owner) => this.guestDied(owner))
    this.sweepTimer = setInterval(() => void this.sweep(), this.config.sweepMs)
  }

  /**
   * The first boot for an owner: the device row with its token, the guest
   * over `homeDir`, online before this resolves. Refused past the per-user
   * cap, and for an owner that has a machine already — one machine per
   * owner, never reused.
   */
  async provision(owner: ManagedHostOwner, userId: string, homeDir: string): Promise<DeviceRecord> {
    if (await this.options.control.getManagedDevice(owner)) throw new Error(`${ownerKey(owner)} already has a machine`)
    if ((await this.options.control.countManagedDevices(userId)) >= this.config.hostsPerUser) {
      throw new ManagedHostError('host_limit', `the limit of ${this.config.hostsPerUser} machines per user is reached`)
    }
    const token = generateDeviceToken()
    const device = await this.options.control.createDevice({
      userId,
      kind: 'managed',
      name: 'cloud',
      platform: 'managed',
      tokenHash: hashDeviceToken(token),
      ...(owner.kind === 'conversation' ? { ownerConversationId: owner.id } : { ownerWorkspaceId: owner.id }),
    })
    const host = this.host(owner, device.id)
    await this.boot(host, () => this.options.provisioner.provision(owner, homeDir, { backendUrl: this.options.backendUrl(), deviceToken: token }))
    return device
  }

  /**
   * A first boot that leaves nothing behind when it fails: the device row,
   * the guest and whatever the provisioner made for the owner are removed,
   * so the next attempt starts from the caller's home as it stands then.
   */
  async provisionFresh(owner: ManagedHostOwner, userId: string, homeDir: string): Promise<DeviceRecord> {
    try {
      return await this.provision(owner, userId, homeDir)
    } catch (error) {
      const device = await this.options.control.getManagedDevice(owner)
      if (device) {
        await this.destroy(owner).catch(noop)
        await this.options.control.deleteDevice(device.id).catch(noop)
        this.hosts.delete(ownerKey(owner))
      }
      throw error
    }
  }

  /**
   * The next action needing the host: a running guest returns at once, a
   * boot in flight is joined, an off guest is woken with a fresh token — a
   * latency, not an error — unless it is crash-looping.
   */
  async ensureRunning(device: DeviceRecord): Promise<void> {
    const owner = ownerOf(device)
    const host = this.host(owner, device.id)
    if (host.state === 'running') return
    if (host.boot) return host.boot
    if (host.save) await host.save.catch(() => {})
    if (host.boot) return host.boot
    const recent = host.deaths.filter((at) => this.now() - at < this.config.crashLoop.windowMs)
    if (recent.length >= this.config.crashLoop.deaths) {
      throw new ManagedHostError('crash_loop', `the machine died ${recent.length} times in the last ${Math.round(this.config.crashLoop.windowMs / 60_000)} minutes and is not restarted automatically`)
    }
    const token = generateDeviceToken()
    await this.options.control.rotateDeviceToken(device.id, hashDeviceToken(token))
    await this.boot(host, () => this.options.provisioner.wake(owner, { backendUrl: this.options.backendUrl(), deviceToken: token }))
  }

  /** The guest a device row denotes, if this backend knows it as running. */
  isRunning(deviceId: string): boolean {
    for (const host of this.hosts.values()) if (host.deviceId === deviceId) return host.state === 'running'
    return false
  }

  /** Flushes the home, kills the guest and saves its home; the next need wakes it. */
  async hibernate(owner: ManagedHostOwner): Promise<void> {
    const host = this.hosts.get(ownerKey(owner))
    if (!host || host.state !== 'running') return
    host.state = 'saving'
    host.save = (async () => {
      try {
        const report = await this.options.registry.sync(host.deviceId, this.config.syncTimeoutMs)
        await this.options.provisioner.hibernate(owner, report)
        this.options.registry.disconnect(host.deviceId)
      } finally {
        host.state = 'off'
        host.save = null
      }
    })()
    await host.save
  }

  /** A guest's `home_grow` (`managed-hosts.md` § Home persistence): its image becomes `bytes` large. */
  async growHome(deviceId: string, bytes: number): Promise<void> {
    const device = await this.options.control.getDevice(deviceId)
    if (!device || device.kind !== 'managed') throw new Error(`device ${deviceId} has no managed home`)
    await this.options.provisioner.growHome(ownerOf(device), bytes)
  }

  /**
   * The backend starting: the provisioner kills and saves what a previous
   * process left before any guest is booted here.
   */
  reconcile(): Promise<void> {
    return this.options.provisioner.reconcile()
  }

  /** The owner is archived or deleted: the guest is hibernated — its home saved — and forgotten; the device row stays with the home (retention is a later item). */
  async destroy(owner: ManagedHostOwner): Promise<void> {
    const device = await this.options.control.getManagedDevice(owner)
    if (!device) return
    const host = this.host(owner, device.id)
    if (host.boot) await host.boot.catch(noop)
    await this.hibernate(owner)
    host.state = 'off'
    await this.options.provisioner.destroy(owner)
    this.options.registry.disconnect(device.id)
  }

  /** The backend closing: every running guest hibernated — synced, killed, saved — before the process goes. */
  async close(): Promise<void> {
    this.closed = true
    if (this.sweepTimer) clearInterval(this.sweepTimer)
    this.sweepTimer = null
    for (const host of [...this.hosts.values()]) {
      if (host.boot) await host.boot.catch(noop)
      await this.hibernate(host.owner).catch((error) => this.log(`hibernate of ${ownerKey(host.owner)} at close failed: ${errorMessage(error)}`))
    }
    await this.options.provisioner.close()
  }

  private host(owner: ManagedHostOwner, deviceId: string): OwnedHost {
    const key = ownerKey(owner)
    let host = this.hosts.get(key)
    if (!host) {
      host = { owner, deviceId, state: 'off', boot: null, died: null, save: null, startedAt: 0, idleSince: null, lastCheckpointAt: 0, deaths: [] }
      this.hosts.set(key, host)
    }
    return host
  }

  private async boot(host: OwnedHost, start: () => Promise<void>): Promise<void> {
    host.state = 'booting'
    const died = deferred<never>()
    died.promise.catch(noop)
    host.died = died
    host.boot = (async () => {
      try {
        await start()
        // Online, or dead on the way there — a death fails the boot at once, not at its timeout.
        await Promise.race([withTimeout(this.options.registry.whenOnline(host.deviceId), this.config.bootTimeoutMs, 'boot timeout'), died.promise])
      } catch (error) {
        host.state = 'off'
        throw errorMessage(error) === 'boot timeout' ? new ManagedHostError('boot_timeout', 'the machine did not come online in time') : error
      }
      const now = this.now()
      host.state = 'running'
      host.startedAt = now
      host.idleSince = null
      host.lastCheckpointAt = now
    })()
    try {
      await host.boot
    } finally {
      host.boot = null
      host.died = null
    }
  }

  private guestDied(owner: ManagedHostOwner): void {
    const host = this.hosts.get(ownerKey(owner))
    if (!host || host.state === 'off' || host.state === 'saving') return
    host.deaths.push(this.now())
    host.state = 'off'
    host.died?.reject(new ManagedHostError('guest_died', 'the machine died while booting'))
    this.options.registry.disconnect(host.deviceId)
    this.log(`managed host of ${ownerKey(owner)} died (${host.deaths.length} deaths recorded)`)
  }

  /** The idle rule, the hard cap and the checkpoint clock over every running guest. */
  async sweep(): Promise<void> {
    if (this.sweeping || this.closed) return
    this.sweeping = true
    try {
      for (const host of [...this.hosts.values()]) {
        if (host.state !== 'running') continue
        const now = this.now()
        const turn = await this.options.turnInFlight(host.owner)
        const jobs = this.options.registry.runningJobs(host.deviceId)
        if (turn || jobs > 0) host.idleSince = null
        else host.idleSince ??= now
        const idle = host.idleSince !== null && now - host.idleSince >= this.config.idleMs
        const capped = !turn && now - host.startedAt >= this.config.hardCapMs
        if (idle || capped) {
          await this.hibernate(host.owner).catch((error) => this.log(`hibernate of ${ownerKey(host.owner)} failed: ${errorMessage(error)}`))
          continue
        }
        if (now - host.lastCheckpointAt >= this.config.checkpointIntervalMs) await this.checkpoint(host)
      }
    } finally {
      this.sweeping = false
    }
  }

  private async checkpoint(host: OwnedHost): Promise<void> {
    this.options.registry.pauseLiveness(host.deviceId)
    try {
      await this.options.provisioner.checkpoint(host.owner)
      host.lastCheckpointAt = this.now()
    } catch (error) {
      this.log(`checkpoint of ${ownerKey(host.owner)} failed: ${errorMessage(error)}`)
    } finally {
      this.options.registry.resumeLiveness(host.deviceId)
    }
  }
}
