import { decodeLatin1, toBytes } from '@demicodes/utils'
import type { TinybashWriter } from '../host'

export type Writer = TinybashWriter

/** Thrown into a writer whose reader has gone away; the producing builtin stops quietly, as SIGPIPE would end it. */
export class PipeClosed extends Error {
  constructor() {
    super('pipe closed')
    this.name = 'PipeClosed'
  }
}

/**
 * A byte pipe between two commands of a pipeline: the writer awaits until the
 * reader has taken the chunk, so a fast producer never outruns a slow consumer
 * by more than one chunk, and a reader that stops early closes the writer.
 */
export class Pipe {
  private readonly chunks: Uint8Array[] = []
  private closed = false
  private readerGone = false
  private wakeReader: (() => void) | null = null
  private wakeWriter: (() => void) | null = null

  readonly write: Writer = async (data) => {
    if (this.readerGone) throw new PipeClosed()
    const bytes = toBytes(data)
    if (bytes.length === 0) return
    this.chunks.push(bytes)
    this.wakeReader?.()
    while (this.chunks.length > 0 && !this.readerGone) {
      await new Promise<void>((resolve) => {
        this.wakeWriter = resolve
      })
      this.wakeWriter = null
    }
    if (this.readerGone && this.chunks.length > 0) throw new PipeClosed()
  }

  /** The writer is done; the reader sees end of stream after the buffered chunks. */
  close(): void {
    this.closed = true
    this.wakeReader?.()
  }

  /** The reader is done; further writes fail with `PipeClosed`. */
  abandon(): void {
    this.readerGone = true
    this.chunks.length = 0
    this.wakeWriter?.()
  }

  async *[Symbol.asyncIterator](): AsyncIterator<Uint8Array> {
    try {
      for (;;) {
        if (this.chunks.length > 0) {
          const chunk = this.chunks.shift()!
          this.wakeWriter?.()
          yield chunk
          continue
        }
        if (this.closed) return
        await new Promise<void>((resolve) => {
          this.wakeReader = resolve
        })
        this.wakeReader = null
      }
    } finally {
      this.abandon()
    }
  }
}

/**
 * Lines of a byte stream, split on `\n`. Each line is returned without its
 * newline plus a flag saying whether one followed it, so tools can reproduce
 * GNU behaviour on a missing final newline. Bytes are decoded as latin1 so
 * every byte round-trips.
 */
export async function* lines(stream: AsyncIterable<Uint8Array>): AsyncIterable<{ text: string; newline: boolean }> {
  let carry = ''
  for await (const chunk of stream) {
    let text = carry + decodeLatin1(chunk)
    let start = 0
    for (;;) {
      const index = text.indexOf('\n', start)
      if (index === -1) break
      yield { text: text.slice(start, index), newline: true }
      start = index + 1
    }
    carry = text.slice(start)
    text = ''
  }
  if (carry.length > 0) yield { text: carry, newline: false }
}
