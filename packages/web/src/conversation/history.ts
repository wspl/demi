import type { TranscriptReveal } from '@demicodes/web-ui/agent/history'
import { apiRequest, readResponse } from '../api/client'
import { commandCallSchema } from '../api/generated/web-api'
import type { Conversation } from '../state/types'
import { readNodeBlocks } from '../state/offline'
import { createHistoryReads } from './history-reads'

/**
 * The reads of conversations' history: a conversation's root is live while its
 * stream is open, as `live` says, and a subagent while it runs. A page before
 * a window comes from the disk copy of `userId`'s when it holds it.
 */
export function createConversationHistoryReads(
  signal: () => AbortSignal,
  live: (conversation: Conversation) => boolean,
  userId: () => string | null,
) {
  const reads = createHistoryReads<Conversation>(
    signal,
    (conversation, node) => node === null
      ? live(conversation)
      : conversation.subagents.find((agent) => agent.id === node)?.phase === 'running'
        || conversation.sides.find((side) => side.id === node)?.sessionPhase === 'running',
    (conversation, node) => `/conversations/${encodeURIComponent(conversation.id)}/agents/${encodeURIComponent(node ?? conversation.id)}`,
    async (conversation, node, end, edge) => {
      const user = userId()
      return user ? readNodeBlocks(user, node ?? conversation.id, end, edge) : null
    },
  )
  return {
    ...reads,
    /** Where the call that started the command `commandId` lies. */
    async findCommand(conversation: Conversation, commandId: string): Promise<TranscriptReveal> {
      const response = await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/commands/${encodeURIComponent(commandId)}`,
        { signal: signal() },
      )
      const call = await readResponse(response, commandCallSchema)
      return { node: call.node === conversation.id ? null : call.node, blockId: call.blockId }
    },
  }
}

export type HistoryReads = ReturnType<typeof createConversationHistoryReads>
