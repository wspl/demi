import type { ConversationState } from './types'
import { latestBlocks } from './history'

/**
 * Admission is visible in the transcript or the server queue for a message,
 * and in the transcript or the pending steers for a steer, which a message
 * sent as a steer keeps its id for.
 */
export function hasAcceptedSubmission(
  state: Pick<ConversationState, 'history' | 'queue' | 'pendingSteers'>,
  id: string,
): boolean {
  return isWritten(state, id) ||
    state.queue.some((message) => message.id === id) ||
    state.pendingSteers.some((steer) => steer.id === id)
}

/** Whether the transcript holds the message `id`: as the message of its turn, or as a steer. */
export function isWritten(state: Pick<ConversationState, 'history'>, id: string): boolean {
  return latestBlocks(state.history).some((block) =>
    (block.type === 'user' && block.turnId === id) || (block.type === 'steer' && block.id === id))
}
