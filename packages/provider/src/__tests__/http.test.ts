import { describe, expect, it } from 'bun:test'
import {
  authStatusFromKey,
  httpErrorCode,
  httpFailureRecord,
  httpRequestFailedEvent,
  normalizeErrorCode,
  providerErrorFromUnknown,
  readHttpFailure,
  retryAtFromHeader,
  withRetryWait,
  type ProviderEvent,
} from '../index'

describe('httpErrorCode', () => {
  it('classifies by status', () => {
    expect(httpErrorCode(401, '')).toBe('auth_expired')
    expect(httpErrorCode(429, '')).toBe('rate_limit')
    expect(httpErrorCode(408, '')).toBe('overloaded')
    expect(httpErrorCode(500, '')).toBe('overloaded')
    expect(httpErrorCode(400, 'context length exceeded'))
      .toBe('context_length_exceeded')
    expect(httpErrorCode(400, 'bad request')).toBeNull()
    expect(httpErrorCode(404, '')).toBeNull()
  })
})

describe('normalizeErrorCode', () => {
  it('categorizes by message keywords, falling back to code', () => {
    expect(normalizeErrorCode(null, 'maximum context length'))
      .toBe('context_length_exceeded')
    expect(normalizeErrorCode(null, 'rate limit reached')).toBe('rate_limit')
    expect(normalizeErrorCode(null, 'invalid api key')).toBe('auth_expired')
    expect(normalizeErrorCode(
      'invalid_request_error',
      'Invalid prompt_cache_key'
    )).toBe('invalid_request_error')
    expect(normalizeErrorCode(null, 'service unavailable')).toBe('overloaded')
    expect(normalizeErrorCode('server_error', 'backend failed'))
      .toBe('overloaded')
    expect(normalizeErrorCode('internal-error', 'backend failed'))
      .toBe('overloaded')
    expect(normalizeErrorCode('custom', 'something else')).toBe('custom')
  })

  it('classifies transport-level transient failures as overloaded', () => {
    expect(normalizeErrorCode(
      null,
      'Codex SSE response headers timed out after 20000ms'
    )).toBe('overloaded')
    expect(normalizeErrorCode(null, 'connect timeout')).toBe('overloaded')
    expect(normalizeErrorCode(null, 'fetch failed')).toBe('overloaded')
    expect(normalizeErrorCode(null, 'socket hang up')).toBe('overloaded')
    expect(normalizeErrorCode(null, 'read ECONNRESET')).toBe('overloaded')
    expect(normalizeErrorCode(null, 'insufficient balance')).toBe('rate_limit')
  })
})

describe('providerErrorFromUnknown', () => {
  it('builds an error event with no failure record: the vendor never answered', () => {
    const event = providerErrorFromUnknown(new Error('fetch failed'))
    expect(event).toEqual({
      type: 'error',
      message: 'fetch failed',
      code: 'overloaded'
    })
  })
})

describe('authStatusFromKey', () => {
  it('authenticates with a key', async () => {
    expect(
      await authStatusFromKey(() => 'sk-1', undefined, 'x-api-key', 'Acme')
    ).toEqual(
      {
        status: 'authenticated'
      }
    )
  })

  it('authenticates when a matching auth header is present', async () => {
    const status = await authStatusFromKey(
      () => null,
      () => ({ Authorization: 'Bearer x' }),
      'authorization',
      'Acme'
    )
    expect(status).toEqual({ status: 'authenticated' })
  })

  it('reports a labeled message when unauthenticated', async () => {
    expect(
      await authStatusFromKey(() => null, undefined, 'x-api-key', 'Acme')
    ).toEqual(
      {
        status: 'unauthenticated',
        message: 'Acme API key is missing',
      }
    )
  })
})

describe('httpRequestFailedEvent', () => {
  it('keeps the vendor text in the message and the whole response as the record', async () => {
    const response = new Response('{"error":{"message":"slow down"}}', {
      status: 429,
      headers: { 'retry-after': '120', 'x-request-id': 'req-9', 'set-cookie': 'a=b' },
    })
    const event = await httpRequestFailedEvent(response, 'Acme', readHttpFailure)
    if (event.type !== 'error')
      throw new Error('expected an error event')
    expect(event.message).toBe('Acme API request failed with HTTP 429: {"error":{"message":"slow down"}}')
    expect(event.code).toBe('rate_limit')
    expect(event.diagnostics?.httpStatus).toBe(429)
    const record = JSON.parse(event.diagnostics!.upstream!)
    // Every header, the body as the vendor wrote it: nothing parsed, filtered or redacted.
    expect(record.status).toBe(429)
    expect(record.body).toBe('{"error":{"message":"slow down"}}')
    expect(record.headers).toContainEqual(['retry-after', '120'])
    expect(record.headers).toContainEqual(['x-request-id', 'req-9'])
    expect(record.headers).toContainEqual(['set-cookie', 'a=b'])
    // The wait comes from the provider's reader over that record.
    expect(event.retryAfterMs).toBe(120_000)
  })

  it('keeps a body that is not JSON as it came, with no wait when no header names one', async () => {
    const event = await httpRequestFailedEvent(
      new Response('<html>Bad gateway</html>', { status: 502 }),
      'Acme',
      readHttpFailure,
    )
    if (event.type !== 'error')
      throw new Error('expected an error event')
    expect(JSON.parse(event.diagnostics!.upstream!).body).toBe('<html>Bad gateway</html>')
    expect(event.retryAfterMs).toBeUndefined()
  })
})

describe('reading a failure record', () => {
  const receivedAt = '2026-09-18T14:00:00.000Z'
  const record = (headers: Record<string, string>) => ({
    source: 'http' as const,
    upstream: httpFailureRecord(429, new Headers(headers), 'slow down'),
  })

  it('reads Retry-After in both spellings, counting a delay from when the failure was recorded', () => {
    const received = Date.parse(receivedAt)
    expect(retryAtFromHeader('120', received)).toBe(received + 120_000)
    expect(retryAtFromHeader('Tue, 22 Sep 2026 07:37:39 GMT', received)).toBe(Date.parse('2026-09-22T07:37:39Z'))
    expect(retryAtFromHeader('soon', received)).toBeNull()
    expect(retryAtFromHeader(null, received)).toBeNull()
    expect(readHttpFailure(record({ 'Retry-After': '90' }), receivedAt)).toEqual({ retryAt: '2026-09-18T14:01:30.000Z' })
  })

  it('names no time without the header, for a stream record, or for a record it cannot read', () => {
    expect(readHttpFailure(record({}), receivedAt)).toEqual({ retryAt: null })
    expect(readHttpFailure({ source: 'stream', upstream: '{"retry-after":"90"}' }, receivedAt)).toEqual({ retryAt: null })
    expect(readHttpFailure({ source: 'http', upstream: 'not a record' }, receivedAt)).toEqual({ retryAt: null })
  })

  it('sets the retry wait only on an error event whose reader names a time', () => {
    const failed: ProviderEvent = { type: 'error', message: 'x', code: null, diagnostics: { source: 'stream', upstream: 'x' } }
    const later = new Date(Date.now() + 30_000).toISOString()
    const waited = withRetryWait(failed, () => ({ retryAt: later }))
    expect(waited.type === 'error' && waited.retryAfterMs).toBeGreaterThan(25_000)
    expect(withRetryWait(failed, () => ({ retryAt: null }))).toBe(failed)
    const bare: ProviderEvent = { type: 'error', message: 'x', code: null }
    expect(withRetryWait(bare, () => ({ retryAt: later }))).toBe(bare)
  })
})
