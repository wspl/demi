import { apiRequest, readResponse } from './client'
import { recentCostSchema, type CostWindow, type RecentCost } from './generated/web-api'

/**
 * What the conversation's calls of the last hour or the last day cost, as the
 * backend counts them from its usage ledger.
 */
export async function readRecentCost(conversationId: string, window: CostWindow, signal?: AbortSignal): Promise<RecentCost> {
  const response = await apiRequest(`/conversations/${encodeURIComponent(conversationId)}/cost/recent?window=${window}`, { signal })
  return readResponse(response, recentCostSchema)
}
