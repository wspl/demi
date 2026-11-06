import { apiRequest, jsonBody } from './client'
import type { SuggestedReplyUsed } from './generated/web-api'

/** Tells the backend the user took a suggested reply, which it keeps to measure them by. */
export async function reportSuggestedReplyUsed(conversationId: string, used: SuggestedReplyUsed): Promise<void> {
  await apiRequest(`/conversations/${encodeURIComponent(conversationId)}/suggested-replies/used`, {
    method: 'POST',
    ...jsonBody(used),
  })
}
