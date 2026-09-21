import { mkdir, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { copyMachineImage } from '../machine-image-store'
import type { GVisorConfig } from './config'
import { makeSystemImage, mountSystemOverlay, requireTool, runTool, thawFilesystem } from './image-tools'

/** Verify the host storage primitives before admitting any Cloud workload. */
export async function verifyStorage(config: GVisorConfig): Promise<void> {
  const stage = await mkdtemp(join(config.dataDir, '.preflight-'))
  const mount = join(stage, 'volume')
  const root = join(stage, 'merged')
  try {
    await makeSystemImage(join(stage, 'volume.ext4'), 32 * 1024 ** 2)
    await mkdir(mount)
    await mkdir(root)
    await mkdir(join(stage, 'base'))
    await requireTool('mount', ['-o', 'loop,nodev', join(stage, 'volume.ext4'), mount])
    await mountSystemOverlay(join(stage, 'base'), mount, root)
    await writeFile(join(root, 'probe'), 'Cloud storage preflight')
    await requireTool('fsfreeze', ['--freeze', mount])
    await copyMachineImage(join(stage, 'volume.ext4'), join(stage, 'copy.ext4'))
  } finally {
    await releaseProbe(stage)
  }
}

/** Release the mounts of a Cloud storage probe before deleting its image. */
async function releaseProbe(stage: string): Promise<void> {
  const volume = join(stage, 'volume')
  const mounted = await runTool('mountpoint', ['-q', volume])
  if (mounted.code === 0) await thawFilesystem(volume)
  for (const name of ['merged', 'volume']) {
    const path = join(stage, name)
    const result = await runTool('mountpoint', ['-q', path])
    if (result.code === 0) await requireTool('umount', [path])
    else if (result.code !== 32 && !(result.code === 1 && result.stderr.includes('No such file'))) {
      throw new Error(`Cannot inspect Cloud storage probe: ${result.stderr}`)
    }
  }
  await rm(stage, { recursive: true, force: true })
}

/** Recover a manager interrupted while testing storage, inside its saved namespace. */
export async function recoverStorageProbes(config: GVisorConfig): Promise<void> {
  for (const name of await readdir(config.dataDir)) {
    if (name.startsWith('.preflight-')) await releaseProbe(join(config.dataDir, name))
  }
}
