/** Clamps a number into the inclusive `[min, max]` range. */
export function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max)
}

/**
 * Slices the first `maxChars` UTF-16 units of `text` without splitting a
 * surrogate pair: a cut that would leave a trailing lone high surrogate moves
 * back one unit instead.
 */
export function sliceHead(text: string, maxChars: number): string {
  if (maxChars <= 0)
    return ''
  if (text.length <= maxChars)
    return text
  const cut = text.charCodeAt(maxChars - 1)
  return text.slice(0, cut >= 0xd800 && cut <= 0xdbff ? maxChars - 1 : maxChars)
}

/**
 * Truncates `text` to at most `maxChars` characters, appending `ellipsis` when
 * shortened.
 */
export function truncate(text: string, maxChars: number, ellipsis = '…'): string {
  if (text.length <= maxChars)
    return text
  if (maxChars <= ellipsis.length)
    return sliceHead(text, maxChars)
  return sliceHead(text, maxChars - ellipsis.length) + ellipsis
}
