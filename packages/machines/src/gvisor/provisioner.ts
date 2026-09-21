import { cp, mkdir, readFile, readdir, rename, rm, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { ActivityGate, createId, errorCode, errorMessage, SerialQueue } from '@demicodes/utils'
import { atomicJson, copyMachineImage, DirMachineImageStore, safeImageId, syncFile, type MachineImageStore } from '../machine-image-store'
import { imageStateSchema, type BootArgs, type MachineImageState, type MachineRuntimeState, type ManagedHostProvisioner, type ManagedVolume } from '../provisioner'
import { importBase } from './base'
import type { GVisorConfig } from './config'
import { makeHomeImage, makeSystemImage, recoverVolume, requireTool } from './image-tools'
import { CloudNetwork } from './network'
import { Sandbox, sandboxRecordSchema } from './sandbox'
import { SlotPool, type Slot } from './slots'

interface Device {
  id: string
  directory: string
  runtime: Sandbox | null
  slot: Slot | null
  transitions: SerialQueue
}

/** One serialized storage/runtime owner per persistent Cloud device. */
export class GVisorProvisioner implements ManagedHostProvisioner {
  private readonly admission = new ActivityGate()
  private readonly devices = new Map<string, Device>()
  private readonly slots: SlotPool
  private readonly network: CloudNetwork
  private readonly store: MachineImageStore
  private readonly deaths = new Set<(id: string) => void>()
  private base: Promise<string> | null = null

  constructor(private readonly config: GVisorConfig) {
    this.slots = new SlotPool(config.subnet, config.slots)
    this.network = new CloudNetwork(config)
    this.store = new DirMachineImageStore(config.imagesDir)
  }

  currentBaseVersion(): Promise<string> {
    return this.base ??= importBase(this.config)
  }

  imageState(id: string): Promise<MachineImageState | null> {
    return this.store.read(id)
  }

  runtimeState(id: string): Promise<MachineRuntimeState> {
    const device = this.device(id)
    return this.transition(device, async () => device.runtime ? 'running' : 'stopped')
  }

  private device(id: string): Device {
    safeImageId(id)
    let device = this.devices.get(id)
    if (!device) {
      device = { id, directory: join(this.config.runDir, id), runtime: null, slot: null, transitions: new SerialQueue() }
      this.devices.set(id, device)
    }
    return device
  }

  /** Keep device transitions concurrent while manager-wide recovery excludes new work. */
  private async transition<T>(device: Device, operation: () => Promise<T>): Promise<T> {
    const release = await this.admission.enter()
    try {
      return await device.transitions.run(operation)
    } finally {
      release()
    }
  }

  private async working(device: Device): Promise<MachineImageState | null> {
    try {
      return imageStateSchema.parse(JSON.parse(await readFile(join(device.directory, 'manifest.json'), 'utf8')))
    } catch (error) {
      if (errorCode(error) === 'ENOENT') return null
      throw error
    }
  }

  private async publish(device: Device, state: MachineImageState, directory: string): Promise<void> {
    await this.store.publish(device.id, { ...state, generation: createId() }, directory)
  }

  private async save(device: Device): Promise<void> {
    const state = await this.working(device)
    if (!state) return
    if (device.runtime) throw new Error('Cannot publish working disks with an active writer')
    const systemBytes = await recoverVolume(join(device.directory, 'system.ext4'))
    const homeBytes = await recoverVolume(join(device.directory, 'home.ext4'))
    await this.publish(device, { ...state, systemBytes, homeBytes }, device.directory)
    await rm(device.directory, { recursive: true })
    await syncFile(this.config.runDir)
  }

  async reconcile(): Promise<void> {
    const release = await this.admission.reserve()
    try {
      await this.recover()
    } finally {
      release()
    }
  }

  private async recover(): Promise<void> {
    await this.drain()
    await mkdir(this.config.runDir, { recursive: true, mode: 0o700 })
    await mkdir(join(this.config.runtimeDir, 'runsc'), { recursive: true, mode: 0o700 })
    for (const name of await readdir(this.config.runDir)) {
      if (name.startsWith('.')) {
        // Staging copies have not replaced a working or committed pair.
        await rm(join(this.config.runDir, name), { recursive: true, force: true })
        continue
      }
      const device = this.device(name)
      const saved = await readFile(join(device.directory, 'sandbox.json'), 'utf8').catch(error => {
        if (errorCode(error) === 'ENOENT') return null
        throw error
      })
      if (saved !== null) {
        const record = sandboxRecordSchema.parse(JSON.parse(saved))
        if (record.slot >= this.config.slots) throw new Error('Existing Cloud slot exceeds configured pool')
        await new Sandbox(this.config, record, device.directory, this.slots.slot(record.slot), this.network).close()
      }
      await this.save(device)
    }
    await this.network.prepare()
    await this.currentBaseVersion()
  }

  private async initialize(device: Device, baseVersion: string): Promise<MachineImageState> {
    const stage = join(this.config.runDir, `.initial-${createId()}`)
    await mkdir(stage)
    try {
      const home = join(stage, 'home')
      await cp(join(this.config.imagesDir, 'bases', safeImageId(baseVersion), 'rootfs', 'etc/skel'), home, { recursive: true, dereference: false })
      await requireTool('chown', ['-R', '--no-dereference', '1000:1000', home])
      const state = {
        generation: createId(), baseVersion, resetId: null,
        systemBytes: this.config.systemMib * 1024 ** 2,
        homeBytes: this.config.homeMib * 1024 ** 2,
      }
      await makeHomeImage(home, join(stage, 'home.ext4'), state.homeBytes)
      await makeSystemImage(join(stage, 'system.ext4'), state.systemBytes)
      await this.store.publish(device.id, state, stage)
      return state
    } finally {
      await rm(stage, { recursive: true, force: true })
    }
  }

  wake(id: string, boot: BootArgs): Promise<void> {
    const device = this.device(id)
    return this.transition(device, async () => {
      if (device.runtime) return
      await this.save(device)
      const state = await this.store.read(id) ?? await this.initialize(device, await this.currentBaseVersion())
      const stage = join(this.config.runDir, `.wake-${createId()}`)
      try {
        await this.store.copy(id, state, stage)
        for (const volume of ['system', 'home']) await syncFile(join(stage, `${volume}.ext4`))
        await atomicJson(join(stage, 'manifest.json'), state)
        await syncFile(stage)
        await rename(stage, device.directory)
        await syncFile(this.config.runDir)
      } finally {
        await rm(stage, { recursive: true, force: true })
      }
      const slot = this.slots.take()
      const runtime = new Sandbox(this.config, { id: `demi-${createId()}`, slot: slot.index }, device.directory, slot, this.network)
      device.runtime = runtime
      device.slot = slot
      try {
        await runtime.start(join(this.config.imagesDir, 'bases', state.baseVersion, 'rootfs'), boot)
      } catch (error) {
        try {
          await this.stop(device)
        } catch (cleanup) {
          throw new AggregateError([error, cleanup], 'Cloud start and cleanup failed; working storage retained')
        }
        throw error
      }
      void runtime.exited?.then(
        () => this.runtimeLost(device, runtime),
        error => {
          console.error(errorMessage(error))
          return this.runtimeLost(device, runtime)
        },
      ).catch(error => console.error(`Cloud recovery failed: ${errorMessage(error)}`))
    })
  }

  private async runtimeLost(device: Device, runtime: Sandbox): Promise<void> {
    await this.transition(device, async () => {
      if (device.runtime !== runtime) return
      try {
        await this.stop(device)
        await this.save(device)
      } finally {
        for (const listener of this.deaths) listener(device.id)
      }
    })
  }

  private async stop(device: Device): Promise<void> {
    if (!device.runtime) return
    await device.runtime.close()
    device.runtime = null
    if (device.slot) this.slots.release(device.slot)
    device.slot = null
  }

  hibernate(id: string): Promise<void> {
    const device = this.device(id)
    return this.transition(device, async () => {
      await this.stop(device)
      await this.save(device)
    })
  }

  checkpoint(id: string): Promise<void> {
    const device = this.device(id)
    return this.transition(device, async () => {
      const runtime = device.runtime
      if (!runtime) return
      const state = await this.working(device)
      if (!state) throw new Error('Running Cloud has no working manifest')
      const stage = join(this.config.runDir, `.checkpoint-${createId()}`)
      await mkdir(stage)
      try {
        const frozen: ManagedVolume[] = []
        let captureError: unknown
        try {
          await runtime.pause()
          for (const volume of ['system', 'home'] as const) {
            // Record the attempt: a failed response does not establish absence of freeze.
            frozen.push(volume)
            await runtime.freeze(volume)
          }
          for (const volume of frozen) {
            const target = join(stage, `${volume}.ext4`)
            await copyMachineImage(join(device.directory, `${volume}.ext4`), target)
            await syncFile(target)
          }
        } catch (error) {
          captureError = error
        }
        const cleanup = await Promise.allSettled(frozen.map(volume => runtime.thaw(volume)))
        const errors = cleanup.flatMap(result => result.status === 'rejected' ? [result.reason] : [])
        try {
          await runtime.resume()
        } catch (error) {
          errors.push(error)
        }
        if (errors.length) {
          // End the broken runtime rather than expose it as running with frozen storage.
          await this.stop(device)
          for (const listener of this.deaths) listener(id)
          throw new AggregateError([...(captureError ? [captureError] : []), ...errors], 'Cloud checkpoint recovery failed')
        }
        if (captureError) throw captureError
        await this.publish(device, state, stage)
      } finally {
        await rm(stage, { recursive: true, force: true })
      }
    })
  }

  growVolume(id: string, volume: ManagedVolume, bytes: number): Promise<void> {
    const device = this.device(id)
    return this.transition(device, async () => {
      if (!device.runtime) throw new Error('Cloud is not running')
      const state = await this.working(device)
      if (!state) throw new Error('Cloud working manifest is missing')
      const key = volume === 'system' ? 'systemBytes' : 'homeBytes'
      if (bytes <= state[key]) return
      const capacity = await device.runtime.grow(volume, bytes)
      await atomicJson(join(device.directory, 'manifest.json'), { ...state, [key]: capacity })
      await syncFile(device.directory)
    })
  }

  reset(id: string, operationId: string, baseVersion: string): Promise<void> {
    const device = this.device(id)
    safeImageId(baseVersion)
    return this.transition(device, async () => {
      await stat(join(this.config.imagesDir, 'bases', baseVersion, 'manifest.json'))
      await this.stop(device)
      await this.save(device)
      const state = await this.store.read(id) ?? await this.initialize(device, baseVersion)
      if (state.resetId === operationId) return
      const stage = join(this.config.runDir, `.reset-${createId()}`)
      try {
        await this.store.copy(id, state, stage)
        await rm(join(stage, 'system.ext4'))
        const systemBytes = this.config.systemMib * 1024 ** 2
        await makeSystemImage(join(stage, 'system.ext4'), systemBytes)
        await this.store.publish(id, { ...state, generation: createId(), resetId: operationId, baseVersion, systemBytes }, stage)
      } finally {
        await rm(stage, { recursive: true, force: true })
      }
    })
  }

  onDeath(listener: (id: string) => void): void {
    this.deaths.add(listener)
  }

  async close(): Promise<void> {
    const release = await this.admission.reserve()
    try {
      await this.drain()
    } finally {
      release()
    }
  }

  private async drain(): Promise<void> {
    const outcomes = await Promise.allSettled([...this.devices.values()].map(device => device.transitions.run(async () => {
      await this.stop(device)
      await this.save(device)
    })))
    const errors = outcomes.flatMap(result => result.status === 'rejected' ? [result.reason] : [])
    if (errors.length) throw new AggregateError(errors, 'Cloud shutdown failed; working state retained')
  }
}
