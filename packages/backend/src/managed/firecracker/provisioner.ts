// The Firecracker provisioner (`managed-hosts.md`): the seam's production
// implementation. A guest per owner over a working home image under the
// run directory; the home-image store holds the durable copy, written at
// hibernate and checkpoint. Images never enter this process's memory.
import { constants, copyFile, mkdir, rm, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { createId, errorMessage } from '@demicodes/utils'
import type { ManagedHostOwner } from '../../storage/control'
import { DirHomeImageStore, type HomeImageStore } from '../../storage/home-image-store'
import { ownerKey, type BootArgs, type ManagedHostProvisioner } from '../provisioner'
import { bootArgs } from './boot-args'
import type { FirecrackerConfig } from './config'
import { growImage, makeHomeImage, missingImageTools, shrinkImage } from './image-tools'
import { SlotPool, type Slot } from './slots'
import { startVm, type RunningVm } from './vm'

interface Guest {
  owner: ManagedHostOwner
  /** The working image, `<runDir>/homes/<ownerKey>.ext4`, present while provisioned or running. */
  workImage: string
  vm: RunningVm | null
  slot: Slot | null
  /** Set around a kill this provisioner performs, so the exit is not a death. */
  stopping: boolean
}

export class FirecrackerProvisioner implements ManagedHostProvisioner {
  private readonly guests = new Map<string, Guest>()
  private readonly slots: SlotPool
  private readonly store: HomeImageStore
  private readonly deathListeners: Array<(owner: ManagedHostOwner) => void> = []
  private readonly log: (line: string) => void

  constructor(
    private readonly config: FirecrackerConfig,
    options: { store?: HomeImageStore; log?: (line: string) => void } = {},
  ) {
    const missing = missingImageTools()
    if (missing.length > 0) throw new Error(`managed hosts need ${missing.join(', ')} on this machine`)
    this.slots = new SlotPool({ subnet: config.subnet, count: config.slots, tapPrefix: config.tapPrefix })
    this.store = options.store ?? new DirHomeImageStore(config.homesDir)
    this.log = options.log ?? ((line) => console.warn(line))
  }

  async provision(owner: ManagedHostOwner, homeDir: string, boot: BootArgs): Promise<void> {
    const guest = this.guest(owner)
    if (guest.vm) throw new Error(`guest ${ownerKey(owner)} already runs`)
    await mkdir(join(this.config.runDir, 'homes'), { recursive: true })
    // The image is made, shrunk and stored before the first boot: the store holds every owner from the start.
    await makeHomeImage(homeDir, guest.workImage, this.config.homeMib * 1024 * 1024)
    await shrinkImage(guest.workImage)
    await this.store.put(ownerKey(owner), guest.workImage)
    await this.boot(guest, boot, true)
  }

  async wake(owner: ManagedHostOwner, boot: BootArgs): Promise<void> {
    const guest = this.guest(owner)
    if (guest.vm) throw new Error(`guest ${ownerKey(owner)} already runs`)
    await this.boot(guest, boot, false)
  }

  async hibernate(owner: ManagedHostOwner, report: { untouched: boolean }): Promise<void> {
    const guest = this.guest(owner)
    await this.stop(guest)
    if (report.untouched) {
      await rm(guest.workImage, { force: true })
      return
    }
    const bytes = await shrinkImage(guest.workImage)
    await this.store.put(ownerKey(owner), guest.workImage)
    this.log(`guest ${ownerKey(owner)} hibernated; home ${bytes} bytes`)
  }

  async checkpoint(owner: ManagedHostOwner): Promise<void> {
    const guest = this.guest(owner)
    if (!guest.vm) return
    const copy = `${guest.workImage}.checkpoint-${createId()}`
    await guest.vm.api.pause()
    try {
      await copyFile(guest.workImage, copy, constants.COPYFILE_FICLONE)
    } finally {
      await guest.vm.api.resume()
    }
    try {
      await shrinkImage(copy)
      await this.store.put(ownerKey(owner), copy)
    } catch (error) {
      await rm(copy, { force: true })
      throw error
    }
  }

  async growHome(owner: ManagedHostOwner, bytes: number): Promise<void> {
    const guest = this.guest(owner)
    if (!guest.vm) throw new Error(`guest ${ownerKey(owner)} is not running`)
    await growImage(guest.workImage, bytes)
    await guest.vm.api.rescanHome(guest.vm.homePathInVm)
  }

  async destroy(owner: ManagedHostOwner): Promise<void> {
    const guest = this.guests.get(ownerKey(owner))
    if (!guest) return
    await this.stop(guest)
    await rm(guest.workImage, { force: true })
    this.guests.delete(ownerKey(owner))
  }

  onDeath(listener: (owner: ManagedHostOwner) => void): void {
    this.deathListeners.push(listener)
  }

  /** Every guest killed; the images stay where they are. */
  async close(): Promise<void> {
    for (const guest of this.guests.values()) await this.stop(guest).catch((error) => this.log(`stopping ${ownerKey(guest.owner)}: ${errorMessage(error)}`))
  }

  running(owner: ManagedHostOwner): boolean {
    return (this.guests.get(ownerKey(owner))?.vm ?? null) !== null
  }

  private guest(owner: ManagedHostOwner): Guest {
    const key = ownerKey(owner)
    let guest = this.guests.get(key)
    if (!guest) {
      guest = { owner, workImage: join(this.config.runDir, 'homes', `${key}.ext4`), vm: null, slot: null, stopping: false }
      this.guests.set(key, guest)
    }
    return guest
  }

  private async boot(guest: Guest, boot: BootArgs, firstBoot: boolean): Promise<void> {
    const key = ownerKey(guest.owner)
    // The working image: the store's current one, enlarged to the nominal size; the guest grows the filesystem at boot.
    const present = await stat(guest.workImage).then(() => true, () => false)
    if (!present) await this.store.get(key, guest.workImage)
    await growImage(guest.workImage, this.config.homeMib * 1024 * 1024)
    const slot = this.slots.take()
    // Short: the API socket path under the run directory must fit a unix socket address (108 bytes).
    const vmId = `vm-${createId().slice(0, 12)}`
    let vm: RunningVm
    try {
      vm = await startVm(this.config, { vmId, slot, homeImage: guest.workImage, bootArgs: bootArgs({ backendUrl: boot.backendUrl, deviceToken: boot.deviceToken, slot, dns: this.config.dns, firstBoot }) }, this.log)
    } catch (error) {
      this.slots.release(slot)
      throw error
    }
    guest.vm = vm
    guest.slot = slot
    guest.stopping = false
    void vm.exited.then((code) => {
      if (guest.vm !== vm) return
      guest.vm = null
      this.slots.release(slot)
      guest.slot = null
      if (!guest.stopping) {
        this.log(`guest ${key} died (exit ${code ?? 'signal'})`)
        for (const listener of this.deathListeners) listener(guest.owner)
      }
    })
  }

  private async stop(guest: Guest): Promise<void> {
    const vm = guest.vm
    if (!vm) return
    guest.stopping = true
    await vm.kill()
    await vm.exited
    guest.vm = null
    if (guest.slot) this.slots.release(guest.slot)
    guest.slot = null
  }
}
