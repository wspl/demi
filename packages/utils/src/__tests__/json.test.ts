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
