import { reactive } from 'vue'

/**
 * The reads a page has under way, by the key of what each reads: what a view
 * shows a loading row by, only while a read lasts. A conversation's history
 * and a share's both track their reads here.
 */
export function createReading() {
  const counts = reactive(new Map<string, number>())
  return {
    /** Runs `work`, which counts as a read of `key` until it settles. */
    async track<T>(key: string, work: () => Promise<T>): Promise<T> {
      counts.set(key, (counts.get(key) ?? 0) + 1)
      try {
        return await work()
      } finally {
        const left = (counts.get(key) ?? 1) - 1
        if (left > 0) {
          counts.set(key, left)
        } else {
          counts.delete(key)
        }
      }
    },
    /** Whether a read of `key` is under way. */
    has: (key: string): boolean => counts.has(key),
  }
}
