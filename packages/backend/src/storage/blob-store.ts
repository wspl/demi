import { mkdirSync } from 'node:fs'
import { mkdir, readFile, rename, rm, stat, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { createId, isFileNotFoundError } from '@demicodes/utils'
import { blobKeySchema, type BlobStore } from '@demicodes/agent'

/** Content-addressed files, atomically replaced and checked on read. */
export class DirBlobStore implements BlobStore {
  constructor(private readonly root: string) {
    mkdirSync(root, { recursive: true })
  }

  async put(data: Uint8Array): Promise<string> {
    const sha256 = digest(data)
    const path = join(this.root, sha256)
    try {
      if (!(await stat(path)).isFile()) throw new Error('Corrupt blob storage entry')
      return sha256
    } catch (error) {
      if (!isFileNotFoundError(error)) throw error
    }
    await mkdir(this.root, { recursive: true })
    const temp = join(this.root, `.tmp-${createId()}`)
    try {
      await writeFile(temp, data)
      await rename(temp, path)
    } catch (error) {
      try {
        await rm(temp, { force: true })
      } catch (cleanupError) {
        throw new AggregateError([error, cleanupError], 'Blob write and cleanup failed')
      }
      throw error
    }
    return sha256
  }

  async get(key: string): Promise<Uint8Array | null> {
    const sha256 = blobKeySchema.parse(key)
    try {
      const bytes = new Uint8Array(await readFile(join(this.root, sha256)))
      if (digest(bytes) !== sha256) throw new Error('Corrupt blob content digest')
      return bytes
    } catch (error) {
      if (isFileNotFoundError(error)) return null
      throw error
    }
  }
}

function digest(bytes: Uint8Array): string {
  return new Bun.CryptoHasher('sha256').update(bytes).digest('hex')
}
