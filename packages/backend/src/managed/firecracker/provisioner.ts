import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { constants, copyFile, mkdir, readFile, readdir, rename, rm, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { createId, delay, errorCode, errorMessage, SerialQueue } from '@demicodes/utils'
import {
  atomicJson,
  DirMachineImageStore,
  safeImageId,
  syncFile,
  type MachineImageStore,
} from '../../storage/machine-image-store'
import {
  imageStateSchema,
  type BootArgs,
  type MachineImageState,
  type ManagedHostProvisioner,
  type ManagedVolume,
} from '../provisioner'
import { bootArgs } from './boot-args'
import type { FirecrackerConfig } from './config'
import { growImage, makeHomeImage, makeSystemImage, missingImageTools } from './image-tools'
import { SlotPool, type Slot } from './slots'
import {
  killVm,
  processAlive,
  readVmRecord,
  removeVmRecord,
  startVm,
  vmDirectory,
  type RunningVm,
} from './vm'

interface Guest {
  id: string
  directory: string
  vm: RunningVm | null
  slot: Slot | null
  stopping: boolean
  transitions: SerialQueue
}
export interface ImageTools {
  makeHomeImage: typeof makeHomeImage
  makeSystemImage: typeof makeSystemImage
  growImage: typeof growImage
}
export interface ProcessControl {
  alive(pid: number): boolean
  kill(vmId: string, pid: number): Promise<void>
}
export interface FirecrackerProvisionerOptions {
  store?: MachineImageStore
  log?: (line: string) => void
  tools?: ImageTools
  processes?: ProcessControl
}

/** Working disks survive failed saves. A generation publishes system and home together. */
export class FirecrackerProvisioner implements ManagedHostProvisioner {
  private readonly guests = new Map<string, Guest>()
  private readonly slots: SlotPool
  private readonly store: MachineImageStore
  private readonly tools: ImageTools
  private readonly processes: ProcessControl
  private readonly deathListeners: Array<(deviceId: string) => void> = []
  private readonly log: (line: string) => void
  private base: Promise<string> | null = null
  private readonly workDir: string

  constructor(
    private readonly config: FirecrackerConfig,
    options: FirecrackerProvisionerOptions = {},
  ) {
    this.workDir = join(config.runDir, 'machines')
    if (!options.tools) {
      const missing = missingImageTools()
      if (missing.length) {
        throw new Error(`managed hosts need ${missing.join(', ')}`)
      }
    }
    this.slots = new SlotPool({ subnet: config.subnet, count: config.slots, tapPrefix: config.tapPrefix })
    this.store = options.store ?? new DirMachineImageStore(config.imagesDir)
    this.tools = options.tools ?? { makeHomeImage, makeSystemImage, growImage }
    this.processes = options.processes ?? { alive: processAlive, kill: (id, pid) => killVm(config, id, pid) }
    this.log = options.log ?? console.warn
  }

  currentBaseVersion(): Promise<string> {
    return (this.base ??= this.pinBase())
  }

  private async pinBase(): Promise<string> {
    const bases = join(this.config.imagesDir, 'bases')
    await mkdir(bases, { recursive: true })
    const stage = join(bases, `.stage-${createId()}`)
    await mkdir(stage)
    try {
      const hash = createHash('sha256')
      for (const [name, source] of [
        ['kernel', this.config.kernel],
        ['rootfs', this.config.rootfs],
      ] as const) {
        const path = join(stage, name)
        await copyFile(source, path, constants.COPYFILE_FICLONE)
        hash.update(name)
        for await (const chunk of createReadStream(path)) {
          hash.update(chunk)
        }
        await syncFile(path)
      }
      const version = hash.digest('hex')
      await syncFile(stage)
      try {
        await rename(stage, join(bases, version))
      } catch (error) {
        // Another pin of the same content may have published this version already.
        if (!['EEXIST', 'ENOTEMPTY'].includes(errorCode(error) ?? '')) {
          throw error
        }
      }
      await syncFile(bases)
      return version
    } finally {
      await rm(stage, { recursive: true, force: true })
    }
  }

  imageState(id: string): Promise<MachineImageState | null> {
    return this.store.read(id)
  }

  async reconcile(): Promise<void> {
    await mkdir(this.workDir, { recursive: true })
    for (const entry of await readdir(this.config.runDir, { withFileTypes: true })) {
      if (!entry.isDirectory() || !entry.name.startsWith('vm-')) {
        continue
      }
      const record = await readVmRecord(this.config, entry.name)
      if (record && this.processes.alive(record.pid)) {
        await this.processes.kill(entry.name, record.pid)
        const deadline = Date.now() + 15_000
        while (this.processes.alive(record.pid)) {
          if (Date.now() > deadline) {
            throw new Error(`VM ${entry.name} did not stop; disks cannot be saved`)
          }
          await delay(100)
        }
      }
      await rm(vmDirectory(this.config, entry.name), { recursive: true, force: true })
    }
    for (const name of await readdir(this.workDir)) {
      if (name.startsWith('.')) {
        await rm(join(this.workDir, name), { recursive: true, force: true })
        continue
      }
      await this.save(this.guest(name))
    }
    await this.currentBaseVersion()
  }

  private guest(id: string): Guest {
    safeImageId(id)
    let guest = this.guests.get(id)
    if (!guest) {
      guest = {
        id,
        directory: join(this.workDir, id),
        vm: null,
        slot: null,
        stopping: false,
        transitions: new SerialQueue(),
      }
      this.guests.set(id, guest)
    }
    return guest
  }

  private async readWorkingState(guest: Guest): Promise<MachineImageState | null> {
    try {
      const manifest = await readFile(join(guest.directory, 'manifest.json'), 'utf8')
      return imageStateSchema.parse(JSON.parse(manifest))
    } catch (error) {
      if (errorCode(error) === 'ENOENT') {
        return null
      }
      throw error
    }
  }

  private async save(guest: Guest): Promise<void> {
    const state = await this.readWorkingState(guest)
    if (!state) {
      return
    }
    await this.publishDisks(guest.id, state, guest.directory)
    await rm(guest.directory, { recursive: true })
    await syncFile(this.workDir)
  }

  private async publishDisks(id: string, state: MachineImageState, directory: string): Promise<void> {
    const system = await stat(join(directory, 'system.ext4'))
    const home = await stat(join(directory, 'home.ext4'))
    const generation = {
      ...state,
      generation: createId(),
      systemBytes: system.size,
      homeBytes: home.size,
    }
    await this.store.publish(id, generation, directory)
  }

  private async initialize(id: string, baseVersion: string): Promise<MachineImageState> {
    const directory = join(this.workDir, `.initial-${createId()}`)
    await mkdir(directory, { recursive: true })
    try {
      const home = join(directory, 'empty-home')
      await mkdir(home)
      const state = {
        generation: createId(),
        baseVersion,
        resetId: null,
        systemBytes: this.config.systemMib * 1024 ** 2,
        homeBytes: this.config.homeMib * 1024 ** 2,
      }
      await this.tools.makeHomeImage(home, join(directory, 'home.ext4'), state.homeBytes)
      await this.tools.makeSystemImage(join(directory, 'system.ext4'), state.systemBytes)
      await this.store.publish(id, state, directory)
      return state
    } finally {
      await rm(directory, { recursive: true, force: true })
    }
  }

  wake(id: string, boot: BootArgs): Promise<void> {
    const guest = this.guest(id)
    return guest.transitions.run(async () => {
      if (guest.vm) {
        return
      }
      await this.save(guest)
      let state = await this.store.read(id)
      const firstBoot = !state
      if (!state) {
        state = await this.initialize(id, await this.currentBaseVersion())
      }

      const stage = join(this.workDir, `.wake-${createId()}`)
      await this.store.copy(id, state, stage)
      await Promise.all(['home.ext4', 'system.ext4'].map(volume => syncFile(join(stage, volume))))
      await atomicJson(join(stage, 'manifest.json'), state)
      await syncFile(stage)
      await rename(stage, guest.directory)
      await syncFile(this.workDir)

      const slot = this.slots.take()
      const vmId = `vm-${createId().slice(0, 12)}`
      let vm: RunningVm
      try {
        const base = join(this.config.imagesDir, 'bases', safeImageId(state.baseVersion))
        vm = await startVm(
          { ...this.config, kernel: join(base, 'kernel'), rootfs: join(base, 'rootfs') },
          {
            vmId,
            slot,
            homeImage: join(guest.directory, 'home.ext4'),
            systemImage: join(guest.directory, 'system.ext4'),
            bootArgs: bootArgs({ ...boot, slot, dns: this.config.dns, firstBoot }),
            owner: id,
          },
          this.log,
        )
      } catch (error) {
        this.slots.release(slot)
        throw error
      }
      guest.vm = vm
      guest.slot = slot
      guest.stopping = false

      void vm.exited.then(async () => {
        if (guest.vm !== vm) {
          return
        }
        const died = !guest.stopping
        await this.releaseVm(guest, vmId)
        if (died) {
          for (const listener of this.deathListeners) {
            listener(id)
          }
        }
      })
    })
  }

  hibernate(id: string): Promise<void> {
    const guest = this.guest(id)
    return guest.transitions.run(async () => {
      await this.stop(guest)
      await this.save(guest)
    })
  }

  checkpoint(id: string): Promise<void> {
    const guest = this.guest(id)
    return guest.transitions.run(async () => {
      if (!guest.vm) {
        return
      }
      const state = await this.readWorkingState(guest)
      if (!state) {
        throw new Error('Running machine has no manifest')
      }
      const stage = join(this.workDir, `.checkpoint-${createId()}`)
      await mkdir(stage)
      try {
        await guest.vm.api.pause()
        try {
          for (const volume of ['system', 'home']) {
            await copyFile(
              join(guest.directory, `${volume}.ext4`),
              join(stage, `${volume}.ext4`),
              constants.COPYFILE_FICLONE,
            )
          }
        } finally {
          await guest.vm.api.resume()
        }
        await this.publishDisks(id, state, stage)
      } finally {
        await rm(stage, { recursive: true, force: true })
      }
    })
  }

  growVolume(id: string, volume: ManagedVolume, bytes: number): Promise<void> {
    const guest = this.guest(id)
    return guest.transitions.run(async () => {
      if (!guest.vm) {
        throw new Error('Machine is not running')
      }
      await this.tools.growImage(join(guest.directory, `${volume}.ext4`), bytes)
      await guest.vm.api.rescanVolume(volume, guest.vm.volumePaths[volume])
    })
  }

  reset(id: string, operationId: string, baseVersion: string): Promise<void> {
    const guest = this.guest(id)
    return guest.transitions.run(async () => {
      await this.stop(guest)
      await this.save(guest)
      let state = await this.store.read(id)
      if (!state) {
        state = await this.initialize(id, baseVersion)
      }
      if (state.resetId === operationId) {
        return
      }
      const stage = join(this.workDir, `.reset-${createId()}`)
      try {
        await this.store.copy(id, state, stage)
        await rm(join(stage, 'system.ext4'))
        const systemBytes = this.config.systemMib * 1024 ** 2
        await this.tools.makeSystemImage(join(stage, 'system.ext4'), systemBytes)
        await this.store.publish(
          id,
          { ...state, generation: createId(), resetId: operationId, baseVersion, systemBytes },
          stage,
        )
      } finally {
        await rm(stage, { recursive: true, force: true })
      }
    })
  }

  onDeath(listener: (id: string) => void): void {
    this.deathListeners.push(listener)
  }

  running(id: string): boolean {
    return !!this.guests.get(id)?.vm
  }

  async close(): Promise<void> {
    for (const guest of this.guests.values()) {
      await this.hibernate(guest.id).catch(error => this.log(errorMessage(error)))
    }
  }

  private async releaseVm(guest: Guest, vmId: string): Promise<void> {
    guest.vm = null
    if (guest.slot) {
      this.slots.release(guest.slot)
    }
    guest.slot = null
    await removeVmRecord(this.config, vmId)
  }

  private async stop(guest: Guest): Promise<void> {
    const vm = guest.vm
    if (!vm) {
      return
    }
    guest.stopping = true
    await vm.kill()
    await vm.exited
    if (guest.vm === vm) {
      await this.releaseVm(guest, vm.id)
    }
  }
}
