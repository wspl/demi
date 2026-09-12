// Newline-delimited JSON over a Bun socket, for both ends of the machine
// wire: a reader that cuts the byte stream into lines, and a writer that
// keeps what the kernel did not take until the socket drains.

export function encodeFrame(value: unknown): Uint8Array {
  return new TextEncoder().encode(`${JSON.stringify(value)}\n`)
}

export class LineReader {
  private readonly decoder = new TextDecoder()
  private partial = ''

  /** Feeds a chunk; calls `onLine` once per complete line, without its newline. */
  push(chunk: Uint8Array, onLine: (line: string) => void): void {
    this.partial += this.decoder.decode(chunk, { stream: true })
    let newline = this.partial.indexOf('\n')
    while (newline !== -1) {
      const line = this.partial.slice(0, newline)
      this.partial = this.partial.slice(newline + 1)
      if (line.length > 0) {
        onLine(line)
      }
      newline = this.partial.indexOf('\n')
    }
  }
}

interface WritableSocket {
  write(data: Uint8Array): number
}

export class FrameWriter {
  private backlog: Uint8Array | null = null

  constructor(private readonly socket: WritableSocket) {}

  write(frame: Uint8Array): void {
    if (this.backlog) {
      this.backlog = concat(this.backlog, frame)
      return
    }
    const written = this.socket.write(frame)
    if (written < frame.byteLength) {
      this.backlog = frame.subarray(written)
    }
  }

  /** The socket's `drain` handler: sends what the last write left behind. */
  drain(): void {
    const backlog = this.backlog
    this.backlog = null
    if (backlog) {
      this.write(backlog)
    }
  }
}

function concat(first: Uint8Array, second: Uint8Array): Uint8Array {
  const joined = new Uint8Array(first.byteLength + second.byteLength)
  joined.set(first)
  joined.set(second, first.byteLength)
  return joined
}
