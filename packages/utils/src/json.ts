import { base64ToBytes, bytesToBase64 } from './bytes'
import { isRecord } from './guards'

/** Values preserved by Demi's transport-neutral portable JSON codec. */
export type PortableJsonValue =
  | null
  | boolean
  | number
  | string
  | bigint
  | Uint8Array
  | Date
  | readonly PortableJsonValue[]
  | { readonly [key: string]: PortableJsonValue }

/**
 * Parses JSON, returning the value, or the original string if it is not valid
 * JSON.
 */
export function parseJsonOrString(value: string): unknown {
  try {
    return JSON.parse(value)
  } catch {
    return value
  }
}

/**
 * Parses JSON and returns it only when it is a plain object; otherwise `null`.
 */
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
      if (typeof nested === 'bigint')
        return nested.toString()
      if (typeof nested === 'symbol')
        return String(nested)
      if (typeof nested === 'function')
        return `[Function ${nested.name || 'anonymous'}]`
      if (nested !== null && typeof nested === 'object') {
        if (seen.has(nested))
          return '[Circular]'
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
const DATE_MARKER = '__demiDate'

/**
 * JSON.stringify that round-trips values plain JSON cannot: `Uint8Array` is
 * encoded as a `__demiUint8Array`-marked base64 object, `bigint` as a
 * `__demiBigInt`-marked string, and `Date` as a `__demiDate`-marked ISO
 * string. Decode with `parsePortableJson`.
 */
export function stringifyPortableJson(value: unknown, space?: number): string {
  const encoded = JSON.stringify(
    value,
    function (this: unknown, key, nested) {
      // toJSON (Date, Node byte arrays) runs before the replacer, so these
      // must be recovered from the holder object rather than from `nested`.
      const original = isRecord(this) || Array.isArray(this)
        ? (this as Record<string, unknown>)[key]
        : undefined
      if (isRecord(original) && [BINARY_MARKER, BIGINT_MARKER, DATE_MARKER]
        .some((marker) => marker in original)) {
        throw new Error('Portable JSON marker keys are reserved')
      }
      if (typeof original === 'function' || typeof original === 'symbol'
        || (Array.isArray(this) && original === undefined))
        throw new Error('Unsupported portable JSON value')
      if (typeof nested === 'number' && !Number.isFinite(nested))
        throw new Error('Portable JSON numbers must be finite')
      if (original instanceof Date) {
        return {
          [DATE_MARKER]: true,
          iso: original.toISOString(),
        }
      }
      if (original instanceof Uint8Array || nested instanceof Uint8Array) {
        return {
          [BINARY_MARKER]: true,
          base64: bytesToBase64(original instanceof Uint8Array
            ? original
            : (nested as Uint8Array)),
        }
      }
      if (isRecord(original) && (
        ![Object.prototype, null].includes(Object.getPrototypeOf(original))
        || typeof original.toJSON === 'function'
      ))
        throw new Error('Unsupported portable JSON object')
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
  if (encoded === undefined)
    throw new Error('Unsupported portable JSON root value')
  return encoded
}

/**
 * Parses JSON produced by `stringifyPortableJson`, reviving marked
 * `Uint8Array`, `bigint`, and `Date` values.
 */
export function parsePortableJson(text: string): unknown {
  try {
    return JSON.parse(text, (_key, nested) => {
      if (typeof nested === 'number' && !Number.isFinite(nested))
        throw new Error('Portable JSON numbers must be finite')
      if (!isRecord(nested))
        return nested
      if (BINARY_MARKER in nested) {
        const encoded = markerPayload(nested, BINARY_MARKER, 'base64')
        const bytes = base64ToBytes(encoded)
        if (bytesToBase64(bytes) !== encoded)
          throw new Error('Invalid portable JSON binary marker')
        return bytes
      }
      if (BIGINT_MARKER in nested) {
        const encoded = markerPayload(nested, BIGINT_MARKER, 'value')
        if (!/^(?:0|-?[1-9][0-9]*)$/.test(encoded))
          throw new Error('Invalid portable JSON bigint marker')
        return BigInt(encoded)
      }
      if (DATE_MARKER in nested) {
        const encoded = markerPayload(nested, DATE_MARKER, 'iso')
        const date = new Date(encoded)
        if (!Number.isFinite(date.getTime()) || date.toISOString() !== encoded)
          throw new Error('Invalid portable JSON date marker')
        return date
      }
      return nested
    })
  } catch (error) {
    if (error instanceof SyntaxError)
      throw new Error('Invalid portable JSON syntax')
    throw error
  }
}

/** Marker keys are reserved; malformed marked objects cannot become user data. */
function markerPayload(
  value: Record<string, unknown>,
  marker: string,
  field: string,
): string {
  const payload = value[field]
  if (value[marker] !== true || typeof payload !== 'string'
    || Object.keys(value).length !== 2) {
    throw new Error('Invalid portable JSON marker')
  }
  return payload
}
