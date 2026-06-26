/** Clamps a number into the inclusive `[min, max]` range. */
export function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max)
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
