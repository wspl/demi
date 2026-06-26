/** Narrows an unknown value to a plain object record (not null, not an array). */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

/** Asserts that a value is a plain object record, throwing `TypeError(message)` otherwise. */
export function asRecord(value: unknown, message = 'Expected an object'): Record<string, unknown> {
  if (!isRecord(value)) throw new TypeError(message)
  return value
}

/** Returns the value when it is a string, otherwise `undefined`. */
export function asString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

/** Returns the value when it is a string, otherwise `null`. */
export function stringOrNull(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}

/** Returns the value when it is a non-blank string, otherwise `undefined`. */
export function nonEmptyString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value : undefined
}

/** Returns the value when it is a finite number, otherwise `0`. */
export function numberOrZero(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

/** Returns the value when it is a finite number, otherwise `null`. */
export function numberOrNull(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}
