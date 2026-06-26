import { describe, expect, it } from 'bun:test'
import { httpErrorCode, normalizeErrorCode, providerErrorFromUnknown, redactSecretText } from '../index'

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
})

describe('providerErrorFromUnknown', () => {
  it('builds a redacted error event', () => {
    const event = providerErrorFromUnknown(new Error('boom sk-1'), 'sk-1')
    expect(event).toEqual({ type: 'error', message: 'boom [redacted]', code: null })
  })
})
