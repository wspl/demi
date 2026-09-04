import { deferred, type Deferred } from './async'

const encoder = new TextEncoder()
const decoder = new TextDecoder()

/** Encodes a string to UTF-8 bytes. */
export function encodeUtf8(text: string): Uint8Array {
  return encoder.encode(text)
}

/** Decodes UTF-8 bytes to a string. */
export function decodeUtf8(data: Uint8Array): string {
  return decoder.decode(data)
}

/** Returns the UTF-8 byte length of a string. */
export function utf8Bytes(text: string): number {
  return encoder.encode(text).byteLength
}

/** Packs a latin1 byte-string (each char = one byte, 0–255) into bytes. */
export function encodeLatin1(text: string): Uint8Array {
  const out = new Uint8Array(text.length)
  for (let i = 0; i < text.length; i += 1) out[i] = text.charCodeAt(i) & 0xff
  return out
}

/**
 * The UTF-8 encoding of `text` as a latin1 byte-string (each char = one
 * byte), for code that processes text byte by byte, as the C locale does.
 */
export function utf8AsLatin1(text: string): string {
  return decodeLatin1(encodeUtf8(text))
}

/** Orders strings by UTF-16 code unit, which for byte-strings is byte order. */
export function compareCodeUnits(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0
}

/** Orders strings by their UTF-8 bytes, as the C locale orders names and lines. */
export function compareUtf8Bytes(a: string, b: string): number {
  return compareCodeUnits(utf8AsLatin1(a), utf8AsLatin1(b))
}

/** Unpacks bytes into a latin1 byte-string (each char = one byte). */
export function decodeLatin1(bytes: Uint8Array): string {
  let out = ''
  const chunk = 0x8000
  for (let i = 0; i < bytes.length; i += chunk) {
    out += String.fromCharCode(...bytes.subarray(i, i + chunk))
  }
  return out
}

/** Strictly decodes UTF-8; returns null when the bytes are not valid UTF-8. */
export function decodeUtf8Strict(bytes: Uint8Array): string | null {
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes)
  } catch {
    return null
  }
}

/** Slices a string by UTF-8 byte offsets, returning the decoded substring. */
export function utf8Slice(text: string, start: number, end: number): string {
  if (start <= 0 && end >= utf8Bytes(text)) return text
  return decoder.decode(encoder.encode(text).slice(start, end))
}

/**
 * Splits a UTF-8 byte stream into lines. Newlines (`\n`, with an optional
 * preceding `\r`) are stripped; a trailing chunk without a final newline is
 * still yielded. Multi-byte sequences split across chunks decode correctly.
 */
export async function* utf8Lines(chunks: AsyncIterable<Uint8Array>): AsyncIterable<string> {
  const streamDecoder = new TextDecoder()
  let buffer = ''
  for await (const chunk of chunks) {
    buffer += streamDecoder.decode(chunk, { stream: true })
    let newline = buffer.indexOf('\n')
    while (newline !== -1) {
      const line = buffer.slice(0, newline)
      yield line.endsWith('\r') ? line.slice(0, -1) : line
      buffer = buffer.slice(newline + 1)
      newline = buffer.indexOf('\n')
    }
  }
  buffer += streamDecoder.decode()
  if (buffer.length > 0) yield buffer.endsWith('\r') ? buffer.slice(0, -1) : buffer
}

/** Concatenates byte chunks into a single `Uint8Array`. */
export function concatBytes(chunks: readonly Uint8Array[]): Uint8Array {
  const length = chunks.reduce((total, chunk) => total + chunk.byteLength, 0)
  const combined = new Uint8Array(length)
  let offset = 0
  for (const chunk of chunks) {
    combined.set(chunk, offset)
    offset += chunk.byteLength
  }
  return combined
}

const BASE64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'

/** Encodes bytes as standard base64 (platform-neutral, no Node or DOM globals). */
export function bytesToBase64(bytes: Uint8Array): string {
  let output = ''
  for (let index = 0; index < bytes.byteLength; index += 3) {
    const first = bytes[index]
    const second = bytes[index + 1]
    const third = bytes[index + 2]
    output += BASE64_ALPHABET[first >> 2]
    output += BASE64_ALPHABET[((first & 0x03) << 4) | ((second ?? 0) >> 4)]
    output += second === undefined ? '=' : BASE64_ALPHABET[((second & 0x0f) << 2) | ((third ?? 0) >> 6)]
    output += third === undefined ? '=' : BASE64_ALPHABET[third & 0x3f]
  }
  return output
}

/** Decodes standard base64 to bytes, throwing on malformed payloads. */
export function base64ToBytes(base64: string): Uint8Array {
  const clean = base64.replace(/\s+/g, '')
  if (clean.length === 0) return new Uint8Array()
  if (clean.length % 4 !== 0) throw new Error('Invalid base64 payload length')

  const padding = clean.endsWith('==') ? 2 : clean.endsWith('=') ? 1 : 0
  const bytes = new Uint8Array((clean.length / 4) * 3 - padding)
  let offset = 0

  for (let index = 0; index < clean.length; index += 4) {
    const first = base64Value(clean[index])
    const second = base64Value(clean[index + 1])
    const third = clean[index + 2] === '=' ? 0 : base64Value(clean[index + 2])
    const fourth = clean[index + 3] === '=' ? 0 : base64Value(clean[index + 3])
    const triple = (first << 18) | (second << 12) | (third << 6) | fourth

    if (offset < bytes.byteLength) bytes[offset++] = (triple >> 16) & 0xff
    if (offset < bytes.byteLength) bytes[offset++] = (triple >> 8) & 0xff
    if (offset < bytes.byteLength) bytes[offset++] = triple & 0xff
  }

  return bytes
}

function base64Value(char: string | undefined): number {
  if (!char || char === '=') throw new Error('Invalid base64 payload')
  const value = BASE64_ALPHABET.indexOf(char)
  if (value === -1) throw new Error('Invalid base64 payload')
  return value
}

/** Text as UTF-8 bytes; bytes unchanged. */
export function toBytes(data: string | Uint8Array): Uint8Array {
  return typeof data === 'string' ? encoder.encode(data) : data
}

/** A byte stream that ends at once. */
export async function* emptyByteStream(): AsyncIterable<Uint8Array> {}

/** A byte stream of one chunk (none when the chunk is empty). */
export async function* bytesStream(bytes: Uint8Array): AsyncIterable<Uint8Array> {
  if (bytes.byteLength > 0) yield bytes
}

/** The streams one after another. */
export async function* concatByteStreams(...streams: AsyncIterable<Uint8Array>[]): AsyncIterable<Uint8Array> {
  for (const stream of streams) yield* stream
}

/** Drains a byte stream into one `Uint8Array`. */
export async function collectBytes(stream: AsyncIterable<Uint8Array>): Promise<Uint8Array> {
  const chunks: Uint8Array[] = []
  for await (const chunk of stream) chunks.push(chunk)
  return concatBytes(chunks)
}

/**
 * One iterator shared by several consumers in turn: a consumer that stops
 * early leaves the stream open for the next, the way processes share a shell's
 * stdin. The stream ends only when the source ends.
 */
export function shareByteStream(source: AsyncIterable<Uint8Array>): AsyncIterable<Uint8Array> {
  const iterator = source[Symbol.asyncIterator]()
  return {
    [Symbol.asyncIterator]: () => ({
      next: () => iterator.next(),
      return: async () => ({ done: true, value: undefined }),
    }),
  }
}

/** Async chunk queue: each pushed chunk is delivered once, in order; close ends the stream. */
export class ByteQueue {
  private readonly chunks: Uint8Array[] = []
  private waiter: (() => void) | null = null
  private isClosed = false

  push(data: Uint8Array): void {
    if (this.isClosed) return
    this.chunks.push(data)
    this.wake()
  }

  close(): void {
    if (this.isClosed) return
    this.isClosed = true
    this.wake()
  }

  get closed(): boolean {
    return this.isClosed
  }

  async *stream(): AsyncIterable<Uint8Array> {
    while (true) {
      const chunk = this.chunks.shift()
      if (chunk) {
        yield chunk
        continue
      }
      if (this.isClosed) return
      await new Promise<void>((resolve) => {
        this.waiter = resolve
      })
    }
  }

  private wake(): void {
    const waiter = this.waiter
    this.waiter = null
    waiter?.()
  }
}

/**
 * A byte channel with backpressure: `push` resolves once the consumer took
 * the chunk, so a producer never runs ahead of its consumer by more than one
 * chunk. `close` ends the stream after the chunks already taken; `fail` ends
 * it with an error. A consumer that stops early fails the producer's next
 * `push`, the way a closed pipe does.
 */
export class ByteChannel {
  private pending: { chunk: Uint8Array; taken: Deferred<void> } | null = null
  private waiting: Deferred<void> | null = null
  private ended: { error?: unknown } | null = null

  async push(chunk: Uint8Array): Promise<void> {
    while (this.pending) await this.pending.taken.promise.catch(() => {})
    if (this.ended) throw this.ended.error ?? new Error('channel closed')
    const taken = deferred<void>()
    this.pending = { chunk, taken }
    this.wake()
    await taken.promise
  }

  close(): void {
    this.end({})
  }

  fail(error: unknown): void {
    this.end({ error })
  }

  async *stream(): AsyncIterable<Uint8Array> {
    try {
      for (;;) {
        const pending = this.pending
        if (pending) {
          this.pending = null
          pending.taken.resolve()
          yield pending.chunk
          continue
        }
        if (this.ended) {
          if ('error' in this.ended) throw this.ended.error
          return
        }
        this.waiting = deferred<void>()
        await this.waiting.promise
      }
    } finally {
      // The consumer is gone: nothing pushed from here on can be delivered.
      this.end({ error: new Error('channel consumer gone') })
    }
  }

  private end(ended: { error?: unknown }): void {
    if (this.ended) return
    this.ended = ended
    this.pending?.taken.reject(ended.error ?? new Error('channel closed'))
    this.pending = null
    this.wake()
  }

  private wake(): void {
    const waiting = this.waiting
    this.waiting = null
    waiting?.resolve()
  }
}
