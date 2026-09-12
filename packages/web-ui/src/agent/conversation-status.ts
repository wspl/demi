import type { ConversationState } from './types'
import { isSubagentRunning, type SubagentRecord } from './subagents'

export type ConversationStatus = 'idle' | 'active' | 'done' | 'error' | 'aborted'

/** A conversation remains active until its root and all children finish. */
export function isConversationActive(
  phase: ConversationState['phase'],
  subagents: readonly Pick<SubagentRecord, 'phase'>[] = [],
): boolean {
  return phase !== 'idle' || subagents.some((agent) => isSubagentRunning(agent.phase))
}

export function conversationStatus(
  state: ConversationState & { subagents?: readonly Pick<SubagentRecord, 'phase'>[] },
): ConversationStatus {
  if (isConversationActive(state.phase, state.subagents))
    return 'active'
  if (state.lastError)
    return 'error'
  const last = state.blocks[state.blocks.length - 1]
  if (last?.type === 'abort')
    return 'aborted'
  if (!state.isResultSeen && state.blocks.length > 0)
    return 'done'
  return 'idle'
}
