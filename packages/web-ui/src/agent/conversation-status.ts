import type { ConversationState } from './types'

export type ConversationStatus = 'idle' | 'active' | 'done' | 'error' | 'aborted'

export function conversationStatus(state: ConversationState): ConversationStatus {
  if (state.phase === 'running' || state.phase === 'compacting') return 'active'
  if (state.lastError) return 'error'
  const last = state.blocks[state.blocks.length - 1]
  if (last?.type === 'abort') return 'aborted'
  if (!state.isResultSeen && state.blocks.length > 0) return 'done'
  return 'idle'
}
