/**
 * What a browser's address bar opens for the text its user typed, as Chrome's
 * omnibox reads it:
 *
 * - A URL with its scheme opens as it is.
 * - A local name, an IP address or a `host:port` without a scheme opens over
 *   `http://`.
 * - Other text with a dot and no space opens over `https://`.
 * - Anything else, a word or words, opens a Google search.
 */

/** Schemes a typed address names without `//`. */
const OPAQUE_SCHEMES = /^(about|data|file|blob|view-source):/i
/** An IPv4 address, or an IPv6 one in brackets. */
const IP = /^(\d{1,3}(\.\d{1,3}){3}|\[[0-9a-f:.]+\])$/i

/**
 * The URL the address bar opens for `text`; null for text that names nothing.
 */
export function addressUrl(text: string): string | null {
  const typed = text.trim()
  if (!typed) {
    return null
  }
  if (typed.includes('://') || OPAQUE_SCHEMES.test(typed)) {
    return URL.parse(typed)?.href ?? search(typed)
  }
  if (/\s/.test(typed)) {
    return search(typed)
  }
  const authority = typed.split(/[/?#]/, 1)[0]
  const port = /:\d{1,5}$/.exec(authority)
  const host = (port ? authority.slice(0, port.index) : authority).toLowerCase()
  const local = host === 'localhost' || host.endsWith('.localhost') || IP.test(host)
  if (local || (port && host)) {
    return URL.parse(`http://${typed}`)?.href ?? search(typed)
  }
  if (host.includes('.')) {
    return URL.parse(`https://${typed}`)?.href ?? search(typed)
  }
  return search(typed)
}

function search(text: string): string {
  return `https://www.google.com/search?q=${encodeURIComponent(text)}`
}
