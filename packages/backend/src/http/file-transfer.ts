// Serving a file's bytes to the browser (`web-api.md` § File text and working
// tree changes, `file-previews.md` § Getting the bytes): which headers say
// how the page may treat them, which part a `Range` asks for, and a body the
// browser paces.
import { showsInPlace } from '@demicodes/core'
import { IdleTimer, deferred, noop } from '@demicodes/utils'

/**
 * How long a transfer waits for the browser to take more bytes before its
 * connection is reset (`sessions-and-targets.md` § Host operations).
 */
export const TRANSFER_STALL_MS = 60_000

/** An image served in place runs no script and fetches nothing, in an opaque origin. */
const IMAGE_POLICY = "default-src 'none'; style-src 'unsafe-inline'; sandbox"

/**
 * How a file's bytes are served (`file-previews.md` § Keeping file content
 * inert): a media type the page shows in place as itself, an image under a
 * policy that keeps an SVG inert; anything else, and anything asked for as a
 * download, as an attachment named `fileName` when there is a name.
 */
export function contentHeaders(
  mediaType: string | null,
  options: { download?: boolean; fileName?: string } = {},
): Record<string, string> {
  if (mediaType !== null && showsInPlace(mediaType) && !options.download) {
    return {
      'content-type': mediaType,
      ...(mediaType.startsWith('image/') ? { 'content-security-policy': IMAGE_POLICY } : {}),
      'x-content-type-options': 'nosniff',
    }
  }
  return {
    'content-type': 'application/octet-stream',
    'content-disposition': attachment(options.fileName),
    'x-content-type-options': 'nosniff',
  }
}

/**
 * `attachment` naming the file for the browser's download (RFC 6266): an
 * ASCII fallback, and the exact name in RFC 8187 form.
 */
function attachment(fileName?: string): string {
  if (!fileName)
    return 'attachment'
  const fallback = fileName.replace(/[^\x20-\x7e]|["\\]/g, '_')
  // encodeURIComponent leaves ' ( ) * as they are; RFC 8187 needs them encoded.
  const encoded = encodeURIComponent(fileName)
    .replace(/['()*]/g, (char) => `%${char.charCodeAt(0).toString(16).toUpperCase()}`)
  return `attachment; filename="${fallback}"; filename*=UTF-8''${encoded}`
}

/** The part of `size` bytes an answer sends, and the headers that describe it. */
export type RangeAnswer =
  | { status: 200 | 206; start: number; length: number; headers: Record<string, string> }
  | { status: 416; headers: Record<string, string> }

/**
 * What a request for `size` bytes answers, given its `Range` header (RFC 9110
 * § 14): one range, `bytes=a-b`, `bytes=a-` or the suffix `bytes=-n`, clamped
 * to the end, answers 206; one that starts past the end answers 416. No
 * header, one that does not parse, or several ranges answer the whole with
 * 200.
 */
export function rangeAnswer(header: string | undefined, size: number): RangeAnswer {
  const whole: RangeAnswer = {
    status: 200,
    start: 0,
    length: size,
    headers: { 'accept-ranges': 'bytes', 'content-length': String(size) },
  }
  const match = header?.trim().match(/^bytes=(\d*)-(\d*)$/)
  if (!match || (match[1] === '' && match[2] === ''))
    return whole
  const [first, last] = [match[1]!, match[2]!]
  let start: number
  let end: number
  if (first === '') {
    start = Math.max(0, size - Number(last))
    end = size - 1
    if (Number(last) === 0)
      return unsatisfiable(size)
  } else {
    start = Number(first)
    end = last === '' ? size - 1 : Math.min(Number(last), size - 1)
    // A last byte before the first is no range at all.
    if (last !== '' && Number(last) < start)
      return whole
  }
  if (start >= size)
    return unsatisfiable(size)
  return {
    status: 206,
    start,
    length: end - start + 1,
    headers: {
      'accept-ranges': 'bytes',
      'content-length': String(end - start + 1),
      'content-range': `bytes ${start}-${end}/${size}`,
    },
  }
}

function unsatisfiable(size: number): RangeAnswer {
  return { status: 416, headers: { 'accept-ranges': 'bytes', 'content-range': `bytes */${size}` } }
}

/**
 * A response body the browser paces: the next chunk is taken from `bytes`
 * only when the browser asks for more, and time spent waiting for `bytes`
 * does not count against the browser.
 *
 * A transfer that goes wrong is cut short, never ended: a browser that asks
 * for nothing for `stallMs`, `signal` ending the transfer from outside (an
 * archive, say), or `bytes` failing. The source stops, `finished` settles so
 * the transfer releases its Host access, the body sends nothing more and no
 * end, and `cutShort` asks the server to close the connection once it is
 * idle. The browser then sees an incomplete response, and a player asks again
 * for the range it still needs. `finished` settles once the body is over,
 * however it ended.
 */
export function pacedBody(
  bytes: AsyncIterable<Uint8Array>,
  options: { stallMs: number; signal: AbortSignal; cutShort: () => void },
): { body: ReadableStream<Uint8Array>; finished: Promise<void> } {
  const { signal } = options
  const chunks = bytes[Symbol.asyncIterator]()
  const over = deferred<void>()
  const cancelled = deferred<void>()
  let halted = false
  // Returning stops the read at its source. Its own failure is the
  // transfer's to report, not this body's, which is over either way.
  const stopReading = () => void chunks.return?.()?.catch(noop)
  const finish = () => {
    stall.close()
    signal.removeEventListener('abort', halt)
    over.resolve()
  }
  // An errored body would only reach the browser when the server next reads
  // it; a body that goes quiet and a closed connection always do.
  function halt(): void {
    if (halted)
      return
    halted = true
    stopReading()
    finish()
    options.cutShort()
  }
  const stall = new IdleTimer(options.stallMs, halt)
  const body = new ReadableStream<Uint8Array>({
    pull: async (controller) => {
      // Cut short: nothing more comes, and the pull waits for the close.
      if (halted)
        return cancelled.promise
      stall.close()
      try {
        const next = await chunks.next()
        // Once cut short, nothing more goes out, least of all an end.
        if (halted)
          return cancelled.promise
        if (next.done) {
          controller.close()
          finish()
          return
        }
        controller.enqueue(next.value)
        stall.touch()
      } catch {
        // The device went away or the transfer was ended: the browser learns
        // it as a response cut short, and the pipe's failure was reported
        // where it happened.
        halt()
      }
    },
    cancel: () => {
      // The browser went away. A read still pending ends through the signal
      // the transfer handed its source.
      cancelled.resolve()
      stopReading()
      finish()
    },
  }, { highWaterMark: 0 })
  signal.addEventListener('abort', halt, { once: true })
  if (signal.aborted)
    halt()
  return { body, finished: over.promise }
}
