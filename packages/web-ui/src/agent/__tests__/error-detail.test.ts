import { expect, test } from 'bun:test'
import { errorFacts, errorReportText, errorSummary } from '../error-detail'

test('the chrome sentence states only what Demi knows; codes derived from a vendor never head a failure', () => {
  for (const code of ['rate_limit', 'overloaded', 'auth_expired', 'context_length_exceeded', 'something_else', null])
    expect(errorSummary(code)).toBe('The provider request failed')
  expect(errorSummary('auth_missing')).toBe('No credentials for this provider')
  expect(errorSummary('credential_not_found')).toBe('No credentials for this provider')
})

test('the facts line carries the status, the code, and the request id when present', () => {
  expect(errorFacts(
      'rate_limit',
      {
        source: 'http',
        httpStatus: 429,
        clientRequestId: 'req_1'
      }
    )).toEqual(
    ['HTTP 429', 'rate_limit', 'req_1']
  )
  expect(errorFacts(null, { source: 'stream' })).toEqual([])
  expect(errorFacts(null, undefined)).toEqual([])
})

test('the report is the upstream message followed by every diagnostic', () => {
  expect(
    errorReportText(
      'boom',
      'rate_limit',
      {
        source: 'http',
        httpStatus: 429,
        providerCode: 'rate_limit_error'
      }
    )
  )
    .toBe('boom\ncode: rate_limit\nsource: http\nhttp: 429\nprovider code: rate_limit_error')
  expect(errorReportText('boom', null, undefined)).toBe('boom')
})
