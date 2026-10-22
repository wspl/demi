import type { PageLogAnswer, PageLogBody } from '@demicodes/web-ui/infra/log'
import type { PageLog } from './generated/web-api'
import { ApiError, REQUEST_ID_HEADER, apiRequest, jsonBody } from './client'

/** The statuses by which the backend refuses lines as not well formed; any other answer leaves them to wait. */
const REFUSED = new Set([400, 413, 422])

/**
 * Sends the page's lines (`POST /log`). Answers what came of it: the lines are
 * stored, or refused as not well formed, or the page is not signed in (401) or
 * has yet to choose its password (403), or they were not delivered at all, as
 * without a network, and wait. `keepalive` is the hidden page's request, which the
 * browser finishes after the page is gone.
 */
export async function sendPageLog(body: PageLogBody, keepalive: boolean): Promise<PageLogAnswer> {
  const lines: PageLog = body
  try {
    const answer = await apiRequest('/log', { method: 'POST', keepalive, waits: false, ...jsonBody(lines) })
    return { kind: 'stored', requestId: answer.headers.get(REQUEST_ID_HEADER) }
  } catch (error) {
    if (error instanceof ApiError) {
      if (error.status === 401 || error.status === 403) {
        return { kind: 'unauthorized' }
      }
      if (REFUSED.has(error.status)) {
        return { kind: 'refused', status: error.status, requestId: error.requestId }
      }
    }
    return { kind: 'unreached' }
  }
}
