import { expect, test } from 'bun:test'
import { createProviderSchema, nicknamePatchSchema } from './generated/web-api'

// Where the page sends a value, its generated schema refuses what the backend
// refuses (`contracts.md` § Strict and tolerant objects), with the cases of
// web-api's own tests of `Trimmed` and `EndpointUrl` (`text.rs`).

test('text the backend trims is trimmed before its length is checked', () => {
  expect(nicknamePatchSchema.safeParse({ nickname: ' \t ' }).success).toBe(false)
  expect(nicknamePatchSchema.parse({ nickname: `﻿  ${'a'.repeat(80)} \t` })).toEqual({ nickname: 'a'.repeat(80) })
  expect(nicknamePatchSchema.safeParse({ nickname: 'a'.repeat(81) }).success).toBe(false)
  // U+0085 is not white space to JavaScript's trim, nor to the backend's.
  expect(nicknamePatchSchema.parse({ nickname: '\u0085' })).toEqual({ nickname: '\u0085' })
})

test('an endpoint is an http or https URL', () => {
  const accepts = (baseUrl: string) =>
    createProviderSchema.safeParse({ source: 'custom', providerType: 'openai', label: 'Local', apiKey: 'key', baseUrl })
      .success
  expect(accepts('https://api.kimi.com/coding/v1')).toBe(true)
  expect(accepts('http://127.0.0.1:8080')).toBe(true)
  for (const refused of ['', 'api.openai.com/v1', 'ftp://example.test/', 'file:///etc', 'https://']) {
    expect({ refused, accepted: accepts(refused) }).toEqual({ refused, accepted: false })
  }
})
