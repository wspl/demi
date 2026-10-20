/**
 * A plugin's disk cache (`docs/decisions/0008-plugin-architecture.md`): bytes by key
 * in the page's IndexedDB, kept across reloads up to a size per plugin, the
 * least recently used going first. It is a cache: nothing ties it to whether
 * the plugin is on, and a page shows what it read from it only until the
 * Host's own answer replaces it.
 */
import { z } from 'zod'
import { IndexedDatabase } from '../infra/indexed-db'

/** The most bytes one plugin keeps. */
export const PLUGIN_CACHE_BYTES = 64 * 1024 * 1024

/** One plugin's cache. */
export interface PluginCache {
  /** The bytes kept under `key`, which are the most recently used from now; null when none are. */
  read(key: string): Promise<Uint8Array<ArrayBuffer> | null>
  /** Keeps `bytes` under `key`, letting the least recently used of the plugin's go past its size. */
  write(key: string, bytes: Uint8Array<ArrayBuffer>): Promise<void>
}

const STORE = 'entries'
/** The entries of one plugin, the least recently used first. */
const BY_USE = 'byUse'

const database = new IndexedDatabase('demi-plugin-cache', 1, (request) => {
  const entries = request.result.createObjectStore(STORE, { keyPath: ['plugin', 'key'] })
  entries.createIndex(BY_USE, ['plugin', 'used'])
})

const entrySchema = z.object({
  plugin: z.string(),
  key: z.string(),
  // What the cache wrote: bytes over an ArrayBuffer of their own, as IndexedDB's structured clone keeps them.
  bytes: z.custom<Uint8Array<ArrayBuffer>>((value) => value instanceof Uint8Array && value.buffer instanceof ArrayBuffer),
  used: z.number(),
})
type Entry = z.infer<typeof entrySchema>

/** The cache of `plugin`, of at most `limit` bytes. */
export function pluginCache(plugin: string, limit = PLUGIN_CACHE_BYTES, now: () => number = Date.now): PluginCache {
  return {
    read: (key) =>
      database.across([STORE], 'readwrite', (transaction) => {
        const entries = transaction.objectStore(STORE)
        let found: Entry | null = null
        const request = entries.get([plugin, key])
        request.onsuccess = () => {
          if (request.result === undefined) {
            return
          }
          found = entrySchema.parse(request.result)
          entries.put({ ...found, used: now() })
        }
        return () => found?.bytes ?? null
      }),
    write: (key, bytes) =>
      database.across([STORE], 'readwrite', (transaction) => {
        const entries = transaction.objectStore(STORE)
        entries.put({ plugin, key, bytes, used: now() } satisfies Entry)
        // Then every entry of the plugin, the least recently used first,
        // which the cache lets go of until the rest fits.
        const sizes: { key: IDBValidKey; size: number }[] = []
        const range = IDBKeyRange.bound([plugin, -Infinity], [plugin, Infinity])
        const cursor = entries.index(BY_USE).openCursor(range)
        cursor.onsuccess = () => {
          const at = cursor.result
          if (at) {
            sizes.push({ key: at.primaryKey, size: entrySchema.parse(at.value).bytes.byteLength })
            at.continue()
            return
          }
          let total = sizes.reduce((sum, entry) => sum + entry.size, 0)
          for (const entry of sizes) {
            if (total <= limit) {
              break
            }
            entries.delete(entry.key)
            total -= entry.size
          }
        }
        return () => undefined
      }),
  }
}
