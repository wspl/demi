import type { StatusDotTone } from '../ui/StatusDot.vue'
import type { ConversationState } from './types'
import { isSubagentRunning, type SubagentRecord } from './subagents'

export type ConversationStatus = 'idle' | 'active' | 'done' | 'error' | 'aborted'

/**
 * The tone of a conversation's status dot wherever it shows, a sidebar
 * row's, a tab's or an agent's mark, so one state has one look everywhere
 * (`conversations.md` § Conversations and projects): running in the accent, which
 * breathes, finished green, failed red. Green and red say there is something new
 * to see, so where the place knows whether it was read, a sidebar row, a read
 * answer or failure settles; an agent's tab and link have no read state, pass no
 * `outcome`, and stay green or red. A Stop is the user's own decision, neither
 * finished nor failed, and like a conversation that has run nothing yet it
 * has no tone: the place's settled look.
 */
export function statusTone(status: ConversationStatus, outcome?: { read: boolean }): StatusDotTone | null {
  switch (status) {
    case 'active':
      return 'accent'
    case 'done':
      return outcome?.read ? null : 'success'
    case 'error':
      return outcome?.read ? null : 'danger'
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
