/** Clamps a number into the inclusive `[min, max]` range. */
export function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max)
}

/** `text` with its first letter capitalized, for a word or phrase that starts a label: "in 7 minutes" as "In 7 minutes". */
export function upperFirst(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
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

/**
 * The form of a noun that a count asks for: `pluralize(1, 'file')` is `file`,
 * `pluralize(3, 'file')` is `files`. The caller gives the word in the casing
 * the text needs (`'Conversation'` in a title, `'file'` in a sentence) and
 * the plural form when adding `s` is not it (`pluralize(2, 'copy', 'copies')`).
 * Any phrase works as the noun: `pluralize(2, 'This project goes', 'These projects go')`.
 */
export function pluralize(count: number, singular: string, plural = `${singular}s`): string {
  return count === 1 ? singular : plural
}

/** A count with its noun, `1 file` or `3 files`, by the same rules as [`pluralize`]. */
export function countOf(count: number, singular: string, plural?: string): string {
  return `${count} ${pluralize(count, singular, plural)}`
}

/** A count with its noun the way a sentence says it, the number left out for one: `model`, `3 models`. */
export function countUnlessOne(count: number, singular: string, plural?: string): string {
  return count === 1 ? singular : countOf(count, singular, plural)
}

/**
 * `template` with each `{name}` among `values` replaced by its value, as a
 * catalogue's text is filled where no catalogue runs, such as in a service
 * worker: `fillText('Reach {host}?', { host: 'build-box' })` is `Reach
 * build-box?`. A name `values` lacks stays as it is.
 */
export function fillText(template: string, values: Readonly<Record<string, string>>): string {
  return template.replace(/\{(\w+)\}/g, (whole, name: string) => values[name] ?? whole)
}
