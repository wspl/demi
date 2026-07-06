import { isRecord } from './guards'

/** Parses JSON, returning the value, or the original string if it is not valid JSON. */
export function parseJsonOrString(value: string): unknown {
  try {
    return JSON.parse(value)
  } catch {
    return value
  }
}

/** Parses JSON and returns it only when it is a plain object; otherwise `null`. */
export function parseJsonObject(value: string): Record<string, unknown> | null {
  try {
    const parsed: unknown = JSON.parse(value)
    return isRecord(parsed) ? parsed : null
  } catch {
    return null
  }
}

/**
 * JSON.stringify tolerant of values that normally throw or vanish: bigints,
 * symbols, and functions are rendered as strings, circular references as
 * "[Circular]". Returns undefined when the top-level value has no JSON form.
 */
export function safeJsonStringify(value: unknown): string | undefined {
  const seen = new WeakSet<object>()
  try {
    return JSON.stringify(value, (_key, nested) => {
      if (typeof nested === 'bigint') return nested.toString()
      if (typeof nested === 'symbol') return String(nested)
      if (typeof nested === 'function') return `[Function ${nested.name || 'anonymous'}]`
      if (nested !== null && typeof nested === 'object') {
        if (seen.has(nested)) return '[Circular]'
        seen.add(nested)
      }
      return nested
    })
  } catch {
    return undefined
  }
}
