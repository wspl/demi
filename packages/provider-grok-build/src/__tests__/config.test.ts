import { expect, test } from 'bun:test'
import { parseGrokBuildProviderConfig } from '../index'

test('public Grok configuration validates its complete serializable input', () => {
  expect(parseGrokBuildProviderConfig(undefined)).toEqual({})
  expect(parseGrokBuildProviderConfig(null)).toEqual({})
  expect(parseGrokBuildProviderConfig({
    grokHome: '/tmp/synthetic-grok', baseUrl: 'https://example.test/v1',
    headers: { 'x-test': 'value' },
  })).toEqual({
    grokHome: '/tmp/synthetic-grok', baseUrl: 'https://example.test/v1',
    headers: { 'x-test': 'value' },
  })
  for (const invalid of [
    1, [], { grokHome: null }, { grokHome: '' }, { baseUrl: 12 },
    { baseUrl: 'file:///private' }, { headers: [] }, { headers: { key: 1 } },
    { headers: null }, { baseURL: 'https://example.test' }, { fetch: () => {} },
  ]) {
    expect(() => parseGrokBuildProviderConfig(invalid)).toThrow()
  }
})
