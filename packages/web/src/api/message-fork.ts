import { modelSelectionSchema } from '@demicodes/web-ui/transport/protocol'
import { z } from 'zod'
import type { MessageForkRequest } from '@demicodes/web-ui/agent/message-fork'
import { apiRequest, jsonBody, readResponse } from './client'
import { conversationSummarySchema } from './contracts'

export async function forkConversation(sourceId: string, request: MessageForkRequest, signal: AbortSignal) {
  const response = await apiRequest(`/conversations/${encodeURIComponent(sourceId)}/fork`, {
    method: 'POST', signal, ...jsonBody(request),
  })
  const result = await readResponse(response, z.object({ conversation: conversationSummarySchema, model: modelSelectionSchema }))
  signal.throwIfAborted()
  return result
}
