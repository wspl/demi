import {
  constants,
  copyFile,
  mkdir,
  open,
  readFile,
  readdir,
  rename,
  rm
} from 'node:fs/promises'
import { join } from 'node:path'
import { z } from 'zod'
import { createId } from '@demicodes/utils'
import { requireTool } from './gvisor/image-tools'
import {
  imageStateSchema,
  type MachineImageState
} from './provisioner'

export function safeImageId(id: string): string {
  return z.string().regex(/^[A-Za-z0-9_-]+$/).parse(id)
}

export async function syncFile(path: string): Promise<void> {
  const file = await open(path, 'r')
  try {
    await file.sync()
  } finally {
    await file.close()
  }
}

/** Clone a Cloud disk image without allocating its empty ranges on non-reflink storage. */
export async function copyMachineImage(source: string, destination: string): Promise<void> {
  if (process.platform === 'linux') {
    // fs.copyFile has no sparse-file policy; Bun's fallback writes every empty range.
    // GNU cp owns both reflink selection and the sparse fallback for Linux storage.
    await requireTool('cp', ['--reflink=auto', '--sparse=always', '--', source, destination], undefined, 300_000)
    return
  }
  await copyFile(source, destination, constants.COPYFILE_FICLONE)
}

export async function atomicJson(path: string, value: unknown): Promise<void> {
  const temp = `${path}.${createId()}`
  try {
    const file = await open(temp, 'wx')
    try {
      await file.writeFile(JSON.stringify(value))
      await file.sync()
    } finally {
      await file.close()
    }
    await rename(temp, path)
  } finally {
    await rm(temp, { force: true })
  }
}

export interface MachineImageStore {
  read(deviceId: string): Promise<MachineImageState | null>
  copy(
    deviceId: string,
    state: MachineImageState,
    destination: string
  ): Promise<void>
  publish(
    deviceId: string,
    state: MachineImageState,
    source: string
  ): Promise<void>
}

/**
 * A generation is immutable. Only the manifest pointer is replaced, after both
 * disks are durable.
 */
export class DirMachineImageStore implements MachineImageStore {
  constructor(private readonly root: string) {}

  private device(deviceId: string): string {
    return join(this.root, safeImageId(deviceId))
  }

  async read(deviceId: string): Promise<MachineImageState | null> {
    try {
      const manifest = await readFile(
        join(this.device(deviceId), 'current.json'),
        'utf8'
      )
      return imageStateSchema.parse(JSON.parse(manifest))
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
        return null
      }
      throw error
    }
  }

  async copy(
    deviceId: string,
    state: MachineImageState,
    destination: string
  ): Promise<void> {
    const from = join(
      this.device(deviceId),
      'generations',
      safeImageId(state.generation)
    )
    await mkdir(destination, { recursive: true })
    for (const volume of ['system', 'home']) {
      await copyMachineImage(
        join(from, `${volume}.ext4`),
        join(destination, `${volume}.ext4`),
      )
    }
  }

  async publish(
    deviceId: string,
    state: MachineImageState,
    source: string
  ): Promise<void> {
    const parsed = imageStateSchema.parse(state)
    const previous = await this.read(deviceId)
    const device = this.device(deviceId)
    const generations = join(device, 'generations')
    const directory = join(generations, parsed.generation)
    await mkdir(generations, { recursive: true })
    await syncFile(this.root)
    await syncFile(device)
    const stage = join(generations, `.publish-${createId()}`)
    await mkdir(stage)
    try {
      for (const volume of ['system', 'home']) {
        const destination = join(stage, `${volume}.ext4`)
        await copyMachineImage(join(source, `${volume}.ext4`), destination)
        await syncFile(destination)
      }
      await atomicJson(join(stage, 'manifest.json'), parsed)
      await syncFile(stage)
      await rename(stage, directory)
    } finally {
      await rm(stage, { recursive: true, force: true })
    }
    await syncFile(generations)
    await atomicJson(join(device, 'current.json'), parsed)
    await syncFile(device)
    // Keep one fallback generation; obsolete checkpoints must not grow without bound.
    for (const entry of await readdir(generations, { withFileTypes: true })) {
      if (entry.isDirectory() &&
        entry.name !== parsed.generation &&
        entry.name !== previous?.generation) {
        await rm(join(generations, entry.name), { recursive: true })
      }
    }
    await syncFile(generations)
  }
}
