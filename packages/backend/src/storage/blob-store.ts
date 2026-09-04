import { mkdirSync } from 'node:fs'
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { createId } from '@demicodes/utils'
import type { BlobStore } from '@demicodes/agent'

/**
 * The N=1 blob store: content-addressed files under the supplied root.
 * Writes go through a temp file + rename so a crash never leaves a partial
 * blob under its final name; an existing blob short-circuits (content
 * addressing makes puts idempotent).
 */
export class DirBlobStore implements BlobStore {
  constructor(private readonly root: string) {
    mkdirSync(root, { recursive: true })
  }

  async put(data: Uint8Array): Promise<string> {
    const hasher = new Bun.CryptoHasher('sha256')
    hasher.update(data)
    const sha256 = hasher.digest('hex')
    const path = join(this.root, sha256)
    if (await Bun.file(path).exists()) return sha256
    await mkdir(this.root, { recursive: true })
    const temp = join(this.root, `.tmp-${createId()}`)
    await writeFile(temp, data)
    try {
      await rename(temp, path)
    } catch (error) {
      await rm(temp, { force: true })
      if (!(await Bun.file(path).exists())) throw error
    }
    return sha256
  }

  async get(sha256: string): Promise<Uint8Array | null> {
    if (!/^[0-9a-f]{64}$/.test(sha256)) return null
    try {
      return new Uint8Array(await readFile(join(this.root, sha256)))
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return null
      throw error
    }
  }
}
