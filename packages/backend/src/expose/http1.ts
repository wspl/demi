// HTTP/1.1 on the relay's byte stream (`expose.md` § The public relay): the
// request is serialized by hand, the response parsed as it arrives — bodies
// are never buffered whole, so chunked and event-stream responses reach the
// visitor as the service sends them.

const CRLF = '\r\n'
const HEAD_END = new Uint8Array([13, 10, 13, 10])

export interface ResponseHead {
  status: number
  headers: Array<[string, string]>
}

/**
 * One HTTP/1.1 response, fed the stream's bytes. The head may be preceded by
 * interim 1xx responses, which are skipped; the body is delivered chunk by
 * chunk under content-length, chunked coding, or read-until-close.
 */
export class HttpResponseParser {
  private buffer: Uint8Array = new Uint8Array(0)
  private head: ResponseHead | null = null
  private mode: 'none' | 'chunked' | 'length' | 'eof' = 'none'
  private remaining = 0
  private done = false

  constructor(
    private readonly onHead: (head: ResponseHead) => void,
    private readonly onBody: (chunk: Uint8Array) => void,
    private readonly onDone: () => void
  ) {}

  feed(data: Uint8Array): void {
    if (this.done)
      return
    this.buffer = this.buffer.length === 0
      ? data
      : concat(this.buffer, data)
    if (!this.head) {
      const headEnd = indexOf(this.buffer, HEAD_END)
      if (headEnd === -1)
        return
      const headText = latin1(this.buffer.subarray(0, headEnd))
      this.buffer = this.buffer.subarray(headEnd + 4)
      const head = parseHead(headText)
      if (head.status >= 100 && head.status < 200) {
        // Interim response (100 Continue): discarded, the real head follows.
        this.feed(new Uint8Array(0))
        return
      }
      this.head = head
      const encoding = header(head, 'transfer-encoding') ?? ''
      const length = header(head, 'content-length')
      if (encoding.includes('chunked'))
        this.mode = 'chunked'
      else if (length !== undefined) {
        this.mode = 'length'
        this.remaining = Number(length)
        if (Number.isNaN(this.remaining))
          return this.fail('invalid content-length')
      } else if (head.status === 204 || head.status === 304) {
        // These statuses never carry a body, whatever the headers claim.
        this.mode = 'length'
        this.remaining = 0
      } else
        this.mode = 'eof'
      this.onHead(head)
      if (this.mode === 'length' && this.remaining === 0)
        this.finish()
    }
    this.parseBody()
  }

  /** The stream ended: read-until-close bodies end with it. */
  end(): void {
    if (this.mode === 'eof' || this.mode === 'none')
      this.finish()
  }

  fail(reason: string): void {
    if (this.done)
      return
    this.finish()
  }

  private finish(): void {
    if (this.done)
      return
    this.done = true
    this.onDone()
  }

  private parseBody(): void {
    if (this.mode === 'length') {
      if (this.remaining > 0 && this.buffer.length > 0) {
        const take = Math.min(this.remaining, this.buffer.length)
        this.onBody(this.buffer.subarray(0, take))
        this.buffer = this.buffer.subarray(take)
        this.remaining -= take
      }
      if (this.remaining === 0)
        this.finish()
    } else if (this.mode === 'chunked') {
      for (;;) {
        const lineEnd = indexOf(this.buffer, CRLF_BYTES)
        if (lineEnd === -1)
          return
        const size = Number(
          latin1(this.buffer.subarray(0, lineEnd)).split(';')[0]
        )
        if (!Number.isInteger(size) || size < 0)
          return this.fail('invalid chunk size')
        if (this.buffer.length < lineEnd + 2 + size + 2)
          return
        const chunk = this.buffer.subarray(lineEnd + 2, lineEnd + 2 + size)
        this.buffer = this.buffer.subarray(lineEnd + 2 + size + 2)
        if (size === 0) {
          // Trailer headers precede the final CRLF; drop until the empty line.
          const trailerEnd = indexOf(this.buffer, HEAD_END)
          if (trailerEnd === -1)
            return
          this.buffer = this.buffer.subarray(trailerEnd + 4)
          this.finish()
          return
        }
        this.onBody(chunk)
      }
    } else if (this.mode === 'eof') {
      if (this.buffer.length > 0) {
        this.onBody(this.buffer)
        this.buffer = new Uint8Array(0)
      }
    }
  }
}

/** Headers that never cross a proxy (`expose.md`: hop-by-hop removal). */
const HOP_BY_HOP = new Set([
  'connection',
  'keep-alive',
  'proxy-authenticate',
  'proxy-authorization',
  'te',
  'trailer',
  'transfer-encoding',
  'upgrade',
])

export function isHopByHop(name: string): boolean {
  return HOP_BY_HOP.has(name.toLowerCase())
}

/**
 * The request's head bytes: request line plus headers as given, ended. The
 * body is the caller's to stream.
 */
export function serializeRequestHead(
  method: string,
  target: string,
  headers: Array<[string, string]>
): Uint8Array {
  const lines = [`${method} ${target} HTTP/1.1`]
  for (const [name, value] of headers)
    lines.push(`${name}: ${value}`)
  return new TextEncoder().encode(`${lines.join(CRLF)}${CRLF}${CRLF}`)
}

export function chunkHeader(size: number): Uint8Array {
  return new TextEncoder().encode(`${size.toString(16)}${CRLF}`)
}

export const CHUNK_END = new TextEncoder().encode(CRLF)
export const CHUNKED_EOF = new TextEncoder().encode(`0${CRLF}${CRLF}`)

function parseHead(text: string): ResponseHead {
  const lines = text.split(CRLF)
  const match = /^HTTP\/1\.[01] (\d{3})(?: (.*))?$/.exec(lines[0]!)
  if (!match)
    throw new Error('malformed response head')
  const headers: Array<[string, string]> = []
  for (const line of lines.slice(1)) {
    const colon = line.indexOf(':')
    if (colon === -1)
      continue
    headers.push([line.slice(0, colon).trim(), line.slice(colon + 1).trim()])
  }
  return { status: Number(match[1]), headers }
}

function header(head: ResponseHead, name: string): string | undefined {
  return head.headers
    .find(([key]) => key.toLowerCase() === name)?.[1]
}

function latin1(bytes: Uint8Array): string {
  // Header bytes are ASCII by protocol; latin1 never throws on a stray byte.
  let out = ''
  for (const byte of bytes)
    out += String.fromCharCode(byte)
  return out
}

function indexOf(haystack: Uint8Array, needle: Uint8Array): number {
  outer:
  for (let i = 0; i <= haystack.length - needle.length; i++) {
    for (let j = 0; j < needle.length; j++) {
      if (haystack[i + j] !== needle[j])
        continue outer
    }
    return i
  }
  return -1
}

function concat(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length + b.length)
  out.set(a, 0)
  out.set(b, a.length)
  return out
}

const CRLF_BYTES = new TextEncoder().encode(CRLF)
