import { apiRequest, readResponse } from './client'
import { lastHourCostSchema, type LastHourCost } from './generated/web-api'

/**
 * What the conversation's calls of the last 60 minutes cost, as the backend
 * counts them from its usage ledger.
 */
export async function readLastHourCost(conversationId: string, signal?: AbortSignal): Promise<LastHourCost> {
  const response = await apiRequest(`/conversations/${encodeURIComponent(conversationId)}/cost/last-hour`, { signal })
  return readResponse(response, lastHourCostSchema)
}
