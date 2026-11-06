/**
 * A WebSocket on a stream of the process's HTTP/2 connection (RFC 8441):
 * the stream is the socket's, and its bytes are WebSocket frames (RFC 6455),
 * which Node's `http2` does not write, so this module does. A client masks
 * each frame it sends; the process's frames come unmasked.
 *
 * It has the part of a browser's `WebSocket` the page connection and a
 * forwarded page's socket use: `send`, `close`, `bufferedAmount`,
 * `binaryType`, and the `open`, `message` and `close` events.
 */
import { randomFillSync } from 'node:crypto'
import type { ClientHttp2Stream } from 'node:http2'

const TEXT = 0x1
const BINARY = 0x2
const CLOSE = 0x8
const PING = 0x9
const PONG = 0xa
const CONTINUATION = 0x0
/** The close of a socket that ended without a close frame. */
const ABNORMAL_CLOSURE = 1006
/** The close frame's code when it carries none. */
const NO_STATUS = 1005

type Listener = (event: never) => void

/** The socket's events, as a WebSocket's. */
export interface StreamSocketEvents {
  open: () => void
  message: (event: { data: string | ArrayBuffer }) => void
  close: (event: { code: number; reason: string }) => void
}

/** A WebSocket on the stream `opening` answers once the process took it. */
export class StreamSocket {
  binaryType: 'blob' | 'arraybuffer' = 'arraybuffer'
  private stream: ClientHttp2Stream | null = null
  private readonly listeners = new Map<keyof StreamSocketEvents, Set<Listener>>()
  /** The bytes read that do not make a whole frame yet. */
  private pending = new Uint8Array(0)
  /** The fragments of a message whose last frame has not come. */
  private fragments: { opcode: number; parts: Uint8Array[] } | null = null
  private closed = false
  /** Whether this end sent its close frame. */
  private closing = false

  constructor(opening: Promise<ClientHttp2Stream>) {
    opening.then(
      (stream) => {
        if (this.closed) {
          stream.close()
          return
        }
        this.stream = stream
        stream.on('data', (chunk: Buffer) => this.read(chunk))
        stream.once('close', () => this.ended(ABNORMAL_CLOSURE, ''))
        stream.on('error', () => {})
        this.emit('open')
      },
      () => this.ended(ABNORMAL_CLOSURE, ''),
    )
  }

  /** The bytes sent that the stream has not written yet. */
  get bufferedAmount(): number {
    return this.stream?.writableLength ?? 0
  }

  addEventListener<K extends keyof StreamSocketEvents>(type: K, listener: StreamSocketEvents[K]): void {
    let set = this.listeners.get(type)
    if (!set) {
      set = new Set()
      this.listeners.set(type, set)
    }
    set.add(listener as Listener)
  }

  removeEventListener<K extends keyof StreamSocketEvents>(type: K, listener: StreamSocketEvents[K]): void {
    this.listeners.get(type)?.delete(listener as Listener)
  }

  /** Sends a message: text for a string, binary for bytes. */
  send(data: string | Uint8Array | ArrayBuffer): void {
    if (typeof data === 'string') {
      this.write(TEXT, new TextEncoder().encode(data))
    } else {
      this.write(BINARY, data instanceof ArrayBuffer ? new Uint8Array(data) : data)
    }
  }

  /** Closes the socket with `code` and `reason`; its `close` event follows. */
  close(code = 1000, reason = ''): void {
    if (this.closed || this.closing) {
      return
    }
    this.closing = true
    if (this.stream === null) {
      this.ended(ABNORMAL_CLOSURE, '')
      return
    }
    const text = new TextEncoder().encode(reason)
    const body = new Uint8Array(2 + text.length)
    new DataView(body.buffer).setUint16(0, code)
    body.set(text, 2)
    this.write(CLOSE, body)
    this.stream.end()
  }

  private emit<K extends keyof StreamSocketEvents>(type: K, ...event: Parameters<StreamSocketEvents[K]>): void {
    for (const listener of [...(this.listeners.get(type) ?? [])]) {
      ;(listener as unknown as (...args: Parameters<StreamSocketEvents[K]>) => void)(...event)
    }
  }

  /** Writes one frame of `opcode` with `payload`, masked. */
  private write(opcode: number, payload: Uint8Array): void {
    const stream = this.stream
    if (stream === null || this.closed || stream.destroyed) {
      return
    }
    const length = payload.length
    const extended = length < 126 ? 0 : length < 0x1_0000 ? 2 : 8
    const frame = new Uint8Array(2 + extended + 4 + length)
    const view = new DataView(frame.buffer)
    frame[0] = 0x80 | opcode
    if (extended === 0) {
      frame[1] = 0x80 | length
    } else if (extended === 2) {
      frame[1] = 0x80 | 126
      view.setUint16(2, length)
    } else {
      frame[1] = 0x80 | 127
      view.setBigUint64(2, BigInt(length))
    }
    const at = 2 + extended
    const mask = frame.subarray(at, at + 4)
    randomFillSync(mask)
    const body = frame.subarray(at + 4)
    for (let index = 0; index < length; index += 1) {
      body[index] = payload[index]! ^ mask[index & 3]!
    }
    stream.write(frame)
  }

  /** Takes bytes of the stream, and handles each whole frame they complete. */
  private read(chunk: Uint8Array): void {
    let bytes = this.pending.length === 0 ? chunk : concat([this.pending, chunk])
    while (bytes.length >= 2) {
      const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
      const fin = (bytes[0]! & 0x80) !== 0
      const opcode = bytes[0]! & 0x0f
      let length = bytes[1]! & 0x7f
      let at = 2
      if (length === 126) {
        if (bytes.length < 4) {
          break
        }
        length = view.getUint16(2)
        at = 4
      } else if (length === 127) {
        if (bytes.length < 10) {
          break
        }
        length = Number(view.getBigUint64(2))
        at = 10
      }
      // The process sends no masked frame.
      if (bytes.length < at + length) {
        break
      }
      const payload = bytes.slice(at, at + length)
      bytes = bytes.subarray(at + length)
      this.frame(fin, opcode, payload)
    }
    this.pending = bytes.slice()
  }

  private frame(fin: boolean, opcode: number, payload: Uint8Array): void {
    switch (opcode) {
      case PING:
        this.write(PONG, payload)
        return
      case PONG:
        return
      case CLOSE: {
        const code = payload.length >= 2 ? new DataView(payload.buffer, payload.byteOffset).getUint16(0) : NO_STATUS
        const reason = new TextDecoder().decode(payload.subarray(2))
        if (!this.closing) {
          this.closing = true
          this.write(CLOSE, payload.subarray(0, 2))
          this.stream?.end()
        }
        this.ended(code, reason)
        return
      }
      case CONTINUATION: {
        if (this.fragments === null) {
          return
        }
        this.fragments.parts.push(payload)
        if (fin) {
          const { opcode: first, parts } = this.fragments
          this.fragments = null
          this.message(first, concat(parts))
        }
        return
      }
      case TEXT:
      case BINARY:
        if (fin) {
          this.message(opcode, payload)
        } else {
          this.fragments = { opcode, parts: [payload] }
        }
    }
  }

  private message(opcode: number, payload: Uint8Array): void {
    const data = opcode === TEXT
      ? new TextDecoder().decode(payload)
      : (payload.buffer.slice(payload.byteOffset, payload.byteOffset + payload.byteLength) as ArrayBuffer)
    this.emit('message', { data })
  }

  private ended(code: number, reason: string): void {
    if (this.closed) {
      return
    }
    this.closed = true
    this.stream?.close()
    this.emit('close', { code, reason })
  }
}

function concat(parts: readonly Uint8Array[]): Uint8Array {
  const whole = new Uint8Array(parts.reduce((total, part) => total + part.length, 0))
  let at = 0
  for (const part of parts) {
    whole.set(part, at)
    at += part.length
  }
  return whole
}
