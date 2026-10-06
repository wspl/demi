/**
 * A raw file answer as the relay's route gives it (`web-api.md` § File text
 * and working tree changes), for the bytes the service worker serves over a
 * direct channel (`direct-channel.md` § Bytes the browser fetches itself):
 * the part a `Range` asks for, and the headers that keep the bytes inert,
 * name their version and say how long they may be kept.
 */
import { IMAGE_CONTENT_POLICY, previewMediaType, showsInPlace } from '@demicodes/protocol'

/** The part of a file of `size` bytes an answer sends (RFC 9110 § 14), as the relay's `RangeAnswer`. */
export type RangeAnswer =
  /** No range, one that does not parse, several, or a reversed one: the whole file, 200. */
  | { kind: 'whole'; size: number }
  /** One satisfiable range, clamped to the end: 206. */
  | { kind: 'part'; start: number; length: number; size: number }
  /** A range that starts past the end, or an empty suffix: 416. */
  | { kind: 'unsatisfiable'; size: number }

/**
 * What a request for `size` bytes answers, given its `Range` header:
 * `bytes=a-b`, `bytes=a-` or the suffix `bytes=-n`.
 */
export function rangeAnswer(header: string | null, size: number): RangeAnswer {
  const whole: RangeAnswer = { kind: 'whole', size }
  const spec = header?.trim()
  if (!spec?.startsWith('bytes='))
    return whole
  const value = spec.slice('bytes='.length)
  const dash = value.indexOf('-')
  if (dash < 0)
    return whole
  const first = value.slice(0, dash)
  const last = value.slice(dash + 1)
  const digits = (text: string) => /^\d*$/.test(text)
  if (!digits(first) || !digits(last) || (first === '' && last === ''))
    return whole
  let start: number
  let end: number | null
  if (first === '') {
    const suffix = Number(last)
    if (suffix === 0)
      return { kind: 'unsatisfiable', size }
    start = Math.max(size - suffix, 0)
    end = size > 0 ? size - 1 : null
  } else {
    start = Number(first)
    if (last === '') {
      end = size > 0 ? size - 1 : null
    } else {
      const lastByte = Number(last)
      // A last byte before the first is no range at all.
      if (lastByte < start)
        return whole
      end = Math.min(lastByte, Math.max(size - 1, 0))
    }
  }
  if (end !== null && start < size)
    return { kind: 'part', start, length: end - start + 1, size }
  return { kind: 'unsatisfiable', size }
}

/** The status an answer of `part` carries. */
export function rangeStatus(part: RangeAnswer): number {
  return part.kind === 'whole' ? 200 : part.kind === 'part' ? 206 : 416
}

/** The bytes an answer of `part` sends; none for a refusal. */
export function rangeBytes(part: RangeAnswer): { offset: number; length: number } | null {
  if (part.kind === 'whole')
    return { offset: 0, length: part.size }
  if (part.kind === 'part')
    return { offset: part.start, length: part.length }
  return null
}

/**
 * Whether `If-None-Match` names `etag`, compared weakly (RFC 9110 § 13.1.2):
 * the tags are equal once a `W/` prefix is set aside, and `*` names every
 * version.
 */
export function notModified(ifNoneMatch: string | null, etag: string): boolean {
  if (ifNoneMatch === null)
    return false
  const strip = (tag: string) => tag.trim().replace(/^(W\/)+/, '')
  return ifNoneMatch.trim() === '*' || ifNoneMatch.split(',').some((tag) => strip(tag) === strip(etag))
}

/** The file's name, which a download is saved under: the last part of `path`, whichever separator ends a directory. */
export function fileName(path: string): string | null {
  const name = path.slice(Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\')) + 1)
  return name === '' ? null : name
}

/**
 * `attachment` naming the file (RFC 6266): an ASCII fallback, in which
 * anything but printable ASCII and the quote and backslash become `_`, and
 * the exact name in RFC 8187 form.
 */
function attachment(name: string | null): string {
  if (!name)
    return 'attachment'
  const fallback = [...name]
    .map((char) => (char === '"' || char === '\\' || char < ' ' || char > '~' ? '_' : char))
    .join('')
  const encoded = encodeURIComponent(name).replace(
    /['()*]/g,
    (char) => `%${char.charCodeAt(0).toString(16).toUpperCase()}`,
  )
  return `attachment; filename="${fallback}"; filename*=UTF-8''${encoded}`
}

/**
 * The headers that serve the bytes of `path`: the media type the page shows
 * in place unless a download is asked for, an image under the policy that
 * keeps it inert, and anything else as a download named after the file.
 */
export function contentHeaders(path: string, download: boolean): Record<string, string> {
  const headers: Record<string, string> = { 'x-content-type-options': 'nosniff' }
  const mediaType = previewMediaType(path)
  if (mediaType !== null && !download && showsInPlace(mediaType)) {
    if (mediaType.startsWith('image/'))
      headers['content-security-policy'] = IMAGE_CONTENT_POLICY
    headers['content-type'] = mediaType
  } else {
    headers['content-type'] = 'application/octet-stream'
    headers['content-disposition'] = attachment(fileName(path))
  }
  return headers
}

/**
 * The headers every raw answer carries: private to the signed-in user,
 * revalidated on each use unless the request named the `versioned` bytes,
 * which can only be that version's.
 */
export function rawFileHeaders(versioned: boolean): Record<string, string> {
  return {
    'cache-control': versioned ? 'private, max-age=31536000, immutable' : 'private, no-cache',
    vary: 'Cookie',
    'x-accel-buffering': 'no',
  }
}

/** The headers that describe an answer's part; a streamed body's length is left out, as the relay leaves it. */
export function rangeHeaders(part: RangeAnswer): Record<string, string> {
  const headers: Record<string, string> = { 'accept-ranges': 'bytes' }
  if (part.kind === 'part')
    headers['content-range'] = `bytes ${part.start}-${part.start + part.length - 1}/${part.size}`
  if (part.kind === 'unsatisfiable')
    headers['content-range'] = `bytes */${part.size}`
  return headers
}

/** What a raw request asks for, as its URL and headers say. */
export interface RawRequest {
  path: string
  /** The version the URL names, whose bytes alone it may get. */
  version: string | null
  download: boolean
  range: string | null
  ifNoneMatch: string | null
}

/** What the file is, as the direct channel's read answered. */
export interface RawFile {
  size: number
  version: string
}

/** An answer to a raw request: its status, its headers, and the bytes it sends. */
export interface RawAnswer {
  status: number
  headers: Record<string, string>
  bytes: { offset: number; length: number } | null
}

/**
 * The relay's answer to `request` for `file`: 304 for a version the page
 * holds, 416 for a range past the end, and otherwise the whole file or its
 * part, with the file's headers. A version the URL names that the file no
 * longer has answers 412 before this, from the channel's refusal.
 */
export function rawAnswer(request: RawRequest, file: RawFile): RawAnswer {
  const versioned = request.version !== null
  if (notModified(request.ifNoneMatch, file.version)) {
    return { status: 304, headers: { ...rawFileHeaders(versioned), etag: file.version }, bytes: null }
  }
  const part = rangeAnswer(request.range, file.size)
  const headers = {
    ...rawFileHeaders(versioned),
    ...contentHeaders(request.path, request.download),
    etag: file.version,
    ...rangeHeaders(part),
  }
  const bytes = rangeBytes(part)
  return {
    status: rangeStatus(part),
    headers,
    bytes: bytes && bytes.length > 0 ? bytes : null,
  }
}
