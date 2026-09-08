import { constants, copyFile, mkdir, open, readFile, readdir, rename, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { z } from 'zod'
import { createId, errorCode } from '@demicodes/utils'
import { imageStateSchema, type MachineImageState } from '../managed/provisioner'

export function safeImageId(id: string): string {
  return z.string().regex(/^[A-Za-z0-9_-]+$/).parse(id)
}

export async function syncFile(path: string): Promise<void> {
  const file = await open(path, 'r')
  try { await file.sync() } finally { await file.close() }
}

export async function atomicJson(path: string, value: unknown): Promise<void> {
  const temp = `${path}.${createId()}`
  const file = await open(temp, 'wx')
  try { await file.writeFile(JSON.stringify(value)); await file.sync() }
  finally { await file.close() }
  try { await rename(temp, path) } finally { await rm(temp, { force: true }) }
}

export interface MachineImageStore {
  read(deviceId: string): Promise<MachineImageState | null>
  copy(deviceId: string, state: MachineImageState, destination: string): Promise<void>
  publish(deviceId: string, state: MachineImageState, source: string): Promise<void>
}

/** A generation is immutable. Only the manifest pointer is replaced, after both disks are durable. */
export class DirMachineImageStore implements MachineImageStore {
  constructor(private readonly root: string) {}

  private device(deviceId: string): string { return join(this.root, safeImageId(deviceId)) }

  async read(deviceId: string): Promise<MachineImageState | null> {
    try { return imageStateSchema.parse(JSON.parse(await readFile(join(this.device(deviceId), 'current.json'), 'utf8'))) }
    catch (error) { if (errorCode(error) === 'ENOENT') return null; throw error }
  }

  async copy(deviceId: string, state: MachineImageState, destination: string): Promise<void> {
    const from = join(this.device(deviceId), 'generations', safeImageId(state.generation))
    await mkdir(destination, { recursive: true })
    for (const volume of ['system', 'home']) await copyFile(join(from, `${volume}.ext4`), join(destination, `${volume}.ext4`), constants.COPYFILE_FICLONE)
  }

  async publish(deviceId: string, state: MachineImageState, source: string): Promise<void> {
    const parsed = imageStateSchema.parse(state)
    const previous = await this.read(deviceId)
    const device = this.device(deviceId)
    const generations = join(device, 'generations')
    const directory = join(generations, parsed.generation)
    await mkdir(generations, { recursive: true })
    await mkdir(directory)
    for (const volume of ['system', 'home']) {
      const destination = join(directory, `${volume}.ext4`)
      await copyFile(join(source, `${volume}.ext4`), destination, constants.COPYFILE_FICLONE)
      await syncFile(destination)
    }
    await atomicJson(join(directory, 'manifest.json'), parsed)
    await syncFile(directory)
    await syncFile(generations)
    await atomicJson(join(device, 'current.json'), parsed)
    await syncFile(device)
    // Keep one fallback generation; obsolete checkpoints must not grow without bound.
    for (const entry of await readdir(generations, { withFileTypes: true })) {
      if (entry.isDirectory() && entry.name !== parsed.generation && entry.name !== previous?.generation) {
        await rm(join(generations, entry.name), { recursive: true })
      }
    }
    await syncFile(generations)
  }
}
