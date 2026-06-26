import { describe, expect, it } from 'bun:test'
import { parseJsonObject, parseJsonOrString } from '../json'
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

describe('normalizeBaseUrl', () => {
  it('strips trailing slashes', () => {
    expect(normalizeBaseUrl('https://api.example.com/')).toBe('https://api.example.com')
    expect(normalizeBaseUrl('https://api.example.com///')).toBe('https://api.example.com')
    expect(normalizeBaseUrl('https://api.example.com/v1')).toBe('https://api.example.com/v1')
  })
})
