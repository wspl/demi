import type { TranscriptReveal } from '@demicodes/web-ui/agent/history'
import { apiRequest, readResponse } from '../api/client'
import { commandCallSchema } from '../api/generated/web-api'
import type { Conversation } from '../state/types'
import { createHistoryReads } from './history-reads'

/**
 * The reads of conversations' history (`web-api-conversations.md` § Conversation
 * history): a conversation's root is live while its stream is open, as
 * `live` says, and a subagent while it runs.
 */
export function createConversationHistoryReads(signal: () => AbortSignal, live: (conversation: Conversation) => boolean) {
  const reads = createHistoryReads<Conversation>(
    signal,
    (conversation, node) => node === null
      ? live(conversation)
      : conversation.subagents.find((agent) => agent.id === node)?.phase === 'running'
        || conversation.sides.find((side) => side.id === node)?.sessionPhase === 'running',
    (conversation) => `/conversations/${encodeURIComponent(conversation.id)}/transcript`,
  )
  return {
    ...reads,
    /** Where the call that started the command `commandId` lies (`web-api-conversations.md` § Subagents and commands). */
    async findCommand(conversation: Conversation, commandId: string): Promise<TranscriptReveal> {
      const response = await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/commands/${encodeURIComponent(commandId)}`,
        { signal: signal() },
      )
      const call = await readResponse(response, commandCallSchema)
      return { node: call.subagentId, blockId: call.blockId }
    },
  }
}

export type HistoryReads = ReturnType<typeof createConversationHistoryReads>
