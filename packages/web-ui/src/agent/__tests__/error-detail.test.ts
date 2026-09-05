import { expect, test } from 'bun:test'
import { errorFacts, errorReportText, errorSummary } from '../error-detail'

test('the chrome sentence follows the normalized code', () => {
  expect(errorSummary('rate_limit')).toBe('Rate limited by the provider')
  expect(errorSummary('overloaded')).toBe('The provider is overloaded or unreachable')
  expect(errorSummary('auth_expired')).toBe('Authentication expired')
  expect(errorSummary('auth_missing')).toBe('No credentials for this provider')
  expect(errorSummary('context_length_exceeded')).toBe('The context is too long for this model')
  expect(errorSummary(null)).toBe('The provider request failed')
  expect(errorSummary('something_else')).toBe('The provider request failed')
})

test('the facts line carries the status, the code, and the request id when present', () => {
  expect(errorFacts('rate_limit', { source: 'http', httpStatus: 429, clientRequestId: 'req_1' })).toEqual(['HTTP 429', 'rate_limit', 'req_1'])
  expect(errorFacts(null, { source: 'stream' })).toEqual([])
  expect(errorFacts(null, undefined)).toEqual([])
})

test('the report is the upstream message followed by every diagnostic', () => {
  expect(errorReportText('boom', 'rate_limit', { source: 'http', httpStatus: 429, providerCode: 'rate_limit_error' }))
    .toBe('boom\ncode: rate_limit\nsource: http\nhttp: 429\nprovider code: rate_limit_error')
  expect(errorReportText('boom', null, undefined)).toBe('boom')
})
