// The WebSocket client on the relay's byte stream (`expose.md` § The public
// relay): the server runtime offers no raw socket after an upgrade, so the
// service side speaks the handshake and framing by hand — client frames are
// masked, server frames are not. Payloads are re-emitted unchanged.

import { serializeRequestHead } from './http1'

export const WS_TEXT = 0x1
export const WS_BINARY = 0x2
export const WS_CLOSE = 0x8
export const WS_PING = 0x9
export const WS_PONG = 0xa

/** A fresh `Sec-WebSocket-Key`, as the handshake requires. */
export function wsKey(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16))
  let binary = ''
  for (const byte of bytes)
    binary += String.fromCharCode(byte)
  return btoa(binary)
}

export function wsHandshake(
  target: string,
  host: string,
  key: string
): Uint8Array {
  return serializeRequestHead('GET', target, [
    ['Host', host],
    ['Upgrade', 'websocket'],
    ['Connection', 'Upgrade'],
    ['Sec-WebSocket-Key', key],
    ['Sec-WebSocket-Version', '13'],
  ])
}

/** One masked client frame: FIN set, one opcode, the whole payload. */
export function encodeClientFrame(
  opcode: number,
  payload: Uint8Array
): Uint8Array {
  const key = crypto.getRandomValues(new Uint8Array(4))
  const masked = new Uint8Array(payload.length)
  for (let i = 0; i < payload.length; i++)
    masked[i] = payload[i]! ^ key[i & 3]!
  let header: Uint8Array
  if (masked.length < 126) {
    header = new Uint8Array([0x80 | opcode, 0x80 | masked.length])
  } else if (masked.length < 65_536) {
    header = new Uint8Array(4)
    header[0] = 0x80 | opcode
    header[1] = 0x80 | 126
    new DataView(header.buffer).setUint16(2, masked.length)
  } else {
    header = new Uint8Array(10)
    header[0] = 0x80 | opcode
    header[1] = 0x80 | 127
    new DataView(header.buffer).setBigUint64(2, BigInt(masked.length))
  }
  const frame = new Uint8Array(header.length + 4 + masked.length)
  frame.set(header, 0)
  frame.set(key, header.length)
  frame.set(masked, header.length + 4)
  return frame
}

/** The close frame's payload: the code and its reason, both directions. */
export function encodeClosePayload(code: number, reason: string): Uint8Array {
  const reasonBytes = new TextEncoder().encode(reason)
  const payload = new Uint8Array(2 + reasonBytes.length)
  payload[0] = (code >> 8) & 0xff
  payload[1] = code & 0xff
  payload.set(reasonBytes, 2)
  return payload
}

export type ServerFrame =
  | { kind: 'message'; opcode: typeof WS_TEXT | typeof WS_BINARY; data: Uint8Array }
  | { kind: 'close'; code: number; reason: string }
  | { kind: 'ping'; data: Uint8Array }
  | { kind: 'pong'; data: Uint8Array }

/**
 * Unmasks and parses server frames as they arrive; a fragmented message is
 * assembled before delivery, control frames never are.
 */
export class WsFrameParser {
  private buffer: Uint8Array = new Uint8Array(0)
  private fragment: { opcode: number; parts: Uint8Array[] } | null = null

  feed(data: Uint8Array): ServerFrame[] {
    this.buffer = this.buffer.length === 0
      ? data
      : concat(this.buffer, data)
    const frames: ServerFrame[] = []
    for (;;) {
      if (this.buffer.length < 2)
        break
      const fin = (this.buffer[0]! & 0x80) !== 0
      const opcode = this.buffer[0]! & 0x0f
      const masked = (this.buffer[1]! & 0x80) !== 0
      let length = this.buffer[1]! & 0x7f
      let offset = 2
      if (length === 126) {
        if (this.buffer.length < 4)
          break
        length = new DataView(
          this.buffer.buffer,
          this.buffer.byteOffset
        ).getUint16(2)
        offset = 4
      } else if (length === 127) {
        if (this.buffer.length < 10)
          break
        length = Number(new DataView(
          this.buffer.buffer,
          this.buffer.byteOffset
        ).getBigUint64(2))
        if (!Number.isSafeInteger(length))
          throw new Error('frame too large')
      }
      if (masked)
        throw new Error('a server frame must not be masked')
      if (this.buffer.length < offset + length)
        break
      const payload = this.buffer.subarray(offset, offset + length)
      this.buffer = this.buffer.subarray(offset + length)
      if (opcode >= 0x8) {
        if (!fin || payload.length > 125)
          throw new Error('malformed control frame')
        if (opcode === WS_CLOSE) {
          frames.push({
            kind: 'close',
            code: payload.length >= 2
              ? (payload[0]! << 8) | payload[1]!
              : 1005,
            reason: payload.length > 2
              ? new TextDecoder().decode(payload.subarray(2))
              : '',
          })
        } else if (opcode === WS_PING) {
          frames.push({ kind: 'ping', data: payload })
        } else {
          frames.push({ kind: 'pong', data: payload })
        }
        continue
      }
      if (opcode === 0) {
        if (!this.fragment)
          throw new Error('continuation without a message')
        this.fragment.parts.push(payload)
        if (!fin)
          continue
        const message = concat(...this.fragment.parts)
        const whole: ServerFrame = { kind: 'message', opcode: this.fragment.opcode as typeof WS_TEXT | typeof WS_BINARY, data: message }
        this.fragment = null
        frames.push(whole)
        continue
      }
      if (!fin) {
        this.fragment = { opcode, parts: [payload] }
        continue
      }
      frames.push({
        kind: 'message',
        opcode: opcode as typeof WS_TEXT | typeof WS_BINARY,
        data: payload,
      })
    }
    return frames
  }
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((n, part) => n + part.length, 0)
  const out = new Uint8Array(total)
  let at = 0
  for (const part of parts) {
    out.set(part, at)
    at += part.length
  }
  return out
}
