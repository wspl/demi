/** Returns the value when it is a non-blank string, otherwise `undefined`. */
export function nonEmptyString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value : undefined
}

/** Returns the value when it is a finite number, otherwise `null`. */
export function numberOrNull(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

/** `value` read as JSON, or null when it is not JSON, for a value from outside that a schema decodes next. */
export function jsonOrNull(value: string): unknown {
  try {
    return JSON.parse(value)
  } catch {
    // Not JSON: the schema that reads the null rejects it as any wrong value.
    return null
  }
}
