import { afterEach, beforeEach, expect, test } from 'bun:test'
import { z } from 'zod'
import { waitFor } from '@demicodes/utils'
import { reportError, reportPageErrorsWith } from '@demicodes/web-ui/infra/errors'
import { ApiError, UnreadableResponse, apiRequest, readResponse } from './client'
import { PAGE_ID, logPageError, logPageWith, pageLogSignedIn } from '@demicodes/web-ui/infra/log'
import { sendPageLog } from './page-log'

// The page keeps the ID the backend names each request by, so that an error
// can show it, and writes its own failures as error lines of its log, which
// the backend stores. Answers played by a fetch stand-in; a few milliseconds.

const realFetch = globalThis.fetch
/** The fields of the error lines the page sent. */
let reports: unknown[]
/** The status the backend answers the next lines with, if not 204. */
let refusal: number | null

beforeEach(() => {
  reports = []
  refusal = null
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
    if (request === 'POST /api/log') {
      const body = JSON.parse(String(init?.body)) as { lines: Array<{ fields: unknown }> }
      reports.push(...body.lines.map((line) => line.fields))
      return refusal !== null
        ? Response.json(
            { code: 'bad_request', message: 'Not well formed', requestId: 'req_refused000000001' },
            { status: refusal, headers: { 'X-Request-Id': 'req_refused000000001' } },
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
// console alone.
test('a request that reached no backend writes no error line; a defect does', async () => {
  const stopLog = logPageWith(sendPageLog)
  pageLogSignedIn(true)
  const stopReports = reportPageErrorsWith(logPageError)
  try {
    const unreached = await apiRequest('/unreachable', { waits: false }).catch((error: unknown) => error)
    reportError('Could Not Read the Settings', unreached)
    // Reports go in the order of their failures: once this one is in, the others would be too.
    reportError('Could Not Open the File', new Error('handler threw'))
    await waitFor(() => reports.length > 0, () => 'the defect was not reported')
    expect(reports).toEqual([expect.objectContaining({ title: 'Could Not Open the File' })])
  } finally {
    stopReports()
    stopLog()
  }
})

test('lines answer the ID of the request that carried them, a refusal its status, and a signed-out page that it is not signed in', async () => {
  const body = {
    page: PAGE_ID,
    lines: [{ at: 1, level: 'error' as const, event: 'error.reported', fields: { title: 'Could Not Read the Plugin’s Data' } }],
  }
  expect(await sendPageLog(body, false)).toEqual({ kind: 'stored', requestId: 'req_report0000000001' })
  refusal = 400
  expect(await sendPageLog(body, false)).toEqual({ kind: 'refused', status: 400, requestId: 'req_refused000000001' })
  refusal = 401
  expect(await sendPageLog(body, false)).toEqual({ kind: 'unauthorized' })
  expect(reports).toHaveLength(3)
})
