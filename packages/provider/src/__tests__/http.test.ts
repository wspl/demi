import { describe, expect, it } from 'bun:test'
import {
  authStatusFromKey,
  httpErrorCode,
  httpRequestFailedEvent,
  normalizeErrorCode,
  providerErrorFromUnknown,
  redactSecretText,
} from '../index'

describe('redactSecretText', () => {
  it('masks the secret and is a no-op without one', () => {
    expect(redactSecretText('key=sk-123 here', 'sk-123')).toBe('key=[redacted] here')
    expect(redactSecretText('no secret', null)).toBe('no secret')
  })
})

describe('httpErrorCode', () => {
  it('classifies by status', () => {
    expect(httpErrorCode(401, '')).toBe('auth_expired')
    expect(httpErrorCode(429, '')).toBe('rate_limit')
    expect(httpErrorCode(500, '')).toBe('rate_limit')
    expect(httpErrorCode(400, 'context length exceeded')).toBe('context_length_exceeded')
    expect(httpErrorCode(400, 'bad request')).toBeNull()
    expect(httpErrorCode(404, '')).toBeNull()
  })
})

describe('normalizeErrorCode', () => {
  it('categorizes by message keywords, falling back to code', () => {
    expect(normalizeErrorCode(null, 'maximum context length')).toBe('context_length_exceeded')
    expect(normalizeErrorCode(null, 'rate limit reached')).toBe('rate_limit')
    expect(normalizeErrorCode(null, 'invalid api key')).toBe('auth_expired')
    expect(normalizeErrorCode(null, 'service unavailable')).toBe('overloaded')
    expect(normalizeErrorCode('custom', 'something else')).toBe('custom')
  })

  it('classifies transport-level transient failures as overloaded', () => {
    expect(normalizeErrorCode(null, 'Codex SSE response headers timed out after 20000ms')).toBe('overloaded')
    expect(normalizeErrorCode(null, 'connect timeout')).toBe('overloaded')
    expect(normalizeErrorCode(null, 'fetch failed')).toBe('overloaded')
    expect(normalizeErrorCode(null, 'socket hang up')).toBe('overloaded')
    expect(normalizeErrorCode(null, 'read ECONNRESET')).toBe('overloaded')
    expect(normalizeErrorCode(null, 'insufficient balance')).toBe('rate_limit')
  })
})

describe('providerErrorFromUnknown', () => {
  it('builds a redacted error event', () => {
    const event = providerErrorFromUnknown(new Error('boom sk-1'), 'sk-1')
    expect(event).toEqual({ type: 'error', message: 'boom [redacted]', code: null })
  })
})

describe('authStatusFromKey', () => {
  it('authenticates with a key', async () => {
    expect(await authStatusFromKey(() => 'sk-1', undefined, 'x-api-key', 'Acme')).toEqual({ status: 'authenticated' })
  })

  it('authenticates when a matching auth header is present', async () => {
    const status = await authStatusFromKey(() => null, () => ({ Authorization: 'Bearer x' }), 'authorization', 'Acme')
    expect(status).toEqual({ status: 'authenticated' })
  })

  it('reports a labeled message when unauthenticated', async () => {
    expect(await authStatusFromKey(() => null, undefined, 'x-api-key', 'Acme')).toEqual({
      status: 'unauthenticated',
      message: 'Acme API key is missing',
    })
  })
})

describe('httpRequestFailedEvent', () => {
  it('builds a labeled, redacted, classified error event', async () => {
    const event = await httpRequestFailedEvent(new Response('nope sk-1', { status: 401 }), 'sk-1', 'Acme')
    expect(event).toEqual({
      type: 'error',
      message: 'Acme API request failed with HTTP 401: nope [redacted]',
      code: 'auth_expired',
    })
  })
})
