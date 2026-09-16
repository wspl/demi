import { describe, expect, test } from 'bun:test'
import {
  CHUNKED_EOF,
  CHUNK_END,
  HttpResponseParser,
  chunkHeader,
  serializeRequestHead
} from '../expose/http1'

function encode(text: string): Uint8Array {
  return new TextEncoder().encode(text)
}

function collect() {
  const heads: Array<{ status: number; headers: Array<[string, string]> }> = []
  const body: Uint8Array[] = []
  let done = 0
  return {
    heads,
    body,
    get done() {
      return done
    },
    parser: new HttpResponseParser(
      head => void heads.push(head),
      chunk => void body.push(chunk),
      () => {
        done += 1
      }
    ),
    text: () => new TextDecoder().decode(concat(body)),
  }
}

function concat(parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((n, part) => n + part.length, 0)
  const out = new Uint8Array(total)
  let at = 0
  for (const part of parts) {
    out.set(part, at)
    at += part.length
  }
  return out
}

describe('HttpResponseParser', () => {
  test('content-length body split across feeds', () => {
    const seen = collect()
    seen.parser.feed(encode('HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhel'))
    expect(seen.heads).toEqual([
      { status: 200, headers: [['Content-Length', '5']] }
    ])
    expect(seen.text()).toBe('hel')
    expect(seen.done).toBe(0)
    seen.parser.feed(encode('lo'))
    expect(seen.text()).toBe('hello')
    expect(seen.done).toBe(1)
  })

  test('chunked body fed byte-by-byte across many feeds terminates', () => {
    const seen = collect()
    const stream = encode(
      'HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n' +
      '5\r\nhello\r\n' +
      '10000\r\n' + 'x'.repeat(0x10000) + '\r\n' +
      '0\r\n\r\n'
    )
    for (let i = 0; i < stream.length; i += 7919)
      seen.parser.feed(stream.subarray(i, Math.min(i + 7919, stream.length)))
    expect(seen.text()).toBe('hello' + 'x'.repeat(0x10000))
    expect(seen.done).toBe(1)
  })

  test('chunked body with extensions and trailers', () => {
    const seen = collect()
    seen.parser.feed(encode(
      'HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n' +
      '3;ext=1\r\nabc\r\n' +
      '4\r\ndefg\r\n' +
      '0\r\nX-Trailer: t\r\n\r\n'
    ))
    expect(seen.text()).toBe('abcdefg')
    expect(seen.done).toBe(1)
  })

  test('interim 100 Continue responses are skipped', () => {
    const seen = collect()
    seen.parser.feed(encode(
      'HTTP/1.1 100 Continue\r\n\r\n' +
      'HTTP/1.1 201 Created\r\nContent-Length: 2\r\n\r\nok'
    ))
    expect(seen.heads.map(head => head.status)).toEqual([201])
    expect(seen.text()).toBe('ok')
    expect(seen.done).toBe(1)
  })

  test('read-until-close body ends with the stream', () => {
    const seen = collect()
    seen.parser.feed(encode('HTTP/1.1 200 OK\r\n\r\npartial'))
    expect(seen.done).toBe(0)
    seen.parser.end()
    expect(seen.text()).toBe('partial')
    expect(seen.done).toBe(1)
  })

  test('204 without a body finishes at the head', () => {
    const seen = collect()
    seen.parser.feed(encode('HTTP/1.1 204 No Content\r\n\r\n'))
    expect(seen.heads.map(head => head.status)).toEqual([204])
    expect(seen.done).toBe(1)
  })

  test('serialized request head round-trips through the parser shape', () => {
    const head = serializeRequestHead('POST', '/path?a=b', [
      ['Host', '127.0.0.1:5173'],
      ['Transfer-Encoding', 'chunked'],
    ])
    const text = new TextDecoder().decode(head)
    expect(text).toBe(
      'POST /path?a=b HTTP/1.1\r\nHost: 127.0.0.1:5173\r\n' +
      'Transfer-Encoding: chunked\r\n\r\n'
    )
  })

  test('chunk framing helpers compose a chunked body', () => {
    const chunk = new TextEncoder().encode('hello')
    const frame = concat([
      chunkHeader(chunk.byteLength),
      chunk,
      CHUNK_END,
      CHUNKED_EOF,
    ])
    expect(new TextDecoder().decode(frame))
      .toBe('5\r\nhello\r\n0\r\n\r\n')
  })
})
