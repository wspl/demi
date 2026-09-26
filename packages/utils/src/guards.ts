/** Returns the value when it is a non-blank string, otherwise `undefined`. */
export function nonEmptyString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value : undefined
}

/** Returns the value when it is a finite number, otherwise `null`. */
export function numberOrNull(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}
