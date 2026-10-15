import { expect, test } from 'bun:test'
import { createPageI18n } from '../../i18n/text'
import { errorReportText, failureHeadline, failureSource, prettyUpstream } from '../error-detail'

// A failed turn's headline is the page's, worded from the failure's code
// and the moment the provider says it works again (`localization.md` § Text
// from the backend); the vendor's own text stays under Provider Response.
test('a failed turn’s headline is worded from its code and when it works again, in the page’s language', () => {
  const now = new Date(2026, 9, 11, 8, 15)
  const reset = new Date(2026, 9, 11, 19, 3, 55).toISOString()
  try {
    expect(failureHeadline('usage_limit', reset, now)).toBe('You’ve reached your usage limit. Resets at 7:04 PM.')
    expect(failureHeadline('rate_limit', null, now)).toBe('The provider is limiting requests.')
    createPageI18n('zh-Hans')
    expect(failureHeadline('usage_limit', reset, now)).toBe('已达到用量限额。19:04 重置。')
    createPageI18n('ja')
    expect(failureHeadline('usage_limit', reset, now)).toBe('使用制限に達しました。19:04 にリセットされます。')
  } finally {
    createPageI18n('en')
  }
})

test('a code the page has no words for reads the neutral line, and the vendor’s text shows as it came', () => {
  const now = new Date(2026, 9, 11, 8, 15)
  expect(failureHeadline('insufficient_quota_xyz', null, now)).toBe('The turn failed.')
  expect(failureHeadline(null, null, now)).toBe('The turn failed.')
  const vendor = 'OpenAI API request failed with HTTP 401: {"error":{"message":"Insufficient balance."}}'
  expect(failureSource(vendor, 'insufficient_quota_xyz', { source: 'http', httpStatus: 401 })).toEqual({ raw: vendor, detail: null })
  expect(failureSource(vendor, 'usage_limit', { source: 'http', upstream: '{"a":1}' })).toEqual({ raw: '{\n  "a": 1\n}', detail: null })
  // Demi's own failure needs no source line; one the page has no words for keeps its text.
  expect(failureSource('The turn was interrupted.', 'interrupted', undefined)).toEqual({ raw: null, detail: null })
  expect(failureSource('An older release’s words.', null, undefined)).toEqual({ raw: null, detail: 'An older release’s words.' })
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
