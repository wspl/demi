/** Clamps a number into the inclusive `[min, max]` range. */
export function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max)
}

/** Strips trailing slashes from a base URL so paths can be appended consistently. */
export function normalizeBaseUrl(baseUrl: string): string {
  return baseUrl.replace(/\/+$/, '')
}

/**
 * Slices the first `maxChars` UTF-16 units of `text` without splitting a
 * surrogate pair: a cut that would leave a trailing lone high surrogate moves
 * back one unit instead.
 */
export function sliceHead(text: string, maxChars: number): string {
  if (maxChars <= 0) return ''
  if (text.length <= maxChars) return text
  const cut = text.charCodeAt(maxChars - 1)
  return text.slice(0, cut >= 0xd800 && cut <= 0xdbff ? maxChars - 1 : maxChars)
}

/**
 * Slices the last `maxChars` UTF-16 units of `text` without splitting a
 * surrogate pair: a cut that would leave a leading lone low surrogate moves
 * forward one unit instead.
 */
export function sliceTail(text: string, maxChars: number): string {
  if (maxChars <= 0) return ''
  if (text.length <= maxChars) return text
  const start = text.length - maxChars
  const cut = text.charCodeAt(start)
  return text.slice(cut >= 0xdc00 && cut <= 0xdfff ? start + 1 : start)
}

const LONE_SURROGATE = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/g

/**
 * Replaces lone UTF-16 surrogates with U+FFFD so the string is well-formed
 * Unicode and can be serialized into any wire payload.
 */
export function toWellFormedText(text: string): string {
  return text.replace(LONE_SURROGATE, '�')
}

/** Truncates `text` to at most `maxChars` characters, appending `ellipsis` when shortened. */
export function truncate(text: string, maxChars: number, ellipsis = '…'): string {
  if (text.length <= maxChars) return text
  if (maxChars <= ellipsis.length) return sliceHead(text, maxChars)
  return sliceHead(text, maxChars - ellipsis.length) + ellipsis
}

/** Returns the last `maxChars` characters of `text`. */
export function tail(text: string, maxChars: number): string {
  return sliceTail(text, maxChars)
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
