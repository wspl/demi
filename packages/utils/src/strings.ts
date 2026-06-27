/** Clamps a number into the inclusive `[min, max]` range. */
export function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max)
}

/** Strips trailing slashes from a base URL so paths can be appended consistently. */
export function normalizeBaseUrl(baseUrl: string): string {
  return baseUrl.replace(/\/+$/, '')
}

/** Truncates `text` to at most `maxChars` characters, appending `ellipsis` when shortened. */
export function truncate(text: string, maxChars: number, ellipsis = '…'): string {
  if (text.length <= maxChars) return text
  if (maxChars <= ellipsis.length) return text.slice(0, Math.max(0, maxChars))
  return text.slice(0, maxChars - ellipsis.length) + ellipsis
}

/** Returns the last `maxChars` characters of `text`. */
export function tail(text: string, maxChars: number): string {
  return text.length <= maxChars ? text : text.slice(text.length - maxChars)
}

/** A short, stable hex hash of a string (32-bit FNV-1a). Not cryptographic. */
export function shortHash(value: string): string {
  let hash = 2166136261
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i)
    hash = Math.imul(hash, 16777619)
  }
  return (hash >>> 0).toString(16)
}
