import { apiRequest, jsonBody, readResponse } from './client'
import { shareAnswerSchema, type ShareAnswer, type ShareTarget } from './generated/web-api'

/** A conversation's owner routes for its shares. */
function sharesPath(conversationId: string): string {
  return `/conversations/${encodeURIComponent(conversationId)}/shares`
}

/** The conversation's or an answer's share: the existing one, or a new snapshot. */
export async function createShare(conversationId: string, target: ShareTarget): Promise<ShareAnswer> {
  const response = await apiRequest(sharesPath(conversationId), { method: 'POST', ...jsonBody(target satisfies ShareTarget) })
  return readResponse(response, shareAnswerSchema)
}

/** Takes the conversation's snapshot again under the same token. */
export async function updateShare(conversationId: string, token: string): Promise<ShareAnswer> {
  const response = await apiRequest(`${sharesPath(conversationId)}/${encodeURIComponent(token)}`, { method: 'PUT' })
  return readResponse(response, shareAnswerSchema)
}

export async function removeShare(conversationId: string, token: string): Promise<void> {
  await apiRequest(`${sharesPath(conversationId)}/${encodeURIComponent(token)}`, { method: 'DELETE' })
}
