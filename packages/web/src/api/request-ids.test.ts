import { afterEach, beforeEach, expect, test } from 'bun:test'
import { z } from 'zod'
import { waitFor } from '@demicodes/utils'
import { reportError, reportPageErrorsWith } from '@demicodes/web-ui/infra/errors'
import { ApiError, UnreadableResponse, apiRequest, readResponse } from './client'
import { sendPageErrorReport } from './page-errors'

// The page keeps the ID the backend names each request by, so that an error
// can show it, and reports its own failures, which the backend logs under
// the report's ID (`crates/backend-http/src/request_id.rs`). Answers played
// by a fetch stand-in; a few milliseconds.

const realFetch = globalThis.fetch
/** The reports the page sent, as their bodies. */
let reports: unknown[]
/** Whether the backend refuses the next report as one too many. */
let tooMany: boolean

beforeEach(() => {
  reports = []
  tooMany = false
  globalThis.fetch = (async (input, init) => {
    const request = `${init?.method ?? 'GET'} ${String(input)}`
    if (request === 'POST /api/conversations/c-1/fork') {
      return Response.json(
        { code: 'fork_conflict', message: 'Another fork is on its way', requestId: 'req_fork0000000000001' },
        { status: 409, headers: { 'X-Request-Id': 'req_fork0000000000001' } },
      )
    }
    if (request === 'GET /api/unreachable') {
      // What fetch rejects with when no connection could be made.
      throw new TypeError('Failed to fetch')
    }
    if (request === 'GET /api/plugin-state') {
      return Response.json({ title: 31337, secret: 'sk-live-value' }, { headers: { 'X-Request-Id': 'req_state000000000001' } })
    }
    if (request === 'POST /api/page-errors') {
      reports.push(JSON.parse(String(init?.body)))
      return tooMany
        ? Response.json(
            { code: 'rate_limited', message: 'Too many', requestId: 'req_refused000000001' },
            { status: 429, headers: { 'X-Request-Id': 'req_refused000000001' } },
          )
        : new Response(null, { status: 204, headers: { 'X-Request-Id': 'req_report0000000001' } })
    }
    throw new Error(`Unexpected request: ${request}`)
  }) as typeof fetch
})

afterEach(() => {
  globalThis.fetch = realFetch
})

test('a refusal carries the ID of the request it answered', async () => {
  const refused = await apiRequest('/conversations/c-1/fork', { method: 'POST' }).catch((error: unknown) => error)
  if (!(refused instanceof ApiError))
    throw new Error(`the fork was not refused: ${String(refused)}`)
  expect(refused.requestId).toBe('req_fork0000000000001')
})

test('an answer that does not read is the page’s own failure: its detail names the answer’s request and none of its values', async () => {
  const response = await apiRequest('/plugin-state')
  const unread = await readResponse(response, z.strictObject({ title: z.string() })).catch((error: unknown) => error)
  if (!(unread instanceof UnreadableResponse))
    throw new Error(`the answer read: ${String(unread)}`)
  const { detail } = unread
  expect(detail).toContain('req_state000000000001')
  expect(detail).toContain('expected string, received number')
  for (const value of ['31337', 'sk-live-value']) {
    expect(detail).not.toContain(value)
  }
})

// Only a defect is reported (`packages/web-ui/src/infra/errors.ts`): a
// request that reached no backend is the environment's failure, told on the
// console alone. Before, it was posted to /api/page-errors as a defect.
test('a request that reached no backend posts no report; a defect does', async () => {
  const stopReports = reportPageErrorsWith(sendPageErrorReport)
  try {
    const unreached = await apiRequest('/unreachable', { waits: false }).catch((error: unknown) => error)
    reportError('Could Not Read the Settings', unreached)
    // Reports go in the order of their failures: once this one is in, the others would be too.
    reportError('Could Not Open the File', new Error('handler threw'))
    await waitFor(() => reports.length > 0, () => 'the defect was not reported')
    expect(reports).toEqual([expect.objectContaining({ title: 'Could Not Open the File' })])
  } finally {
    stopReports()
  }
})

test('a report answers the ID the backend logged it under, and one refused as too many answers none', async () => {
  const report = { title: 'Could Not Read the Plugin’s Data', message: 'Invalid server response', detail: 'at title' }
  expect(await sendPageErrorReport(report)).toBe('req_report0000000001')
  tooMany = true
  expect(await sendPageErrorReport(report)).toBeNull()
  expect(reports).toEqual([report, report])
})
