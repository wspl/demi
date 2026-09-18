import { expect, test } from 'bun:test'
import { errorFacts, errorPresentation, errorReportText, prettyUpstream } from '../error-detail'

test('a plain sentence from the source is the whole record', () => {
  expect(errorPresentation('The usage limit has been reached')).toEqual({
    label: 'The usage limit has been reached',
    detail: null,
  })
  expect(errorPresentation('The agent session was shut down while this turn was running.')).toEqual({
    label: 'The agent session was shut down while this turn was running.',
    detail: null,
  })
})

test('a wrapped vendor body leads with the sentence inside it and keeps the full text', () => {
  const credits = 'OpenAI API request failed with HTTP 401: {"type":"error","error":{"type":"CreditsError","message":"Insufficient balance."}}'
  expect(errorPresentation(credits)).toEqual({ label: 'Insufficient balance.', detail: credits })
  const plan = 'OpenAI API request failed with HTTP 429: {"error":{"code":"1311","message":"当前订阅套餐暂未开放GLM-5.3-Highspeed权限"}}'
  expect(errorPresentation(plan)).toEqual({ label: '当前订阅套餐暂未开放GLM-5.3-Highspeed权限', detail: plan })
  expect(errorPresentation('Upstream said: {"error":"model overloaded"}').label).toBe('model overloaded')
  expect(errorPresentation('Gateway: {"message":"try later"}').label).toBe('try later')
})

test('a message with no sentence to lead with gets the neutral line over its text', () => {
  const long = `Request failed: ${'x'.repeat(300)}`
  expect(errorPresentation(long)).toEqual({ label: 'The turn failed', detail: long })
  expect(errorPresentation('first line\nsecond line')).toEqual({
    label: 'The turn failed',
    detail: 'first line\nsecond line',
  })
  expect(errorPresentation('HTTP 500: {"unexpected":true}')).toEqual({
    label: 'HTTP 500: {"unexpected":true}',
    detail: null,
  })
  expect(errorPresentation('HTTP 500: {not json')).toEqual({ label: 'HTTP 500: {not json', detail: null })
  expect(errorPresentation('')).toEqual({ label: 'The turn failed', detail: null })
})

test('the facts line leaves the status, the code and the request ids to the report', () => {
  expect(errorFacts({ source: 'http', httpStatus: 429, clientRequestId: 'req_1' })).toEqual([])
  expect(errorFacts(undefined)).toEqual([])
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

test('the facts lead with when the vendor says it works again, read from the stored payload', () => {
  const upstream = JSON.stringify({ type: 'error', error: { resets_at: 1790062659 }, status_code: 429 })
  expect(errorFacts(
    { source: 'stream', httpStatus: 429, clientRequestId: 'req_1', upstream },
    '2026-09-18T14:00:00.000Z',
  )).toEqual([`resets ${new Date(1790062659 * 1000).toLocaleString()}`])
  // Without a named time nothing is claimed.
  expect(errorFacts({ source: 'stream', upstream: '{"error":{}}' }, '2026-09-18T14:00:00.000Z')).toEqual([])
})

test('the reader sees the vendor payload without its transport headers; the report keeps them', () => {
  const upstream = JSON.stringify({ type: 'error', error: { plan_type: 'pro' }, headers: { 'X-Codex-Plan-Type': 'pro' } })
  expect(prettyUpstream(upstream, { headers: false })).toBe('{\n  "type": "error",\n  "error": {\n    "plan_type": "pro"\n  }\n}')
  expect(prettyUpstream(upstream, { headers: true })).toContain('X-Codex-Plan-Type')
  expect(prettyUpstream('plain text', { headers: false })).toBe('plain text')
  expect(prettyUpstream('[1,2]', { headers: false })).toBe('[\n  1,\n  2\n]')
})

test('the copied report carries the vendor payload, indented', () => {
  const report = errorReportText('The usage limit has been reached', 'rate_limit', {
    source: 'stream',
    upstream: '{"error":{"plan_type":"pro"}}',
  })
  expect(report).toContain('upstream:\n{\n  "error": {\n    "plan_type": "pro"\n  }\n}')
})
