import { FIRST_BLOCKS } from '@demicodes/protocol'
import type { TranscriptReveal } from '@demicodes/web-ui/agent/history'
import type { TranscriptRead } from '@demicodes/web-ui/transport/protocol'
import { apiRequest, readResponse, readSizedResponse } from '../api/client'
import { commandCallSchema, transcriptPageSchema } from '../api/generated/web-api'
import type { Conversation } from '../state/types'
import { readNodeBlocks } from '../state/offline'
import { createHistoryReads } from './history-reads'

/**
 * The reads of conversations' history, through the routes under `base`, a
 * conversation's own (`/conversations/:id`) or a view link's: a
 * conversation's root is live while its stream is open, as `live` says, and a
 * subagent while it runs. A page before a window comes from the disk copy of
 * `userId`'s when it holds it.
 */
export function createConversationHistoryReads(
  signal: () => AbortSignal,
  live: (conversation: Conversation) => boolean,
  userId: () => string | null,
  base: (conversation: Conversation) => string = (conversation) => `/conversations/${encodeURIComponent(conversation.id)}`,
) {
  const reads = createHistoryReads<Conversation>(
    signal,
    (conversation, node) => node === null
      ? live(conversation)
      : conversation.subagents.find((agent) => agent.id === node)?.phase === 'running'
        || conversation.sides.find((side) => side.id === node)?.sessionPhase === 'running',
    (conversation, node) => `${base(conversation)}/agents/${encodeURIComponent(node ?? conversation.id)}`,
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
        `${base(conversation)}/commands/${encodeURIComponent(commandId)}`,
        { signal: signal() },
      )
      const call = await readResponse(response, commandCallSchema)
      return { node: call.node === conversation.id ? null : call.node, blockId: call.blockId }
    },
  }
}

export type HistoryReads = ReturnType<typeof createConversationHistoryReads>

/**
 * The latest page of `node`'s transcript, through the routes under `base`,
 * with its version; with `first`, its last `FIRST_BLOCKS` alone. The answer
 * and its size come with it, for the caller's log.
 */
export async function readLatestTranscript(
  base: string,
  node: string,
  signal: AbortSignal,
  first: boolean,
): Promise<{ page: TranscriptRead; response: Response; bytes: number }> {
  const query = first ? `latest=true&blocks=${FIRST_BLOCKS}` : 'latest=true'
  const response = await apiRequest(`${base}/agents/${encodeURIComponent(node)}/transcript?${query}`, { signal })
  const { value: page, bytes } = await readSizedResponse(response, transcriptPageSchema)
  if (!page.version) {
    throw new Error('The latest page names no version of its transcript')
  }
  return { page: { ...page, version: page.version }, response, bytes }
}
