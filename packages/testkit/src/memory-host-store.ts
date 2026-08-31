import type { HostStore } from '@demicodes/shell'

/** In-memory `HostStore` for tests (values held by reference, no cloning). */
export function memoryHostStore(): HostStore {
  const map = new Map<string, unknown>()
  return {
    readJson: async <T>(key: string) => (map.has(key) ? (map.get(key) as T) : null),
    writeJson: async (key, value) => {
      map.set(key, value)
    },
    delete: async (key) => {
      map.delete(key)
    },
    list: async (prefix) => [...map.keys()].filter((key) => key.startsWith(prefix)),
  }
}
