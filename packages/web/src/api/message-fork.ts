import type { MessageForkRequest } from '@demicodes/web-ui/agent/message-fork'
import { apiRequest, jsonBody, readResponse } from './client'
import { forkAnswerSchema, type ForkRequest } from './generated/web-api'

export async function forkConversation(sourceId: string, request: MessageForkRequest, signal: AbortSignal) {
  const response = await apiRequest(`/conversations/${encodeURIComponent(sourceId)}/fork`, {
    method: 'POST', signal, ...jsonBody(request satisfies ForkRequest),
  })
  const result = await readResponse(response, forkAnswerSchema)
  signal.throwIfAborted()
  return result
}
