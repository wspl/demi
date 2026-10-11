import type { StatusDotTone } from '../ui/StatusDot.vue'
import type { ConversationState } from './types'
import { isSubagentRunning, type SubagentRecord } from './subagents'

export type ConversationStatus = 'idle' | 'active' | 'done' | 'error' | 'aborted'

/**
 * The tone of a conversation's status dot wherever it shows, a sidebar
 * row's, a tab's or an agent's mark, so one state has one look everywhere
 * (`product.md` § Conversations and projects): running in the accent, which
 * breathes, finished green, failed red. A Stop is the user's own decision,
 * neither finished nor failed, and like a conversation that has run nothing
 * yet it has no tone: the place's settled look.
 */
export function statusTone(status: ConversationStatus): StatusDotTone | null {
  switch (status) {
    case 'active':
      return 'accent'
    case 'done':
      return 'success'
    case 'error':
      return 'danger'
    case 'aborted':
    case 'idle':
      return null
  }
}

/** A conversation remains active until its root and all children finish. */
export function isConversationActive(
  phase: ConversationState['phase'],
  subagents: readonly Pick<SubagentRecord, 'phase'>[] = [],
): boolean {
  return phase !== 'idle' || subagents.some((agent) => isSubagentRunning(agent.phase))
}
