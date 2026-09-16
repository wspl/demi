import { describe, expect, test } from 'bun:test'
import {
  WS_BINARY,
  WS_CLOSE,
  WS_PING,
  WS_TEXT,
  WsFrameParser,
  encodeClientFrame,
  encodeClosePayload,
  wsHandshake
} from '../expose/ws'

/** Unmasks a client frame the way a server would, then re-parses it. */
function unmask(frame: Uint8Array): Uint8Array {
  const opcode = frame[0]! & 0x0f
  const masked = (frame[1]! & 0x80) !== 0
  let length = frame[1]! & 0x7f
  let offset = 2
  if (length === 126) {
    length = new DataView(frame.buffer).getUint16(2)
    offset = 4
  } else if (length === 127) {
    length = Number(new DataView(frame.buffer).getBigUint64(2))
    offset = 10
  }
  expect(opcode).toBeGreaterThan(0)
  if (!masked)
    return frame.subarray(offset, offset + length)
  const key = frame.subarray(offset, offset + 4)
  const payload = frame.subarray(offset + 4, offset + 4 + length)
  const out = new Uint8Array(payload.length)
  for (let i = 0; i < payload.length; i++)
    out[i] = payload[i]! ^ key[i & 3]!
  return out
}

describe('client framing', () => {
  test('a text frame round-trips through its own mask', () => {
    const payload = new TextEncoder().encode('hello')
    const frame = encodeClientFrame(WS_TEXT, payload)
    expect(frame[0]! & 0x0f).toBe(WS_TEXT)
    expect(frame[1]! & 0x80).not.toBe(0)
    expect(new TextDecoder().decode(unmask(frame))).toBe('hello')
  })

  test('16-bit and 64-bit lengths frame large payloads', () => {
    for (const size of [300, 70_000]) {
      const payload = crypto.getRandomValues(new Uint8Array(size))
      const frame = encodeClientFrame(WS_BINARY, payload)
      expect(unmask(frame).length).toBe(size)
      expect(Buffer.from(unmask(frame)).equals(Buffer.from(payload))).toBe(true)
    }
  })

  test('the close payload carries the code and the reason', () => {
    const payload = encodeClosePayload(4001, 'bye')
    expect(payload[0]).toBe(0x0f)
    expect(payload[1]).toBe(0xa1)
    expect(new TextDecoder().decode(payload.subarray(2))).toBe('bye')
  })

  test('the handshake request names the upgrade', () => {
    const text = new TextDecoder().decode(wsHandshake('/ws?x=1', '127.0.0.1:9', 'key=='))
    expect(text).toContain('GET /ws?x=1 HTTP/1.1\r\n')
    expect(text).toContain('Upgrade: websocket')
    expect(text).toContain('Sec-WebSocket-Key: key==')
  })
})

describe('WsFrameParser', () => {
  test('parses text, binary, ping and close frames', () => {
    const parser = new WsFrameParser()
    const server = (opcode: number, payload: Uint8Array): Uint8Array => {
      const header = new Uint8Array([0x80 | opcode, payload.length])
      return new Uint8Array([...header, ...payload])
    }
    const frames = parser.feed(new Uint8Array([
      ...server(WS_TEXT, new TextEncoder().encode('hi')),
      ...server(WS_PING, new Uint8Array([1])),
      ...server(WS_CLOSE, encodeClosePayload(1000, 'done')),
    ]))
    expect(frames).toEqual([
      { kind: 'message', opcode: WS_TEXT, data: new TextEncoder().encode('hi') },
      { kind: 'ping', data: new Uint8Array([1]) },
      { kind: 'close', code: 1000, reason: 'done' },
    ])
  })

  test('assembles a fragmented message', () => {
    const parser = new WsFrameParser()
    const first = new Uint8Array([0x01, 2, 0x68, 0x69]) // text, no FIN
    const second = new Uint8Array([0x80, 1, 0x21]) // FIN continuation
    expect(parser.feed(first)).toEqual([])
    expect(parser.feed(second)).toEqual([
      { kind: 'message', opcode: WS_TEXT, data: new TextEncoder().encode('hi!') }
    ])
  })

  test('delivers frames that arrive split mid-header', () => {
    const parser = new WsFrameParser()
    const whole = new Uint8Array([0x81, 3, 0x61, 0x62, 0x63])
    expect(parser.feed(whole.subarray(0, 2))).toEqual([])
    expect(parser.feed(whole.subarray(2))).toEqual([
      { kind: 'message', opcode: WS_TEXT, data: new TextEncoder().encode('abc') }
    ])
  })

  test('rejects a masked server frame', () => {
    const parser = new WsFrameParser()
    expect(() => parser.feed(new Uint8Array([0x81, 0x80 | 1, 0x61])))
      .toThrow('a server frame must not be masked')
  })
})
