import type { AgentServer } from '@demicodes/agent'
import type { ConversationRecord } from '../storage/control'
import type { ConversationStores } from '../storage/conversation-store'

/**
 * Live activity wins; a cold unfinished checkpoint is interrupted, never
 * reported as still running.
 */
export function conversationSummary(
  conversation: ConversationRecord,
  stores: ConversationStores,
  server: AgentServer
) {
  const summary = stores.summary(conversation.id)
  const live = server.sessionPhase(conversation.id)
  let status:
    'running' | 'compacting' | 'interrupted' | 'error' |
    'stopped' | 'completed' | 'idle' = 'idle'
  if (server.treeActive(conversation.id))
    status = live === 'compacting' ? 'compacting' : 'running'
  else if (!live && summary.phase !== 'idle')
    status = 'interrupted'
  else if (summary.last === 'error')
    status = 'error'
  else if (summary.last === 'abort')
    status = 'stopped'
  else if (summary.last === 'response')
    status = 'completed'
  return {
    ...conversation,
    status,
    revision: summary.revision,
    unread: summary.revision > conversation.readRevision
  }
}
