import type { PendingCall } from '@demicodes/protocol'
import type { ConversationState } from './types'
import type { WaitClock } from './wait-clock'

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
  /** The action whose start began that wait, as the server tells it; null for none. */
  waitAction: string | null
  /** The presses of the page's own actions; absent where the page acts on none, as for a subagent. */
  waitClock?: WaitClock
  /**
   * The page's own action other than a message that is on its way, a
   * recovery or a side question's question, follow-up or Retry: its id,
   * until the server shows it has it; null for none.
   */
  sentAction: string | null
}

/** The root's activity, as its conversation holds it. */
export function rootActivity(
  state: Pick<ConversationState, 'pendingCalls' | 'waitingSince' | 'waitAction' | 'waitClock' | 'pendingAction'>,
): AgentActivity {
  return {
    pendingCalls: state.pendingCalls,
    waitingSince: state.waitingSince,
    waitAction: state.waitAction,
    waitClock: state.waitClock,
    sentAction: state.pendingAction?.id ?? null,
  }
}

/** What an agent does while it does nothing, as a share's page holds it. */
export const AT_REST: AgentActivity = { pendingCalls: [], waitingSince: null, waitAction: null, sentAction: null }
