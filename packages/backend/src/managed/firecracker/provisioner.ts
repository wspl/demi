// The Firecracker provisioner (`managed-hosts.md`): the seam's production
// implementation. A guest per owner over a working home image under the
// run directory; the home-image store holds the durable copy. The one
// invariant every path keeps: a working image exists only while its guest
// runs or until its save succeeded, and the store holds the current home
// whenever no working image exists — so hibernate, destroy, the backend
// closing and a crash found at the next start all end in the same save.
// Images never enter this process's memory.
import { constants, copyFile, mkdir, readdir, rm, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { createId, delay, errorMessage, SerialQueue } from '@demicodes/utils'
import type { ManagedHostOwner } from '../../storage/control'
import { DirHomeImageStore, type HomeImageStore } from '../../storage/home-image-store'
import { ownerFromKey, ownerKey, type BootArgs, type ManagedHostProvisioner } from '../provisioner'
import { bootArgs } from './boot-args'
import type { FirecrackerConfig } from './config'
import { growImage, makeHomeImage, missingImageTools, shrinkImage } from './image-tools'
import { SlotPool, type Slot } from './slots'
import { killVm, processAlive, readVmRecord, removeVmRecord, startVm, vmDirectory, type RunningVm } from './vm'

interface Guest {
  owner: ManagedHostOwner
  /** The working image, `<runDir>/homes/<ownerKey>.ext4`, present while running or until saved. */
  workImage: string
  vm: RunningVm | null
  slot: Slot | null
  /** Set around a kill this provisioner performs, so the exit is not a death. */
  stopping: boolean
  /** One guest's transitions run one at a time: a checkpoint never copies under a kill, a save never races another. */
  transitions: SerialQueue
}

/** The image tools, injectable so the reconciliation and save paths run in tests without e2fsprogs. */
export interface ImageTools {
  makeHomeImage: typeof makeHomeImage
  shrinkImage: typeof shrinkImage
  growImage: typeof growImage
}

/** The process facts reconciliation needs, injectable for tests. */
export interface ProcessControl {
  alive(pid: number): boolean
  kill(vmId: string, pid: number): Promise<void>
}

export interface FirecrackerProvisionerOptions {
  store?: HomeImageStore
  log?: (line: string) => void
  tools?: ImageTools
  processes?: ProcessControl
}

/** How long a VM found at start may take to die after the kill before reconciliation gives up on it. */
const RECONCILE_KILL_MS = 15_000

export class FirecrackerProvisioner implements ManagedHostProvisioner {
  private readonly guests = new Map<string, Guest>()
  private readonly slots: SlotPool
  private readonly store: HomeImageStore
  private readonly tools: ImageTools
  private readonly processes: ProcessControl
  private readonly deathListeners: Array<(owner: ManagedHostOwner) => void> = []
  private readonly log: (line: string) => void

  constructor(
    private readonly config: FirecrackerConfig,
    options: FirecrackerProvisionerOptions = {},
  ) {
    if (!options.tools) {
      const missing = missingImageTools()
      if (missing.length > 0) throw new Error(`managed hosts need ${missing.join(', ')} on this machine`)
    }
    this.slots = new SlotPool({ subnet: config.subnet, count: config.slots, tapPrefix: config.tapPrefix })
    this.store = options.store ?? new DirHomeImageStore(config.homesDir)
    this.tools = options.tools ?? { makeHomeImage, shrinkImage, growImage }
    this.processes = options.processes ?? { alive: processAlive, kill: (vmId, pid) => killVm(config, vmId, pid) }
    this.log = options.log ?? ((line) => console.warn(line))
  }

  /**
   * What a previous backend process left: VMs still running (killed — a
   * second guest over the same image would corrupt it, and their taps are
   * this pool's), then every working image, which is newer than the store
   * (saved; a save that fails keeps the image and refuses that owner's
   * next boot until it succeeds).
   */
  async reconcile(): Promise<void> {
    await mkdir(this.workDir, { recursive: true })
    await mkdir(this.config.runDir, { recursive: true })
    for (const entry of await readdir(this.config.runDir, { withFileTypes: true })) {
      if (!entry.isDirectory() || entry.name === 'homes') continue
      const vmId = entry.name
      const record = await readVmRecord(this.config, vmId)
      if (record && this.processes.alive(record.pid)) {
        this.log(`vm ${vmId} of ${record.owner} outlived the previous backend; killing it`)
        await this.processes.kill(vmId, record.pid)
        const deadline = Date.now() + RECONCILE_KILL_MS
        while (this.processes.alive(record.pid)) {
          if (Date.now() > deadline) throw new Error(`vm ${vmId} (pid ${record.pid}) of ${record.owner} did not die; its home cannot be saved safely`)
          await delay(100)
        }
      }
      await rm(vmDirectory(this.config, vmId), { recursive: true, force: true })
    }
    for (const name of await readdir(this.workDir)) {
      if (name.includes('.checkpoint-')) {
        await rm(join(this.workDir, name), { force: true })
        continue
      }
      const owner = name.endsWith('.ext4') ? ownerFromKey(name.slice(0, -'.ext4'.length)) : null
      if (!owner) continue
      await this.save(this.guest(owner)).catch((error) => this.log(`home of ${ownerKey(owner)} left by the previous backend is not saved yet: ${errorMessage(error)}`))
    }
  }

  provision(owner: ManagedHostOwner, homeDir: string, boot: BootArgs): Promise<void> {
    const guest = this.guest(owner)
    return guest.transitions.run(async () => {
      if (guest.vm) throw new Error(`guest ${ownerKey(owner)} already runs`)
      await mkdir(this.workDir, { recursive: true })
      // The image is made and stored before the first boot: the store holds every owner from the start.
      await this.tools.makeHomeImage(homeDir, guest.workImage, this.config.homeMib * 1024 * 1024)
      await this.save(guest)
      await this.boot(guest, boot, true)
    })
  }

  wake(owner: ManagedHostOwner, boot: BootArgs): Promise<void> {
    const guest = this.guest(owner)
    return guest.transitions.run(async () => {
      if (guest.vm) throw new Error(`guest ${ownerKey(owner)} already runs`)
      await this.boot(guest, boot, false)
    })
  }

  hibernate(owner: ManagedHostOwner, report: { untouched: boolean }): Promise<void> {
    const guest = this.guest(owner)
    return guest.transitions.run(async () => {
      await this.stop(guest)
      if (report.untouched) {
        await rm(guest.workImage, { force: true })
        return
      }
      await this.save(guest)
    })
  }

  checkpoint(owner: ManagedHostOwner): Promise<void> {
    const guest = this.guest(owner)
    return guest.transitions.run(async () => {
      if (!guest.vm) return
      const copy = `${guest.workImage}.checkpoint-${createId()}`
      await guest.vm.api.pause()
      try {
        await copyFile(guest.workImage, copy, constants.COPYFILE_FICLONE)
      } finally {
        await guest.vm.api.resume()
      }
      try {
        await this.tools.shrinkImage(copy)
        await this.store.put(ownerKey(owner), copy)
      } catch (error) {
        await rm(copy, { force: true })
        throw error
      }
    })
  }

  growHome(owner: ManagedHostOwner, bytes: number): Promise<void> {
    const guest = this.guest(owner)
    return guest.transitions.run(async () => {
      if (!guest.vm) throw new Error(`guest ${ownerKey(owner)} is not running`)
      await this.tools.growImage(guest.workImage, bytes)
      await guest.vm.api.rescanHome(guest.vm.homePathInVm)
    })
  }

  /** The guest killed if it runs and its home saved; the entry is forgotten only once the save succeeded. */
  async destroy(owner: ManagedHostOwner): Promise<void> {
    const guest = this.guests.get(ownerKey(owner))
    if (!guest) return
    await guest.transitions.run(async () => {
      await this.stop(guest)
      await this.save(guest)
    })
    this.guests.delete(ownerKey(owner))
  }

  onDeath(listener: (owner: ManagedHostOwner) => void): void {
    this.deathListeners.push(listener)
  }

  /** Every guest killed and every home saved; an image whose save failed stays for the next start's reconciliation. */
  async close(): Promise<void> {
    for (const guest of this.guests.values()) {
      await guest.transitions
        .run(async () => {
          await this.stop(guest)
          await this.save(guest)
        })
        .catch((error) => this.log(`closing ${ownerKey(guest.owner)}: ${errorMessage(error)}`))
    }
  }

  running(owner: ManagedHostOwner): boolean {
    return (this.guests.get(ownerKey(owner))?.vm ?? null) !== null
  }

  private get workDir(): string {
    return join(this.config.runDir, 'homes')
  }

  private guest(owner: ManagedHostOwner): Guest {
    const key = ownerKey(owner)
    let guest = this.guests.get(key)
    if (!guest) {
      guest = { owner, workImage: join(this.workDir, `${key}.ext4`), vm: null, slot: null, stopping: false, transitions: new SerialQueue() }
      this.guests.set(key, guest)
    }
    return guest
  }

  /** The working image, when present, shrunk and made the store's current copy; nothing to do when there is none. */
  private async save(guest: Guest): Promise<void> {
    const present = await stat(guest.workImage).then(() => true, () => false)
    if (!present) return
    const key = ownerKey(guest.owner)
    try {
      const bytes = await this.tools.shrinkImage(guest.workImage)
      await this.store.put(key, guest.workImage)
      this.log(`home of ${key} saved: ${bytes} bytes`)
    } catch (error) {
      this.log(`home of ${key} not saved, kept as ${guest.workImage}: ${errorMessage(error)}`)
      throw error
    }
  }

  private async boot(guest: Guest, boot: BootArgs, firstBoot: boolean): Promise<void> {
    const key = ownerKey(guest.owner)
    // A working image left by a failed save is saved now, or the boot is refused: the store must hold the current home before a fresh copy is taken from it.
    await this.save(guest)
    // The working image: the store's current one, enlarged to the nominal size; the guest grows the filesystem at boot.
    await this.store.get(key, guest.workImage)
    await this.tools.growImage(guest.workImage, this.config.homeMib * 1024 * 1024)
    const slot = this.slots.take()
    // Short: the API socket path under the run directory must fit a unix socket address (108 bytes).
    const vmId = `vm-${createId().slice(0, 12)}`
    let vm: RunningVm
    try {
      vm = await startVm(this.config, { vmId, slot, homeImage: guest.workImage, bootArgs: bootArgs({ backendUrl: boot.backendUrl, deviceToken: boot.deviceToken, slot, dns: this.config.dns, firstBoot }), owner: key }, this.log)
    } catch (error) {
      this.slots.release(slot)
      await rm(vmDirectory(this.config, vmId), { recursive: true, force: true })
      throw error
    }
    guest.vm = vm
    guest.slot = slot
    guest.stopping = false
    void vm.exited.then((code) => {
      if (guest.vm !== vm) return
      const died = !guest.stopping
      void this.released(guest, vmId)
      if (died) {
        this.log(`guest ${key} died (exit ${code ?? 'signal'}); console at ${join(vmDirectory(this.config, vmId), 'console.log')}`)
        for (const listener of this.deathListeners) listener(guest.owner)
      }
    })
  }

  /** The VM process is gone: its slot back in the pool, its record removed so no later start tries to kill it. */
  private async released(guest: Guest, vmId: string): Promise<void> {
    guest.vm = null
    if (guest.slot) this.slots.release(guest.slot)
    guest.slot = null
    await removeVmRecord(this.config, vmId)
  }

  private async stop(guest: Guest): Promise<void> {
    const vm = guest.vm
    if (!vm) return
    guest.stopping = true
    await vm.kill()
    await vm.exited
    // The exit handler released it; a handler still queued finds `vm` gone and stops there.
    if (guest.vm === vm) await this.released(guest, vm.id)
  }

}

