import { expect, test } from 'bun:test'
import { errorFacts, errorPresentation, errorReportText } from '../error-detail'

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

test('the facts lead with when the vendor says it works again, read from the stored payload', () => {
  const upstream = JSON.stringify({ type: 'error', error: { resets_at: 1790062659 }, status_code: 429 })
  const facts = errorFacts('rate_limit', { source: 'stream', httpStatus: 429, upstream }, '2026-09-18T14:00:00.000Z')
  expect(facts[0]).toBe(`resets ${new Date(1790062659 * 1000).toLocaleString()}`)
  expect(facts.slice(1)).toEqual(['HTTP 429', 'rate_limit'])
  // Without a named time, or without the record's time for a relative wait, nothing is claimed.
  expect(errorFacts('rate_limit', { source: 'stream', upstream: '{"error":{}}' }, '2026-09-18T14:00:00.000Z')).toEqual(['rate_limit'])
})

test('the copied report carries the vendor payload, indented', () => {
  const report = errorReportText('The usage limit has been reached', 'rate_limit', {
    source: 'stream',
    upstream: '{"error":{"plan_type":"pro"}}',
  })
  expect(report).toContain('upstream:\n{\n  "error": {\n    "plan_type": "pro"\n  }\n}')
})
