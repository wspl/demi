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
