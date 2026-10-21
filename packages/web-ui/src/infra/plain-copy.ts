/**
 * Returns a deep copy of `value` that holds no reactive proxy at any depth,
 * for state that leaves Vue's reach: stored in the web browser, or kept apart
 * from what it was copied from.
 *
 * `structuredClone(toRaw(value))` is not enough. `toRaw` unwraps only the
 * value itself, and a raw object still holds a proxy wherever a reactive read
 * was put into it, such as `{ files: message.files }` for a reactive
 * `message`. The web browser refuses to clone a proxy, so that copy throws.
 *
 * The copy reads `value` through its proxies, so an effect that makes it,
 * such as a watcher's getter, depends on every value copied.
 */
export function plainCopy<T>(value: T): T {
  return structuredClone(unwrapped(value)) as T
}

/**
 * `value` with each array, map, set and plain object in it rebuilt. Vue
 * wraps only those, so whatever else it holds, such as a `File`, is no proxy.
 */
function unwrapped(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(unwrapped)
  }
  if (value instanceof Map) {
    return new Map([...value].map(([key, each]) => [unwrapped(key), unwrapped(each)]))
  }
  if (value instanceof Set) {
    return new Set([...value].map(unwrapped))
  }
  if (value !== null && typeof value === 'object' && Object.getPrototypeOf(value) === Object.prototype) {
    return Object.fromEntries(Object.entries(value).map(([key, each]) => [key, unwrapped(each)]))
  }
  return value
}
