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
