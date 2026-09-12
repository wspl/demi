import { describe, expect, it } from 'bun:test'
import {
  parseJsonObject,
  parseJsonOrString,
  parsePortableJson,
  stringifyPortableJson
} from '../json'
import { normalizeBaseUrl } from '../strings'

describe('parseJsonOrString', () => {
  it('parses valid JSON', () => {
    expect(parseJsonOrString('{"a":1}')).toEqual({ a: 1 })
    expect(parseJsonOrString('42')).toBe(42)
  })

  it('returns the original string when invalid', () => {
    expect(parseJsonOrString('not json')).toBe('not json')
  })
})

describe('parseJsonObject', () => {
  it('returns the object for a JSON object', () => {
    expect(parseJsonObject('{"a":1}')).toEqual({ a: 1 })
  })

  it('returns null for non-objects and invalid JSON', () => {
    expect(parseJsonObject('[1,2]')).toBeNull()
    expect(parseJsonObject('42')).toBeNull()
    expect(parseJsonObject('nope')).toBeNull()
  })
})

describe('portable JSON codec', () => {
  it('round-trips Uint8Array and bigint values', () => {
    const encoded = stringifyPortableJson({
      metadata: { count: 42n },
      bytes: new Uint8Array([1, 2, 3]),
      empty: new Uint8Array(),
      one: new Uint8Array([255]),
      two: new Uint8Array([254, 253]),
    })

    expect(parsePortableJson(encoded)).toEqual({
      metadata: { count: 42n },
      bytes: new Uint8Array([1, 2, 3]),
      empty: new Uint8Array(),
      one: new Uint8Array([255]),
      two: new Uint8Array([254, 253]),
    })
  })

  it('round-trips Date values at the root, nested and in arrays', () => {
    const date = new Date('2026-08-31T12:34:56.789Z')
    const value = { at: date, list: [date], s: date.toISOString() }
    expect(parsePortableJson(stringifyPortableJson(value))).toEqual(value)
    const top = parsePortableJson(stringifyPortableJson(date))
    if (!(top instanceof Date))
      throw new Error('Expected a Date')
    expect(top.getTime()).toBe(date.getTime())
  })

  it('round-trips Buffer as Uint8Array despite Buffer.toJSON', () => {
    expect(parsePortableJson(stringifyPortableJson({ bytes: Buffer.from([9, 8, 7]) })))
      .toEqual({ bytes: new Uint8Array([9, 8, 7]) })
  })

  it('parses plain JSON without markers unchanged', () => {
    expect(parsePortableJson('{"a":[1,2]}')).toEqual({ a: [1, 2] })
  })

  it('rejects malformed reserved markers without exposing payload data', () => {
    const invalid = [
      { __demiUint8Array: false, base64: 'AQ==' },
      { __demiUint8Array: true },
      { __demiUint8Array: true, base64: 123 },
      { __demiUint8Array: true, base64: 'AQ==', extra: 'secret' },
      { __demiUint8Array: true, base64: 'AQ==AAAA' },
      { __demiUint8Array: true, base64: 'AR==' },
      { __demiUint8Array: true, base64: 'AQI=' + '\n' },
      { __demiBigInt: true, value: '01' },
      { __demiBigInt: true, value: '-0' },
      { __demiBigInt: true, value: ' 1' },
      { __demiBigInt: true, value: 'secret' },
      { __demiDate: true, iso: 'secret' },
      { __demiDate: true, iso: '2026-02-30T00:00:00.000Z' },
      { __demiDate: true, iso: '2026-01-01' },
      { __demiDate: true, iso: '2026-01-01T00:00:00.000Z', extra: 1 },
      { __demiDate: true, __demiBigInt: true, value: '1' },
    ]
    for (const value of invalid) {
      expect(() => parsePortableJson(JSON.stringify({ nested: value })))
        .toThrow()
      try {
        parsePortableJson(JSON.stringify(value))
      } catch (error) {
        expect(String(error)).not.toContain('secret')
      }
    }
  })

  it('reserves marker keys at write time and rejects nonfinite JSON numbers', () => {
    expect(() => stringifyPortableJson({ __demiBigInt: true, value: '42' }))
      .toThrow('reserved')
    for (const number of [NaN, Infinity, -Infinity]) {
      expect(() => stringifyPortableJson({ number })).toThrow('finite')
    }
    expect(() => parsePortableJson('{"number":1e400}')).toThrow('finite')
    for (const value of [0n, -1n, 123456789012345678901234567890n]) {
      const decoded: unknown = parsePortableJson(stringifyPortableJson(value))
      expect(decoded === value).toBe(true)
    }
  })

  it('rejects lossy writes while permitting absent optional object fields', () => {
    for (const value of [undefined, () => {}, Symbol('x'), [undefined], new Map(), new Set(), { toJSON: () => ({}) }]) {
      expect(() => stringifyPortableJson(value)).toThrow()
    }
    expect(stringifyPortableJson({ absent: undefined, present: null }))
      .toBe('{"present":null}')
  })

  it('supports pretty-printing via the space parameter', () => {
    expect(stringifyPortableJson({ a: 1 }, 2)).toBe('{\n  "a": 1\n}')
  })
})

describe('normalizeBaseUrl', () => {
  it('strips trailing slashes', () => {
    expect(normalizeBaseUrl('https://api.example.com/'))
      .toBe('https://api.example.com')
    expect(normalizeBaseUrl('https://api.example.com///'))
      .toBe('https://api.example.com')
    expect(normalizeBaseUrl('https://api.example.com/v1'))
      .toBe('https://api.example.com/v1')
  })
})
