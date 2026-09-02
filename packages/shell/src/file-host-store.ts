import { decodeUtf8, dirnamePath, encodeUtf8, isFileNotFoundError, normalizePath, parsePortableJson, stringifyPortableJson } from '@demicodes/utils'
import type { HostDirent, HostFileSystem, HostStore } from './host'

/**
 * A `HostStore` kept as JSON files under a directory of a Host's filesystem:
 * one file per key, portable JSON (so `Uint8Array` and `bigint` round-trip),
 * every write a same-directory temp file renamed into place so a reader —
 * another process included — never sees a torn document.
 */
export function fileHostStore(fs: HostFileSystem, root: string): HostStore {
  const pathFor = (key: string): string => {
    validateHostStoreKey(key)
    return key === '' || key === '.' ? root : normalizePath(`${root}/${key}`)
  }
  return {
    async readJson<T>(key: string): Promise<T | null> {
      try {
        return parsePortableJson<T>(decodeUtf8(await fs.readFile(pathFor(key))))
      } catch (error) {
        if (isFileNotFoundError(error)) return null
        throw error
      }
    },
    async writeJson<T>(key: string, value: T): Promise<void> {
      const path = pathFor(key)
      await fs.mkdir(dirnamePath(path), { recursive: true })
      const temp = `${path}.${crypto.randomUUID()}.tmp`
      try {
        await fs.writeFile(temp, encodeUtf8(stringifyPortableJson(value, 2)))
        await fs.mv(temp, path)
      } catch (error) {
        await fs.rm(temp, { force: true })
        throw error
      }
    },
    async delete(key: string): Promise<void> {
      await fs.rm(pathFor(key), { force: true, recursive: true })
    },
    async list(prefix: string): Promise<string[]> {
      const found: string[] = []
      await collectFiles(fs, pathFor(prefix), root, found)
      return found.sort()
    },
  }
}

async function collectFiles(fs: HostFileSystem, dir: string, root: string, found: string[]): Promise<void> {
  let entries: HostDirent[]
  try {
    entries = await fs.readdir(dir, { withFileTypes: true })
  } catch (error) {
    if (isFileNotFoundError(error)) return
    throw error
  }
  for (const entry of entries) {
    const path = `${dir}/${entry.name}`
    if (entry.isDirectory) await collectFiles(fs, path, root, found)
    else found.push(path.slice(root.length + 1))
  }
}

function validateHostStoreKey(key: string): void {
  if (key === '' || key === '.') return
  if (key.includes('\0')) throw new Error(`Invalid HostStore key: ${key}`)
  if (key.startsWith('/') || /^[A-Za-z]:[\\/]/.test(key)) {
    throw new Error(`HostStore keys must be relative: ${key}`)
  }
  for (const segment of key.split(/[\\/]+/)) {
    if (segment === '..') throw new Error(`HostStore keys must not contain path traversal: ${key}`)
  }
}
