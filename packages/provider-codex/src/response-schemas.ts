import { z } from 'zod'
import { parseProviderData, parseProviderJson, responsesEventSchema, type ResponsesStreamEvent } from '@demicodes/provider'

const webSocketEnvelopeSchema = z.looseObject({
  type: z.string().optional(),
  event: z.unknown().optional(),
})

export function parseCodexWebSocketEvent(text: string): ResponsesStreamEvent {
  const envelope = parseProviderJson(webSocketEnvelopeSchema, text, 'Codex WebSocket envelope')
  if (envelope.type === 'response.done') {
    return parseProviderData(responsesEventSchema, {
      type: 'response.completed', response: envelope.response,
    }, 'Codex WebSocket response')
  }
  return parseProviderData(
    responsesEventSchema,
    envelope.event === undefined ? envelope : envelope.event,
    'Codex WebSocket event'
  )
}
