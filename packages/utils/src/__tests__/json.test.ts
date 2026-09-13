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

    // toEqual distinguishes a Uint8Array from a plain array, so this also
    // asserts that the markers revived into typed arrays.
    expect(parsePortableJson(encoded)).toEqual({
      metadata: { count: 42n },
      bytes: new Uint8Array([1, 2, 3]),
      empty: new Uint8Array(),
      one: new Uint8Array([255]),
      two: new Uint8Array([254, 253]),
    })
  })

  it(
    'round-trips Date values, including nested, top-level, and in arrays',
    () => {
      const date = new Date('2026-08-31T12:34:56.789Z')
      // toEqual distinguishes a Date from its ISO string, so this also
      // asserts that the markers revived into Date values.
      expect(parsePortableJson(
        stringifyPortableJson({ at: date, list: [date] }),
      )).toEqual({ at: date, list: [date] })

      expect(parsePortableJson(stringifyPortableJson(date))).toEqual(date)

      // A plain ISO string stays a string — only marked Dates revive.
      expect(parsePortableJson(stringifyPortableJson({
        s: date.toISOString()
      }))).toEqual({ s: date.toISOString() })
    }
  )

  it('round-trips Buffer as Uint8Array despite Buffer.toJSON', () => {
    expect(parsePortableJson(
      stringifyPortableJson({ bytes: Buffer.from([9, 8, 7]) }),
    )).toEqual({ bytes: new Uint8Array([9, 8, 7]) })
  })

  it('parses plain JSON without markers unchanged', () => {
    expect(parsePortableJson('{"a":[1,2]}')).toEqual({
      a: [1, 2]
    })
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
