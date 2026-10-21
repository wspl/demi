import type { PendingCall } from '@demicodes/protocol'
import type { ConversationState } from './types'

/**
 * What the agent whose transcript a list shows does now, as the page holds
 * it: the root's, a subagent's or a side question's, each taken from its
 * entry. Every view of the agent's transcript takes it whole, so a fact of
 * the agent reaches each of them by one path.
 */
export interface AgentActivity {
  /** The calls its model is writing, shown after the transcript. */
  pendingCalls: readonly PendingCall[]
  /**
   * When it began to wait for the model's answer, as `Date.now()`, as the
   * server tells it; null when it waits for none.
   */
  waitingSince: number | null
  /**
   * The page's own action other than a message, a recovery or a side
   * question's question, follow-up or Retry, is on its way: the server has
   * not answered it yet.
   */
  actionSent: boolean
}

/** The root's activity, as its conversation holds it. */
export function rootActivity(
  state: Pick<ConversationState, 'pendingCalls' | 'waitingSince' | 'pendingAction'>,
): AgentActivity {
  return {
    pendingCalls: state.pendingCalls,
    waitingSince: state.waitingSince,
    actionSent: state.pendingAction !== null,
  }
}

/** What an agent does while it does nothing, as a share's page holds it. */
export const AT_REST: AgentActivity = { pendingCalls: [], waitingSince: null, actionSent: false }
