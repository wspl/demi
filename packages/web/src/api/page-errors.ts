import type { PageErrorReport } from '@demicodes/web-ui/infra/errors'
import type { PageErrorReport as PageErrorReportBody } from './generated/web-api'
import { ApiError, REQUEST_ID_HEADER, apiRequest, jsonBody } from './client'

/**
 * Sends the report of a failure the page found itself
 * (`POST /page-errors`) and answers the ID the backend logged it under, or
 * null when the backend refused it, as past thirty a minute.
 */
export async function sendPageErrorReport(report: PageErrorReport): Promise<string | null> {
  const body: PageErrorReportBody = report
  try {
    const answer = await apiRequest('/page-errors', { method: 'POST', ...jsonBody(body) })
    return answer.headers.get(REQUEST_ID_HEADER)
  } catch (error) {
    if (error instanceof ApiError) {
      return null
    }
    throw error
  }
}
