import { expect, test } from 'bun:test'
import { createPageI18n } from '../../i18n/text'
import { errorReportText, failureHeadline, failureSource, prettyUpstream, recordHeadline } from '../error-detail'
import { notExposedOffer, signedOutOffer } from '../session-status'

// A failed turn's headline is the page's, worded from the failure's code
// and the moment the provider says it works again; the vendor's own text stays
// under Provider Response.
test('a failed turn’s headline is worded from its code and when it works again, in the page’s language', () => {
  const now = new Date(2026, 9, 11, 8, 15)
  const reset = new Date(2026, 9, 11, 19, 3, 55).toISOString()
  try {
    expect(failureHeadline('usage_limit', reset, now)).toBe('You’ve reached your usage limit. Resets at 7:04 PM.')
    expect(failureHeadline('rate_limit', null, now)).toBe('The provider is limiting requests.')
    createPageI18n('zh-Hans')
    expect(failureHeadline('usage_limit', reset, now)).toBe('已达到用量限额。19:04 重置。')
    createPageI18n('ja')
    expect(failureHeadline('usage_limit', reset, now)).toBe('利用上限に達しました。19:04 にリセットされます。')
  } finally {
    createPageI18n('en')
  }
})

test('a code the page has no words for reads the neutral line, and the vendor’s text shows as it came', () => {
  const now = new Date(2026, 9, 11, 8, 15)
  expect(failureHeadline('insufficient_quota_xyz', null, now)).toBe('The turn failed.')
  expect(failureHeadline(null, null, now)).toBe('The turn failed.')
  const vendor = 'OpenAI API request failed with HTTP 401: {"error":{"message":"Insufficient balance."}}'
  expect(failureSource(vendor, 'insufficient_quota_xyz', { source: 'http', httpStatus: 401 })).toEqual({ raw: vendor, headers: null, detail: null })
  expect(failureSource(vendor, 'usage_limit', { source: 'stream', upstream: '{"a":1}' })).toEqual({ raw: '{\n  "a": 1\n}', headers: null, detail: null })
  // Demi's own failure needs no source line; one the page has no words for keeps its text.
  expect(failureSource('The turn was interrupted.', 'interrupted', undefined)).toEqual({ raw: null, headers: null, detail: null })
  expect(failureSource('An older release’s words.', null, undefined)).toEqual({ raw: null, headers: null, detail: 'An older release’s words.' })
})

// The vendor's reason is what the reader opens Provider Response for, so
// an http record leads with its status and body and keeps its headers
// apart, folded; Copy still carries the whole record.
test('an http record shows its status and the vendor’s reason first, and its headers apart', () => {
  const body = '{"error":{"message":"Your Go plan doesn\'t include API access.","code":"upgrade_required"}}'
  const upstream = JSON.stringify({ status: 403, headers: [['cf-ray', 'a4c1'], ['content-type', 'application/json']], body })
  const diagnostics = { source: 'http', httpStatus: 403, upstream } as const
  expect(failureSource('OpenAI API request failed with HTTP 403', 'permission_denied', diagnostics)).toEqual({
    raw: 'HTTP 403\n\n{\n  "error": {\n    "message": "Your Go plan doesn\'t include API access.",\n    "code": "upgrade_required"\n  }\n}',
    headers: 'cf-ray: a4c1\ncontent-type: application/json',
    detail: null,
  })
  expect(errorReportText('boom', 'permission_denied', diagnostics)).toContain('"cf-ray"')
  // A body that is not JSON shows as it came; an empty one leaves the status.
  const plain = JSON.stringify({ status: 403, headers: [], body: 'Forbidden' })
  expect(failureSource('x', 'permission_denied', { source: 'http', upstream: plain }).raw).toBe('HTTP 403\n\nForbidden')
  const empty = JSON.stringify({ status: 403, headers: [], body: '' })
  expect(failureSource('x', 'permission_denied', { source: 'http', upstream: empty })).toEqual({ raw: 'HTTP 403', headers: null, detail: null })
})

// A 403 says the vendor knows the credential and refuses the request, so
// the record never sends the user to sign in again, and an API key entry's
// refused credential is its key, never a sign-in; the vendor's own words
// stay under Provider Response.
test('a key entry’s 403 is a refused request and its 401 a refused key, with the vendor’s text below', () => {
  const now = new Date(2026, 9, 17, 21, 42)
  const onKey = { retryAt: null, signedOut: null, notExposed: null, deviceOffline: null, credential: 'api_key' } as const
  const plan = 'OpenAI API request failed with HTTP 403: {"error":{"message":"Your Go plan doesn\'t include API access.","code":"upgrade_required"}}'
  expect(recordHeadline('permission_denied', onKey, now)).toBe('The provider refused this request.')
  expect(failureSource(plan, 'permission_denied', { source: 'http', httpStatus: 403 }).raw).toBe(plan)
  expect(recordHeadline('auth_expired', onKey, now)).toBe('The provider refused the API key.')
  try {
    createPageI18n('zh-Hans')
    expect(recordHeadline('permission_denied', onKey, now)).toBe('供应商拒绝了此请求。')
    expect(recordHeadline('auth_expired', onKey, now)).toBe('供应商拒绝了 API 密钥。')
  } finally {
    createPageI18n('en')
  }
})

test('a subscription’s refused sign-in names the signed-out account, and the dock offers Open Providers', () => {
  const now = new Date(2026, 9, 17, 21, 42)
  const signedOut = { provider: 'Claude Code', account: 'alex@example.com' }
  expect(recordHeadline('auth_expired', { retryAt: null, signedOut, notExposed: null, deviceOffline: null, credential: 'subscription' }, now))
    .toBe('Claude Code signed alex@example.com out. Sign it in again in Settings › Providers.')
  expect(signedOutOffer('idle', [{ type: 'error', id: 'e1', code: 'auth_expired' }], { e1: { signedOut } })).toBe(true)
  // Signed in again since, it reads as the sign-in failure, never a key's.
  expect(recordHeadline('auth_expired', { retryAt: null, signedOut: null, notExposed: null, deviceOffline: null, credential: 'subscription' }, now))
    .toBe('The provider’s sign-in expired or was refused.')
})

test('a request to a port the device has not exposed names the port and the device while it still is not', () => {
  const now = new Date(2026, 9, 17, 21, 42)
  const notExposed = { device: 'alex-mbp', port: 11434 }
  expect(recordHeadline('port_not_exposed', { retryAt: null, signedOut: null, notExposed, deviceOffline: null, credential: 'api_key' }, now))
    .toBe('Port 11434 of alex-mbp isn’t exposed.')
  // Exposed since, the record keeps the code's own words.
  expect(recordHeadline('port_not_exposed', { retryAt: null, signedOut: null, notExposed: null, deviceOffline: null, credential: 'api_key' }, now))
    .toBe('The device’s port wasn’t exposed.')
})

test('a port exposed since the failure, from Settings or another tab, is no longer offered to expose', () => {
  const blocks = [{ type: 'error', id: 'e1', code: 'port_not_exposed' }]
  const named = { deviceId: 'mac', device: 'alex-mbp', port: 11434 }
  const failures = { e1: { notExposed: named } }
  expect(notExposedOffer('idle', blocks, failures, () => false)).toEqual(named)
  expect(notExposedOffer('idle', blocks, failures, (device, port) => device === 'mac' && port === 11434)).toBeNull()
})

test('a request to a device that was offline names the device', () => {
  const now = new Date(2026, 9, 17, 21, 42)
  const facts = { retryAt: null, signedOut: null, notExposed: null, deviceOffline: { device: 'alex-mbp' }, credential: 'api_key' } as const
  expect(recordHeadline('device_offline', facts, now)).toBe('alex-mbp is offline.')
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

test('the provider response shows whole, indented when it is JSON', () => {
  const upstream = JSON.stringify({ type: 'error', error: { plan_type: 'pro' }, headers: { 'X-Codex-Plan-Type': 'pro' } })
  expect(prettyUpstream(upstream)).toBe('{\n  "type": "error",\n  "error": {\n    "plan_type": "pro"\n  },\n  "headers": {\n    "X-Codex-Plan-Type": "pro"\n  }\n}')
  expect(prettyUpstream('plain text')).toBe('plain text')
  expect(prettyUpstream('[1,2]')).toBe('[\n  1,\n  2\n]')
})

test('the copied report carries the vendor payload, indented', () => {
  const report = errorReportText('The usage limit has been reached', 'usage_limit', {
    source: 'stream',
    upstream: '{"error":{"plan_type":"pro"}}',
  })
  expect(report).toContain('upstream:\n{\n  "error": {\n    "plan_type": "pro"\n  }\n}')
})
