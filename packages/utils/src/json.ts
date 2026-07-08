import { base64ToBytes, bytesToBase64 } from './bytes'
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

const BINARY_MARKER = '__demiUint8Array'
const BIGINT_MARKER = '__demiBigInt'

/**
 * JSON.stringify that round-trips values plain JSON cannot: `Uint8Array` is
 * encoded as a `__demiUint8Array`-marked base64 object and `bigint` as a
 * `__demiBigInt`-marked string. Decode with `parsePortableJson`.
 */
export function stringifyPortableJson(value: unknown, space?: number): string {
  return JSON.stringify(
    value,
    (_key, nested) => {
      if (nested instanceof Uint8Array) {
        return {
          [BINARY_MARKER]: true,
          base64: bytesToBase64(nested),
        }
      }
      if (typeof nested === 'bigint') {
        return {
          [BIGINT_MARKER]: true,
          value: nested.toString(),
        }
      }
      return nested
    },
    space,
  )
}

/** Parses JSON produced by `stringifyPortableJson`, reviving marked `Uint8Array` and `bigint` values. */
export function parsePortableJson<T>(text: string): T {
  return JSON.parse(text, (_key, nested) => {
    if (isEncodedUint8Array(nested)) {
      return base64ToBytes(nested.base64)
    }
    if (isEncodedBigInt(nested)) {
      return BigInt(nested.value)
    }
    return nested
  }) as T
}

function isEncodedUint8Array(value: unknown): value is { base64: string } {
  return isRecord(value) && value[BINARY_MARKER] === true && typeof value.base64 === 'string'
}

function isEncodedBigInt(value: unknown): value is { value: string } {
  return isRecord(value) && value[BIGINT_MARKER] === true && typeof value.value === 'string'
}
