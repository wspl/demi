import { describe, expect, it } from 'bun:test'
import { parseJsonObject, parseJsonOrString, parsePortableJson, stringifyPortableJson } from '../json'
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

    const decoded = parsePortableJson<{
      metadata: { count: bigint }
      bytes: Uint8Array
      empty: Uint8Array
      one: Uint8Array
      two: Uint8Array
    }>(encoded)

    expect(decoded.metadata.count).toBe(42n)
    expect(decoded.bytes).toBeInstanceOf(Uint8Array)
    expect([...decoded.bytes]).toEqual([1, 2, 3])
    expect([...decoded.empty]).toEqual([])
    expect([...decoded.one]).toEqual([255])
    expect([...decoded.two]).toEqual([254, 253])
  })

  it('round-trips Date values, including nested, top-level, and in arrays', () => {
    const date = new Date('2026-08-31T12:34:56.789Z')
    const decoded = parsePortableJson<{ at: Date; list: Date[] }>(
      stringifyPortableJson({ at: date, list: [date] }),
    )
    expect(decoded.at).toBeInstanceOf(Date)
    expect(decoded.at.toISOString()).toBe(date.toISOString())
    expect(decoded.list[0]).toBeInstanceOf(Date)

    const top = parsePortableJson<Date>(stringifyPortableJson(date))
    expect(top).toBeInstanceOf(Date)
    expect(top.getTime()).toBe(date.getTime())

    // A plain ISO string stays a string — only marked Dates revive.
    expect(parsePortableJson<{ s: string }>(stringifyPortableJson({ s: date.toISOString() })).s).toBe(
      date.toISOString(),
    )
  })

  it('round-trips Buffer as Uint8Array despite Buffer.toJSON', () => {
    const decoded = parsePortableJson<{ bytes: Uint8Array }>(
      stringifyPortableJson({ bytes: Buffer.from([9, 8, 7]) }),
    )
    expect(decoded.bytes).toBeInstanceOf(Uint8Array)
    expect([...decoded.bytes]).toEqual([9, 8, 7])
  })

  it('parses plain JSON without markers unchanged', () => {
    expect(parsePortableJson<{ a: number[] }>('{"a":[1,2]}')).toEqual({ a: [1, 2] })
  })

  it('supports pretty-printing via the space parameter', () => {
    expect(stringifyPortableJson({ a: 1 }, 2)).toBe('{\n  "a": 1\n}')
  })
})

describe('normalizeBaseUrl', () => {
  it('strips trailing slashes', () => {
    expect(normalizeBaseUrl('https://api.example.com/')).toBe('https://api.example.com')
    expect(normalizeBaseUrl('https://api.example.com///')).toBe('https://api.example.com')
    expect(normalizeBaseUrl('https://api.example.com/v1')).toBe('https://api.example.com/v1')
  })
})
