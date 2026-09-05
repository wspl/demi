import { constants, copyFile, mkdir, rename, rm, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { createId } from '@demicodes/utils'

/**
 * The home-image store (`storage.md`): one named, mutable, owner-bound
 * object per owner — the managed host's home as an ext4 image — with one
 * current version, replaced whole on hibernate and checkpoint. Not the
 * blob store: no content addressing, no history. Files in and out, never
 * bytes in the heap: an image is gigabytes.
 */
export interface HomeImageStore {
  has(ownerKey: string): Promise<boolean>
  /** Makes the file at `path` the owner's current image, atomically replacing the previous one; the file is consumed. */
  put(ownerKey: string, path: string): Promise<void>
  /** A working copy of the owner's current image at `path` — a reflink where the filesystem offers one, a copy otherwise. */
  get(ownerKey: string, path: string): Promise<void>
  delete(ownerKey: string): Promise<void>
}

/** The N=1 store: `homes/<ownerKey>.ext4` under the data directory; `put` renames into place, so the file must be on the same filesystem. */
export class DirHomeImageStore implements HomeImageStore {
  constructor(private readonly root: string) {}

  imagePath(ownerKey: string): string {
    return join(this.root, `${ownerKey}.ext4`)
  }

  async has(ownerKey: string): Promise<boolean> {
    return stat(this.imagePath(ownerKey)).then(
      () => true,
      () => false,
    )
  }

  async put(ownerKey: string, path: string): Promise<void> {
    await mkdir(this.root, { recursive: true })
    await rename(path, this.imagePath(ownerKey))
  }

  async get(ownerKey: string, path: string): Promise<void> {
    const temp = `${path}.${createId()}`
    await copyFile(this.imagePath(ownerKey), temp, constants.COPYFILE_FICLONE)
    try {
      await rename(temp, path)
    } catch (error) {
      await rm(temp, { force: true })
      throw error
    }
  }

  async delete(ownerKey: string): Promise<void> {
    await rm(this.imagePath(ownerKey), { force: true })
  }
}
